//! `girsync sync` — mirror src onto dst.
//!
//! Phase order matters and is deliberate:
//! 1. validate + back up both caches and snapshot dst's pre-sync state,
//! 2. fix dst casing to match src (insensitive mode),
//! 3. plan from the diff,
//! 4. apply: mkdirs, pre-drop cache entries, deletes, parallel copies, rmdirs,
//! 5. prune + flush.
//!
//! Cache entries are always dropped *before* the corresponding filesystem
//! change, so a crash mid-run re-copies rather than trusting a half-written
//! file. There is no resume; rerun and verify with `compare`.

use anyhow::{bail, Context, Result};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use tracing::{debug, error, info, trace};

use crate::cache::{backup_db, open_db, snapshot_old, FileRec, CACHE_PREFIX, META_KEY};
use crate::config::{LogCtx, ScanMode, SyncOpts};
use crate::diff::diff_maps;
use crate::effective::build_effective_folder;
use crate::filter::is_excluded;
use crate::hash::hash_file;
use crate::scan::walk_live;
use crate::util::{elapsed_s, is_record_path, mtime_ns_of};

/// Mirror `src` onto `dst`. Returns the process exit code (always 0; errors
/// propagate as `Err`).
pub fn cmd_sync(opts: SyncOpts, log: &LogCtx) -> Result<i32> {
    let t0 = std::time::Instant::now();
    let SyncOpts {
        src,
        dst,
        fast,
        missing_only,
        keep_extra,
        dry_run,
        jobs,
        common,
    } = opts;
    let span = tracing::info_span!(
        "girsync.sync",
        pid = std::process::id(),
        src = %src.display(),
        dst = %dst.display(),
        algos = ?common.algos,
        fast,
        missing_only,
        keep_extra,
        dry_run,
        jobs,
        include = ?common.includes,
        exclude = ?common.excludes,
        case_sensitive = common.case_sensitive,
        max_depth = common.max_depth,
        ignore_cache = common.ignore_cache,
        console_level = %log.level.to_ascii_lowercase(),
        file_level = "trace",
        log_file = %log.file_display(),
    );
    let _span_guard = span.enter();
    info!("start");
    if is_record_path(&src) || is_record_path(&dst) {
        bail!("sync needs folder vs folder (record inputs are compare-only)");
    }
    if !src.is_dir() || !dst.is_dir() {
        bail!("src and dst must both be directories");
    }
    let canon_src = src
        .canonicalize()
        .with_context(|| format!("canon {}", src.display()))?;
    let canon_dst = dst
        .canonicalize()
        .with_context(|| format!("canon {}", dst.display()))?;
    if canon_src == canon_dst {
        bail!("src == dst");
    }
    if jobs == 0 {
        bail!("--jobs must be >= 1");
    }

    let src_db_path = src.join(CACHE_PREFIX);
    let dst_db_path = dst.join(CACHE_PREFIX);

    if !dry_run {
        // backups before modifying either DB
        if src_db_path.exists() {
            backup_db(&src_db_path)?;
        }
        if dst_db_path.exists() {
            backup_db(&dst_db_path)?;
            // old-state snapshot before any dst changes
            snapshot_old(&dst_db_path)?;
        }
        if common.ignore_cache {
            for p in [&src_db_path, &dst_db_path] {
                if p.exists() {
                    std::fs::remove_dir_all(p)
                        .with_context(|| format!("remove {}", p.display()))?;
                    info!(path = %p.display(), "ignore-cache removed");
                }
            }
        }
    } else {
        info!("dry-run: skipping backups and cache writes");
    }

    let src_db = if dry_run {
        // open without backup; avoid creating walls of writes later (writes skipped by flag)
        open_db(&src_db_path, common.case_sensitive, false, false).or_else(|_| {
            // dry-run on missing cache: use temp DB so we never create real one
            sled::Config::new()
                .temporary(true)
                .open()
                .context("open temp db")
        })?
    } else {
        open_db(&src_db_path, common.case_sensitive, false, false)?
    };
    let dst_db = if dry_run {
        open_db(&dst_db_path, common.case_sensitive, false, false).or_else(|_| {
            sled::Config::new()
                .temporary(true)
                .open()
                .context("open temp db")
        })?
    } else {
        open_db(&dst_db_path, common.case_sensitive, false, false)?
    };

    let mode = ScanMode {
        fast,
        force_hash: false,
        dry_run,
    };
    info!("loading src effective map");
    let sm = build_effective_folder(&src, &src_db, &common, mode)?;
    info!("loading dst effective map");
    let mut dm = build_effective_folder(&dst, &dst_db, &common, mode)?;
    info!(src_entries = sm.len(), dst_entries = dm.len(), "maps ready");

    // case-insensitive rename pass (sync mode): rename dst to src casing first.
    let mut renamed = 0usize;
    if !common.case_sensitive {
        // map lower -> (src_rel, dst_rel)
        let mut slow: HashMap<String, &String> = HashMap::new();
        for k in sm.keys() {
            slow.insert(k.to_lowercase(), k);
        }
        let mut renames: Vec<(PathBuf, PathBuf, String, String)> = Vec::new();
        for (lk, srel) in &slow {
            // find dst key with same lower but different case
            if let Some(drel) = dm.keys().find(|k| k.to_lowercase() == *lk && *k != *srel) {
                renames.push((
                    dst.join(drel),
                    dst.join(*srel),
                    (*drel).clone(),
                    (*srel).clone(),
                ));
            }
        }
        info!(pending = renames.len(), "rename pass");
        for (from, to, drel, srel) in renames {
            println!("RENAME {} -> {}", drel, srel);
            info!(from = %drel, to = %srel, "rename");
            if !dry_run {
                if let Some(parent) = to.parent() {
                    std::fs::create_dir_all(parent)
                        .with_context(|| format!("mkdir {}", parent.display()))?;
                }
                // two-step for Windows case-only rename
                let tmp = from.with_extension("girsync_case_tmp");
                std::fs::rename(&from, &tmp)
                    .with_context(|| format!("rename {} (case fix step1)", from.display()))?;
                std::fs::rename(&tmp, &to)
                    .with_context(|| format!("rename to {} (case fix step2)", to.display()))?;
                // move DB entry + in-memory entry
                if let Some(v) = dm.remove(&drel) {
                    dm.insert(srel.clone(), v);
                }
                if let Ok(Some(raw)) = dst_db.get(drel.as_bytes()) {
                    dst_db.remove(drel.as_bytes())?;
                    dst_db.insert(srel.as_bytes(), raw)?;
                }
                renamed += 1;
            } else {
                renamed += 1;
            }
        }
    }

    let diff = diff_maps(
        &sm,
        &dm,
        &common.algos,
        true, /* post-rename: exact keys */
    );
    // NOTE: after rename pass, keys align; recompute case-sensitive diff.

    let mut to_copy: Vec<String> = Vec::new();
    for r in &diff.missing {
        if sm.get(r).map(|e| e.kind == "file").unwrap_or(false) {
            to_copy.push(r.clone());
        }
    }
    if !missing_only {
        for r in &diff.changed {
            if sm.get(r).map(|e| e.kind == "file").unwrap_or(false) {
                to_copy.push(r.clone());
            }
        }
        // type-conflicts where src is file: remove dst then copy; where src is dir: remove file then mkdir
        for r in &diff.type_conflict {
            if sm.get(r).map(|e| e.kind == "file").unwrap_or(false) {
                to_copy.push(r.clone());
            }
        }
    }
    to_copy.sort();
    to_copy.dedup();

    let mut to_delete_files: Vec<String> = Vec::new();
    let mut to_fix_dirs: Vec<String> = Vec::new(); // dst type-conflict where src is dir
    if !keep_extra {
        for r in &diff.extra {
            if dm.get(r).map(|e| e.kind == "file").unwrap_or(false) {
                to_delete_files.push(r.clone());
            }
            // extra dirs are removed by the unknown-dirs pass below
        }
    }
    for r in &diff.type_conflict {
        if sm.get(r).map(|e| e.kind == "dir").unwrap_or(false) {
            to_fix_dirs.push(r.clone());
        }
    }

    // src dirs to ensure
    let mut to_mkdir: Vec<String> = Vec::new();
    for (rel, rec) in &sm {
        if rec.kind == "dir" && !dm.contains_key(rel) {
            to_mkdir.push(rel.clone());
        }
    }
    to_mkdir.sort();
    info!(
        renamed,
        mkdir = to_mkdir.len(),
        copy = to_copy.len(),
        delete_files = to_delete_files.len(),
        fix_dirs = to_fix_dirs.len(),
        dry_run,
        "plan"
    );
    debug!(copy = ?to_copy, "plan copy list");
    debug!(mkdir = ?to_mkdir, "plan mkdir list");

    if dry_run {
        for r in &to_mkdir {
            println!("MKDIR {}", r);
        }
        for r in &to_copy {
            println!("COPY {}", r);
        }
        for r in &to_delete_files {
            println!("DELETE {}", r);
        }
        for r in &to_fix_dirs {
            println!("RMDIR-FILE {}", r);
        }
        println!(
            "SUMMARY renamed={} mkdir={} copy={} delete={} missing_only={} keep_extra={} dry_run=true",
            renamed,
            to_mkdir.len(),
            to_copy.len(),
            to_delete_files.len() + to_fix_dirs.len(),
            missing_only,
            keep_extra
        );
        info!(
            renamed,
            mkdir = to_mkdir.len(),
            copy = to_copy.len(),
            delete = to_delete_files.len() + to_fix_dirs.len(),
            dry_run = true,
            elapsed_s = elapsed_s(t0),
            exit = 0,
            "end"
        );
        return Ok(0);
    }

    // Apply: mkdirs
    info!(dirs = to_mkdir.len(), "apply mkdirs");
    for (i, r) in to_mkdir.iter().enumerate() {
        std::fs::create_dir_all(dst.join(r))
            .with_context(|| format!("mkdir {}", dst.join(r).display()))?;
        dst_db.insert(
            r.as_bytes(),
            serde_json::to_vec(&FileRec {
                kind: "dir".into(),
                size: 0,
                mtime_ns: 0,
                hashes: HashMap::new(),
            })?,
        )?;
        debug!(rel = %r, "mkdir");
        if (i + 1) % 100 == 0 {
            info!(done = i + 1, total = to_mkdir.len(), "mkdir progress");
        }
    }
    // Fix type-conflicts where src is dir: remove dst file, mkdir
    info!(count = to_fix_dirs.len(), "apply fix-dirs");
    for r in &to_fix_dirs {
        dst_db.remove(r.as_bytes())?; // delete entries before change
        let p = dst.join(r);
        if p.is_file() || p.is_symlink() {
            std::fs::remove_file(&p).with_context(|| format!("remove {}", p.display()))?;
        } else if p.is_dir() {
            std::fs::remove_dir_all(&p).with_context(|| format!("rmdir {}", p.display()))?;
        }
        std::fs::create_dir_all(&p).with_context(|| format!("mkdir {}", p.display()))?;
        println!("FIX-DIR {}", r);
        info!(rel = %r, "fix-dir");
    }

    // Delete entries before file changes (crash leaves truncated risk documented;
    // entries already dropped so resume re-copies).
    for r in to_copy.iter().chain(to_delete_files.iter()) {
        dst_db.remove(r.as_bytes())?;
    }
    dst_db.flush()?;

    // Delete extra files (+ prune unknown dirs afterwards)
    let mut deleted = 0usize;
    info!(files = to_delete_files.len(), "apply deletes");
    for (i, r) in to_delete_files.iter().enumerate() {
        let p = dst.join(r);
        // eff map only tracks files here for extra; dirs handled below
        if let Some(rec) = dm.get(r) {
            if rec.kind == "dir" {
                continue;
            }
        }
        if p.is_file() || p.is_symlink() {
            std::fs::remove_file(&p).with_context(|| format!("delete {}", p.display()))?;
            println!("DELETE {}", r);
            debug!(rel = %r, "delete");
            deleted += 1;
        } else if p.is_dir() {
            std::fs::remove_dir_all(&p).with_context(|| format!("rmdir {}", p.display()))?;
            println!("RMDIR {}", r);
            debug!(rel = %r, "rmdir");
            deleted += 1;
        } else if p.exists() {
            bail!("unsupported type {}", p.display());
        }
        if (i + 1) % 100 == 0 {
            info!(
                done = i + 1,
                total = to_delete_files.len(),
                deleted,
                "delete progress"
            );
        }
    }
    info!(deleted, "deletes done");

    // Copy files in parallel (truncate + write in place, preserve mtime, verify).
    info!(files = to_copy.len(), jobs, "copy start");
    let copy_total = to_copy.len();
    let copy_done = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let copy_done_cb = copy_done.clone();
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(jobs)
        .build()
        .context("build thread pool")?;
    let results = pool.install(|| {
        use rayon::prelude::*;
        to_copy
            .par_iter()
            .map(|rel| {
                debug!(rel = %rel, "copy start");
                let r = copy_one(&src, &dst, rel, &common.algos);
                let n = copy_done_cb.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                if n % 25 == 0 || n == copy_total {
                    info!(done = n, total = copy_total, "copy progress");
                } else {
                    trace!(done = n, total = copy_total, rel = %rel, "copy progress");
                }
                r
            })
            .collect::<Vec<_>>()
    });
    let mut copied = 0usize;
    let mut new_recs: Vec<(String, FileRec)> = Vec::new();
    for (rel, r) in to_copy.iter().zip(results) {
        let rec = r.with_context(|| format!("copy {}", rel))?;
        println!("COPY {}", rel);
        debug!(rel = %rel, "copy done");
        new_recs.push((rel.clone(), rec));
        copied += 1;
    }
    info!(copied, total = copy_total, "copy done");
    for (rel, rec) in new_recs {
        dst_db.insert(rel.as_bytes(), serde_json::to_vec(&rec)?)?;
    }

    // Remove dst dirs not in src (unknown empties + leftovers), deepest first.
    let mut removed_dirs = 0usize;
    info!("scan dst for unknown dirs");
    let live_after = walk_live(&dst, common.max_depth)?;
    let mut unknown_dirs: Vec<String> = Vec::new();
    for e in &live_after {
        if !e.is_dir {
            continue;
        }
        if is_excluded(
            &e.rel,
            &common.includes,
            &common.excludes,
            common.case_sensitive,
        ) {
            continue;
        }
        if !sm.contains_key(&e.rel) {
            unknown_dirs.push(e.rel.clone());
        }
    }
    unknown_dirs.sort_by_key(|s| std::cmp::Reverse(s.len()));
    info!(unknown = unknown_dirs.len(), "rmdir pass");
    for (i, r) in unknown_dirs.iter().enumerate() {
        let p = dst.join(r);
        if p.is_dir() {
            // dir may already be gone as child of removed parent
            if let Err(e) = std::fs::remove_dir_all(&p) {
                if p.exists() {
                    bail!("rmdir {}: {:#}", p.display(), e);
                }
            }
            dst_db.remove(r.as_bytes())?;
            println!("RMDIR {}", r);
            debug!(rel = %r, "rmdir");
            removed_dirs += 1;
        }
        if (i + 1) % 100 == 0 {
            info!(
                done = i + 1,
                total = unknown_dirs.len(),
                removed = removed_dirs,
                "rmdir progress"
            );
        }
    }
    // final prune of anything else missing + flush
    {
        let live2 = walk_live(&dst, common.max_depth)?;
        let mut set: HashSet<String> = HashSet::new();
        for e in live2 {
            if !is_excluded(
                &e.rel,
                &common.includes,
                &common.excludes,
                common.case_sensitive,
            ) {
                set.insert(e.rel);
            }
        }
        for kv in dst_db.iter() {
            let (k, _) = kv?;
            if k.as_ref() == META_KEY.as_bytes() {
                continue;
            }
            let rel = String::from_utf8(k.to_vec()).context("non-UTF8 key")?;
            if !set.contains(&rel) {
                dst_db.remove(rel.as_bytes())?;
            }
        }
    }
    src_db.flush()?;
    dst_db.flush()?;

    println!(
        "SUMMARY renamed={} mkdir={} copied={} deleted={} rmdir={} missing_only={} keep_extra={}",
        renamed,
        to_mkdir.len(),
        copied,
        deleted,
        removed_dirs,
        missing_only,
        keep_extra
    );
    info!(
        renamed,
        mkdir = to_mkdir.len(),
        copied,
        deleted,
        rmdir = removed_dirs,
        missing_only,
        keep_extra,
        dry_run = false,
        elapsed_s = elapsed_s(t0),
        exit = 0,
        "end"
    );
    Ok(0)
}

/// Copy one file src -> dst, preserving mtime, then verify before returning the
/// record to cache. `File::create` truncates in place, so a killed run can
/// leave a truncated file; the caller drops the cache entry first so a rerun
/// re-copies rather than trusting it.
#[tracing::instrument(skip_all, fields(rel, bytes = tracing::field::Empty))]
fn copy_one(src_root: &Path, dst_root: &Path, rel: &str, algos: &[String]) -> Result<FileRec> {
    trace!("copy_one start");
    let s = src_root.join(rel);
    let d = dst_root.join(rel);
    // dst dir that is in the way of a file copy: remove first
    if d.is_dir() && !d.is_symlink() {
        std::fs::remove_dir_all(&d).with_context(|| format!("rmdir {}", d.display()))?;
    }
    if let Some(parent) = d.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("mkdir {}", parent.display()))?;
    }
    let smeta = std::fs::metadata(&s).with_context(|| format!("stat {}", s.display()))?;
    let ssize = smeta.len();
    let smtime_sys = smeta.modified().ok();
    let smtime_ns = mtime_ns_of(&smeta);
    // stream copy (truncate + write in place)
    let mut fin = std::fs::File::open(&s).with_context(|| format!("open {}", s.display()))?;
    let fout = std::fs::File::create(&d).with_context(|| format!("create {}", d.display()))?;
    // File::create truncates existing. Write via &File (Write for &File).
    let mut fout = fout;
    std::io::copy(&mut fin, &mut fout)
        .with_context(|| format!("copy {} -> {}", s.display(), d.display()))?;
    fout.sync_all().ok();
    drop(fout);
    // preserve mtime
    if let Some(st) = smtime_sys {
        let f = std::fs::OpenOptions::new().write(true).open(&d)?;
        f.set_modified(st)
            .with_context(|| format!("set mtime {}", d.display()))?;
    }
    // verify: size + mtime + hash
    let dmeta = std::fs::metadata(&d).with_context(|| format!("stat {}", d.display()))?;
    if dmeta.len() != ssize {
        bail!(
            "verify size {} (want {} got {})",
            d.display(),
            ssize,
            dmeta.len()
        );
    }
    if algos.is_empty() {
        // size+mtime only
        let dmtime = mtime_ns_of(&dmeta);
        if dmtime != smtime_ns {
            bail!("verify mtime {}", d.display());
        }
        return Ok(FileRec {
            kind: "file".into(),
            size: ssize,
            mtime_ns: smtime_ns,
            hashes: HashMap::new(),
        });
    }
    let sh = hash_file(&s, algos).with_context(|| format!("hash {}", s.display()))?;
    let dh = hash_file(&d, algos).with_context(|| format!("hash {}", d.display()))?;
    for a in algos {
        if sh.get(a) != dh.get(a) {
            error!(algo = %a, path = %d.display(), "verify hash mismatch");
            bail!("verify hash({}) {}", a, d.display());
        }
    }
    tracing::Span::current().record("bytes", ssize);
    trace!(bytes = ssize, "copy_one done");
    Ok(FileRec {
        kind: "file".into(),
        size: ssize,
        mtime_ns: smtime_ns,
        hashes: dh,
    })
}
