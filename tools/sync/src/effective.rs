//! Building the *effective* map for one side of a comparison.
//!
//! An effective map is the authoritative view of a side once the cache, the
//! glob filters, and the case mode have been applied. Disk always governs:
//! stale cache entries are dropped wholesale and rehashed, and rows for paths
//! that vanished are pruned.

use anyhow::{bail, Context, Result};
use glob::Pattern;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use tracing::{debug, info, trace, warn};

use crate::cache::{load_all_records, open_db, put_rec, FileRec, CACHE_PREFIX};
use crate::config::{CommonOpts, ScanMode};
use crate::filter::is_excluded;
use crate::hash::hash_file;
use crate::scan::{check_mixed_case, walk_live};
use crate::util::{elapsed_s, is_cache_rel, is_record_path};

/// A resolved path: kind, stat data, and whatever hashes were needed.
#[derive(Clone, Debug)]
pub struct EffRec {
    /// "file" or "dir"
    pub kind: String,
    pub size: u64,
    pub mtime_ns: i64,
    pub hashes: HashMap<String, String>,
}

impl EffRec {
    fn dir() -> Self {
        Self {
            kind: "dir".into(),
            size: 0,
            mtime_ns: 0,
            hashes: HashMap::new(),
        }
    }
}

/// Build the effective map for a folder, using the embedded cache to short-circuit.
///
/// Writes back to the cache unless `mode.dry_run`, and prunes rows for paths
/// that are no longer on disk (or are now filtered out).
///
/// In insensitive mode the on-disk name governs: cached keys are indexed by
/// lowercase so a disk/cached casing difference is fixed to the disk name first,
/// reusing the cached hashes when stat matches, instead of erroring. Multiple
/// stale alternates are pruned below, not treated as a conflict.
pub fn build_effective_folder(
    root: &Path,
    db: &sled::Db,
    common: &CommonOpts,
    mode: ScanMode,
) -> Result<HashMap<String, EffRec>> {
    let algos: &[String] = &common.algos;
    let includes: &[Pattern] = &common.includes;
    let excludes: &[Pattern] = &common.excludes;
    let case_sensitive = common.case_sensitive;
    let max_depth = common.max_depth;
    let ScanMode {
        fast,
        force_hash,
        dry_run,
    } = mode;

    let live = walk_live(root, max_depth)?;
    check_mixed_case(&live, &root.display().to_string(), case_sensitive)?;
    let mut eff = HashMap::new();
    let mut live_set: HashSet<String> = HashSet::new();
    for e in &live {
        if is_excluded(&e.rel, includes, excludes, case_sensitive) {
            continue; // treated as not existing at all
        }
        live_set.insert(e.rel.clone());
    }
    info!(
        root = %root.display(),
        live = live.len(),
        fast,
        force_hash,
        dry_run,
        algos = ?algos,
        "effective start"
    );
    let t_eff = std::time::Instant::now();
    let mut n_done: usize = 0;
    let mut n_hashed: usize = 0;
    let mut n_fast_hit: usize = 0;
    let total_live = live.len();
    let alt_recs: HashMap<String, FileRec> = if !case_sensitive {
        load_all_records(db)?
    } else {
        HashMap::new()
    };
    let mut alt_index: HashMap<String, Vec<String>> = HashMap::new();
    if !case_sensitive {
        for key in alt_recs.keys() {
            alt_index
                .entry(key.to_lowercase())
                .or_default()
                .push(key.clone());
        }
    }
    let mut last_prog = std::time::Instant::now();
    for e in live {
        if is_excluded(&e.rel, includes, excludes, case_sensitive) {
            continue;
        }
        if e.is_dir {
            eff.insert(e.rel.clone(), EffRec::dir());
            if !dry_run {
                let cur: Option<FileRec> = db
                    .get(e.rel.as_bytes())?
                    .map(|v| serde_json::from_slice(&v))
                    .transpose()
                    .context("parse cache entry")?;
                if cur.map(|c| c.kind != "dir").unwrap_or(true) {
                    put_rec(
                        db,
                        &e.rel,
                        &FileRec {
                            kind: "dir".into(),
                            size: 0,
                            mtime_ns: 0,
                            hashes: HashMap::new(),
                        },
                    )?;
                }
            }
            continue;
        }
        // file
        // `adopted` = cache entry came from an alternate-cased key, so the
        // disk-cased key must be (re)written even on a fast-hit.
        let (cached, adopted): (Option<FileRec>, bool) = match db
            .get(e.rel.as_bytes())?
            .map(|v| serde_json::from_slice(&v))
            .transpose()
            .context("parse cache entry")?
        {
            Some(c) => (Some(c), false),
            // Exact-case miss in insensitive mode: adopt the single stale
            // alternate-cased entry as the cache candidate (disk name wins).
            None if !case_sensitive => {
                match alt_index.get(&e.rel.to_lowercase()) {
                    Some(alts) => {
                        let others: Vec<&String> = alts.iter().filter(|k| *k != &e.rel).collect();
                        if others.len() == 1 {
                            let old = others[0];
                            let rec = alt_recs.get(old).cloned();
                            if !dry_run {
                                db.remove(old.as_bytes())?; // old casing dropped; new one upserted below
                            }
                            (rec, true)
                        } else {
                            (None, false) // zero (truly new) or several stale: prune handles leftovers
                        }
                    }
                    None => (None, false),
                }
            }
            None => (None, false),
        };
        let fresh = cached
            .as_ref()
            .map(|c| c.size == e.size && c.mtime_ns == e.mtime_ns && c.kind == "file")
            .unwrap_or(false);
        if fresh && !force_hash {
            let c = cached.as_ref().unwrap();
            let have_all = algos.iter().all(|a| c.hashes.contains_key(a));
            if have_all {
                if adopted && !dry_run {
                    put_rec(db, &e.rel, c)?;
                }
                eff.insert(
                    e.rel.clone(),
                    EffRec {
                        kind: "file".into(),
                        size: c.size,
                        mtime_ns: c.mtime_ns,
                        hashes: c.hashes.clone(),
                    },
                );
                n_done += 1;
                n_fast_hit += 1;
                trace!(rel = %e.rel, "cache-hit");
                if n_done.is_multiple_of(100) || last_prog.elapsed().as_secs() >= 5 {
                    info!(
                        root = %root.display(),
                        done = n_done,
                        total = total_live,
                        hashed = n_hashed,
                        fast_hit = n_fast_hit,
                        elapsed_s = t_eff.elapsed().as_secs_f64(),
                        "hash progress"
                    );
                    last_prog = std::time::Instant::now();
                }
                continue;
            }
        }
        // stale or missing or --no-fast or missing algos: drop stale entry, rehash
        if !dry_run && cached.is_some() && !fresh {
            db.remove(e.rel.as_bytes())?; // preemptively drop whole path record
        }
        let have_all = cached
            .as_ref()
            .map(|c| algos.iter().all(|a| c.hashes.contains_key(a)))
            .unwrap_or(false);
        if !fast || !fresh || force_hash || !have_all {
            debug!(rel = %e.rel, path = %e.abs.display(), "hashing");
            let hashes =
                hash_file(&e.abs, algos).with_context(|| format!("hash {}", e.abs.display()))?;
            trace!(rel = %e.rel, algos = ?hashes.keys().collect::<Vec<_>>(), "hashed");
            if !dry_run {
                put_rec(
                    db,
                    &e.rel,
                    &FileRec {
                        kind: "file".into(),
                        size: e.size,
                        mtime_ns: e.mtime_ns,
                        hashes: hashes.clone(),
                    },
                )?;
            }
            eff.insert(
                e.rel.clone(),
                EffRec {
                    kind: "file".into(),
                    size: e.size,
                    mtime_ns: e.mtime_ns,
                    hashes,
                },
            );
            n_done += 1;
            n_hashed += 1;
        } else {
            // fast hit path (fresh && fast && have_all handled above); fallback hash
            let c = cached.as_ref().unwrap();
            eff.insert(
                e.rel.clone(),
                EffRec {
                    kind: "file".into(),
                    size: c.size,
                    mtime_ns: c.mtime_ns,
                    hashes: c.hashes.clone(),
                },
            );
            n_done += 1;
            n_fast_hit += 1;
            trace!(rel = %e.rel, "cache-hit");
        }
        if n_done.is_multiple_of(100) || last_prog.elapsed().as_secs() >= 5 {
            info!(
                root = %root.display(),
                done = n_done,
                total = total_live,
                hashed = n_hashed,
                fast_hit = n_fast_hit,
                elapsed_s = t_eff.elapsed().as_secs_f64(),
                "hash progress"
            );
            last_prog = std::time::Instant::now();
        }
    }
    // prune DB rows for files no longer on disk (or now excluded)
    let mut pruned = 0usize;
    if !dry_run {
        let existing = load_all_records(db)?;
        for rel in existing.keys() {
            if !live_set.contains(rel) {
                db.remove(rel.as_bytes())?;
                pruned += 1;
                trace!(rel = %rel, "prune cache");
            }
        }
        db.flush()?;
    }
    let n_files = eff.values().filter(|r| r.kind == "file").count();
    let n_dirs = eff.values().filter(|r| r.kind == "dir").count();
    info!(
        root = %root.display(),
        files = n_files,
        dirs = n_dirs,
        hashed = n_hashed,
        fast_hit = n_fast_hit,
        pruned,
        elapsed_s = elapsed_s(t_eff),
        "effective done"
    );
    Ok(eff)
}

/// Effective map for a record input: the DB read as-is, with no FS access and
/// no writes. Cache rows and filtered paths are dropped.
pub fn load_record_side(db_path: &Path, common: &CommonOpts) -> Result<HashMap<String, EffRec>> {
    if !db_path.exists() {
        bail!("record {} not found", db_path.display());
    }
    let db = sled::open(db_path)
        .with_context(|| format!("open record {} (corrupt?)", db_path.display()))?;
    let all = load_all_records(&db)?;
    let mut out = HashMap::new();
    for (rel, r) in all {
        if is_cache_rel(&rel) {
            continue;
        }
        if is_excluded(
            &rel,
            &common.includes,
            &common.excludes,
            common.case_sensitive,
        ) {
            continue;
        }
        out.insert(
            rel,
            EffRec {
                kind: r.kind,
                size: r.size,
                mtime_ns: r.mtime_ns,
                hashes: r.hashes,
            },
        );
    }
    if !common.case_sensitive {
        let mut map: HashMap<String, Vec<String>> = HashMap::new();
        for k in out.keys() {
            map.entry(k.to_lowercase()).or_default().push(k.clone());
        }
        for (_, v) in map {
            let uniq: HashSet<&String> = v.iter().collect();
            if uniq.len() > 1 {
                bail!(
                    "mixed-case collision in record {}: {}",
                    db_path.display(),
                    v.join(" vs ")
                );
            }
        }
    }
    Ok(out)
}

/// One side of a `compare`: either a folder root or a record directory.
#[derive(Debug)]
pub enum Side {
    Folder(PathBuf),
    Record(PathBuf),
}

/// Classify an input path by basename: `girpr-cache*` means record, else folder.
pub fn classify(p: &Path) -> Side {
    if is_record_path(p) {
        Side::Record(p.to_path_buf())
    } else {
        Side::Folder(p.to_path_buf())
    }
}

/// Effective map for a record or folder side. All four combinations work.
///
/// A folder side lazily populates and may leave untouched entries stale — by
/// design, since the cache is a cache and disk is the truth.
pub fn load_side(
    side: &Side,
    common: &CommonOpts,
    mode: ScanMode,
) -> Result<HashMap<String, EffRec>> {
    match side {
        Side::Record(dbp) => {
            info!(record = %dbp.display(), "load record side");
            let m = load_record_side(dbp, common)?;
            info!(record = %dbp.display(), entries = m.len(), "record loaded");
            Ok(m)
        }
        Side::Folder(root) => {
            if !root.is_dir() {
                bail!("folder {} not found", root.display());
            }
            let db_path = root.join(CACHE_PREFIX);
            if !db_path.exists() {
                // missing cache: just create it
                info!(cache = %db_path.display(), "cache missing, creating");
                let db = open_db(&db_path, common.case_sensitive, false, false)?;
                let eff = build_effective_folder(root, &db, common, mode)?;
                return Ok(eff);
            }
            match open_db(
                &db_path,
                common.case_sensitive,
                common.ignore_cache,
                !mode.dry_run,
            ) {
                Ok(db) => build_effective_folder(root, &db, common, mode),
                Err(e) => {
                    warn!(root = %root.display(), error = format!("{:#}", e), "cache open failed");
                    Err(e.context(format!(
                        "cache for {} (use --ignore-cache to rebuild)",
                        root.display()
                    )))
                }
            }
        }
    }
}
