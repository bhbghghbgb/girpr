//! Executing a sync plan against the filesystem.
//!
//! All shared state is hung off [`Applier`] so each phase reads as a short list
//! of steps. The phase *order* is a correctness invariant, so it lives in one
//! place ([`Applier::apply`]) rather than being spread across the coordinator:
//!
//! 1. mkdir + fix-dirs (create the shapes src expects),
//! 2. pre-drop the cache entries for every path about to change,
//! 3. delete extras, copy src files, remove unknown dirs,
//! 4. prune leftover cache rows, then flush.
//!
//! Steps 1 and 2 must precede any content change: cache entries are always
//! dropped *before* the corresponding filesystem change, so a crash mid-run
//! re-copies rather than trusting a half-written file. There is no resume.

use anyhow::{bail, Context, Result};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use tracing::{debug, error, info, trace};

use super::plan::Plan;
use crate::cache::{FileRec, META_KEY};
use crate::config::CommonOpts;
use crate::effective::EffRec;
use crate::filter::is_excluded;
use crate::hash::hash_file;
use crate::scan::walk_live;
use crate::util::mtime_ns_of;

/// Everything the apply phases share: both roots, the dst cache, the two
/// resolved side maps, and the resolved config.
pub(super) struct Applier<'a> {
    pub src: &'a Path,
    pub dst: &'a Path,
    pub dst_db: &'a sled::Db,
    /// src effective map — decides which dst dirs are "unknown".
    pub sm: &'a HashMap<String, EffRec>,
    /// dst effective map, after the rename pass.
    pub dm: &'a HashMap<String, EffRec>,
    pub common: &'a CommonOpts,
    pub jobs: usize,
}

/// What a run actually did, for the `SUMMARY` line.
///
/// Field order matches the phase order in [`Applier::apply`].
#[derive(Debug, Default)]
pub(super) struct Applied {
    pub deleted: usize,
    pub copied: usize,
    pub removed_dirs: usize,
}

impl Applier<'_> {
    /// Run every apply phase in the order documented on this module.
    pub(super) fn apply(&self, plan: &Plan) -> Result<Applied> {
        self.mkdirs(plan)?;
        self.fix_dirs(plan)?;
        self.predrop_cache_entries(plan)?;
        // Field order is the phase order: deletes, then copies, then rmdirs.
        let applied = Applied {
            deleted: self.delete_extras(plan)?,
            copied: self.copy_files(plan)?,
            removed_dirs: self.remove_unknown_dirs()?,
        };
        self.prune_dst_cache()?;
        Ok(applied)
    }

    /// Create every src directory that dst lacks.
    fn mkdirs(&self, plan: &Plan) -> Result<()> {
        info!(dirs = plan.mkdir.len(), "apply mkdirs");
        for (i, r) in plan.mkdir.iter().enumerate() {
            std::fs::create_dir_all(self.dst.join(r))
                .with_context(|| format!("mkdir {}", self.dst.join(r).display()))?;
            self.dst_db.insert(
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
                info!(done = i + 1, total = plan.mkdir.len(), "mkdir progress");
            }
        }
        Ok(())
    }

    /// Resolve type-conflicts toward src: dst has a file where src has a dir.
    fn fix_dirs(&self, plan: &Plan) -> Result<()> {
        info!(count = plan.fix_dirs.len(), "apply fix-dirs");
        for r in &plan.fix_dirs {
            self.dst_db.remove(r.as_bytes())?; // delete entries before change
            let p = self.dst.join(r);
            if p.is_file() || p.is_symlink() {
                std::fs::remove_file(&p).with_context(|| format!("remove {}", p.display()))?;
            } else if p.is_dir() {
                std::fs::remove_dir_all(&p).with_context(|| format!("rmdir {}", p.display()))?;
            }
            std::fs::create_dir_all(&p).with_context(|| format!("mkdir {}", p.display()))?;
            println!("FIX-DIR {}", r);
            info!(rel = %r, "fix-dir");
        }
        Ok(())
    }

    /// Drop the dst cache rows for every path this run will change.
    fn predrop_cache_entries(&self, plan: &Plan) -> Result<()> {
        for r in plan.copy.iter().chain(plan.delete_files.iter()) {
            self.dst_db.remove(r.as_bytes())?;
        }
        self.dst_db.flush()?;
        Ok(())
    }

    /// Delete dst-only files, returning how many paths were removed.
    fn delete_extras(&self, plan: &Plan) -> Result<usize> {
        let mut deleted = 0usize;
        info!(files = plan.delete_files.len(), "apply deletes");
        for (i, r) in plan.delete_files.iter().enumerate() {
            let p = self.dst.join(r);
            // the plan only holds files here; dirs are handled by the
            // unknown-dirs pass
            if let Some(rec) = self.dm.get(r)
                && rec.kind == "dir"
            {
                continue;
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
                    total = plan.delete_files.len(),
                    deleted,
                    "delete progress"
                );
            }
        }
        info!(deleted, "deletes done");
        Ok(deleted)
    }

    /// Copy every planned src file across, in parallel, returning the count.
    fn copy_files(&self, plan: &Plan) -> Result<usize> {
        info!(files = plan.copy.len(), jobs = self.jobs, "copy start");
        let copy_total = plan.copy.len();
        let copy_done = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let copy_done_cb = copy_done.clone();
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(self.jobs)
            .build()
            .context("build thread pool")?;
        let results = pool.install(|| {
            use rayon::prelude::*;
            plan.copy
                .par_iter()
                .map(|rel| {
                    debug!(rel = %rel, "copy start");
                    let r = copy_one(self.src, self.dst, rel, &self.common.algos);
                    let n = copy_done_cb.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                    if n.is_multiple_of(25) || n == copy_total {
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
        for (rel, r) in plan.copy.iter().zip(results) {
            let rec = r.with_context(|| format!("copy {}", rel))?;
            println!("COPY {}", rel);
            debug!(rel = %rel, "copy done");
            new_recs.push((rel.clone(), rec));
            copied += 1;
        }
        info!(copied, total = copy_total, "copy done");
        // Only record files that actually verified, so a partial failure never
        // leaves a cache claiming content the dst does not have.
        for (rel, rec) in new_recs {
            self.dst_db
                .insert(rel.as_bytes(), serde_json::to_vec(&rec)?)?;
        }
        Ok(copied)
    }

    /// Remove dst directories that src does not have, deepest first.
    ///
    /// Runs after the copies so directories emptied by them are collected too.
    fn remove_unknown_dirs(&self) -> Result<usize> {
        let mut removed_dirs = 0usize;
        info!("scan dst for unknown dirs");
        let live_after = walk_live(self.dst, self.common.max_depth)?;
        let mut unknown_dirs: Vec<String> = Vec::new();
        for e in &live_after {
            if !e.is_dir {
                continue;
            }
            if is_excluded(
                &e.rel,
                &self.common.includes,
                &self.common.excludes,
                self.common.case_sensitive,
            ) {
                continue;
            }
            if !self.sm.contains_key(&e.rel) {
                unknown_dirs.push(e.rel.clone());
            }
        }
        // deepest first: a child may already be gone as part of its parent
        unknown_dirs.sort_by_key(|s| std::cmp::Reverse(s.len()));
        info!(unknown = unknown_dirs.len(), "rmdir pass");
        for (i, r) in unknown_dirs.iter().enumerate() {
            let p = self.dst.join(r);
            if p.is_dir() {
                if let Err(e) = std::fs::remove_dir_all(&p)
                    && p.exists()
                {
                    bail!("rmdir {}: {:#}", p.display(), e);
                }
                self.dst_db.remove(r.as_bytes())?;
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
        Ok(removed_dirs)
    }

    /// Drop dst cache rows for paths that no longer exist on disk.
    fn prune_dst_cache(&self) -> Result<()> {
        let live = walk_live(self.dst, self.common.max_depth)?;
        let mut present: HashSet<String> = HashSet::new();
        for e in live {
            if !is_excluded(
                &e.rel,
                &self.common.includes,
                &self.common.excludes,
                self.common.case_sensitive,
            ) {
                present.insert(e.rel);
            }
        }
        for kv in self.dst_db.iter() {
            let (k, _) = kv?;
            if k.as_ref() == META_KEY.as_bytes() {
                continue;
            }
            let rel = String::from_utf8(k.to_vec()).context("non-UTF8 key")?;
            if !present.contains(&rel) {
                self.dst_db.remove(rel.as_bytes())?;
            }
        }
        Ok(())
    }
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
