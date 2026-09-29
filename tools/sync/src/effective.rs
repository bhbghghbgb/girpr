//! Building the *effective* map for one side of a comparison.
//!
//! An effective map is the authoritative view of a side once the cache, the
//! glob filters, and the case mode have been applied. Disk always governs:
//! stale cache entries are dropped wholesale and rehashed, and rows for paths
//! that vanished are pruned.

use anyhow::{Context, Result, bail};
use glob::Pattern;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use tracing::{debug, info, trace, warn};

use crate::cache::{CACHE_PREFIX, CacheDb, CacheOpen, FileRec, open_db};
use crate::config::{CommonOpts, ScanMode};
use crate::filter::is_excluded;
use crate::hash::hash_file;
use crate::scan::{check_mixed_case, walk_live};
use crate::util::{elapsed_s, is_cache_rel, is_record_path};

/// A resolved path: kind, stat data, and whatever hashes were needed.
/// Hash values are raw digest bytes (see [`crate::hash::hash_file`]).
#[derive(Clone, Debug)]
pub struct EffRec {
    /// "file" or "dir"
    pub kind: String,
    pub size: u64,
    pub mtime_ns: i64,
    pub hashes: HashMap<String, Vec<u8>>,
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
/// that are no longer on disk (or are now filtered out). Writes are batched
/// and committed periodically, so an interrupted scan keeps most of its
/// progress (the commit is the durability point).
///
/// **Write shape.** A row is rewritten whenever a path is rehashed, and what it
/// keeps afterwards depends only on whether the stat still matches:
///
/// | stat            | digests stored                                    |
/// | --------------- | ------------------------------------------------- |
/// | changed         | only the algos computed this run (all old ones are suspect) |
/// | unchanged       | those, merged over the algos already on the row  |
///
/// So a run never destroys a digest it did not recompute, unless the file's
/// stat says it changed. `--hash none` and `no_trust_cached_hashes` both write
/// stat data while leaving unrequested digests intact.
///
/// In insensitive mode the on-disk name governs: cached keys are indexed by
/// lowercase so a disk/cached casing difference is fixed to the disk name first,
/// reusing the cached hashes when stat matches, instead of erroring. Multiple
/// stale alternates are pruned below, not treated as a conflict.
pub fn build_effective_folder(
    root: &Path,
    cache: &CacheDb,
    common: &CommonOpts,
    mode: ScanMode,
) -> Result<HashMap<String, EffRec>> {
    let algos: &[String] = &common.algos;
    let includes: &[Pattern] = &common.includes;
    let excludes: &[Pattern] = &common.excludes;
    let case_sensitive = common.case_sensitive;
    let max_depth = common.max_depth;
    let ScanMode {
        no_trust_cached_hashes,
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
        no_trust_cached_hashes,
        dry_run,
        algos = ?algos,
        "effective start"
    );
    let t_eff = std::time::Instant::now();
    let mut n_done: usize = 0;
    let mut n_hashed: usize = 0;
    let mut n_cache_hit: usize = 0;
    let total_live = live.len();
    let alt_recs: HashMap<String, FileRec> = if !case_sensitive {
        // Read once up front; the scan's batch handle starts from the same
        // committed state.
        cache.load_all()?
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
    // Non-dry runs funnel every cache mutation through one batched handle,
    // which auto-commits periodically so progress survives interruption.
    let mut batch = if dry_run {
        None
    } else {
        Some(cache.begin_write()?)
    };
    let mut last_prog = std::time::Instant::now();
    for e in live {
        if is_excluded(&e.rel, includes, excludes, case_sensitive) {
            continue;
        }
        if e.is_dir {
            eff.insert(e.rel.clone(), EffRec::dir());
            if let Some(w) = batch.as_mut() {
                let cur: Option<FileRec> = w.get(&e.rel)?;
                if cur.map(|c| c.kind != "dir").unwrap_or(true) {
                    w.put(&e.rel, &FileRec::dir())?;
                }
            }
            continue;
        }
        // file
        // `adopted` = cache entry came from an alternate-cased key, so the
        // disk-cased key must be (re)written even on a cache hit.
        let (cached, adopted): (Option<FileRec>, bool) = match match batch.as_mut() {
            Some(w) => w.get(&e.rel)?,
            None => cache.get(&e.rel)?,
        } {
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
                            if let Some(w) = batch.as_mut() {
                                w.remove(old)?; // old casing dropped; new one upserted below
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
        // Reuse the cached row only when the stat still matches, this side
        // trusts the cache, and every requested algo is present. Every other
        // case falls through to the rehash below.
        let trusted = cached.as_ref().filter(|c| {
            fresh && !no_trust_cached_hashes && algos.iter().all(|a| c.hashes.contains_key(a))
        });
        if let Some(c) = trusted {
            if adopted && let Some(w) = batch.as_mut() {
                w.put(&e.rel, c)?;
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
            n_cache_hit += 1;
            trace!(rel = %e.rel, "cache-hit");
        } else {
            debug!(rel = %e.rel, path = %e.abs.display(), "hashing");
            let computed =
                hash_file(&e.abs, algos).with_context(|| format!("hash {}", e.abs.display()))?;
            trace!(rel = %e.rel, algos = ?computed.keys().collect::<Vec<_>>(), "hashed");
            // What the row ends up holding is decided by `fresh` alone, never
            // by `no_trust_cached_hashes`: distrusting the cache means "re-read
            // the file", not "forget what we already know about it". Keying the
            // write on the trust flag instead would make every `update` run
            // (which always distrusts) discard digests it never recomputed.
            //
            //   stat changed   -> the content is assumed changed, so every
            //                     stored digest is suspect and only the algos
            //                     computed this run are kept;
            //   stat unchanged -> the file is assumed identical, so digests
            //                     this run did not ask for are still valid and
            //                     ride along.
            //
            // The second case is what keeps a digest-free scan from destroying
            // good digests: `--hash none` and stat-only recording both land
            // here with an empty `computed`.
            let hashes = match cached.as_ref() {
                Some(c) if fresh => {
                    let mut merged = c.hashes.clone();
                    merged.extend(computed);
                    merged
                }
                _ => computed,
            };
            if let Some(w) = batch.as_mut() {
                w.put(
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
            n_hashed += 1;
        }
        n_done += 1;
        if n_done.is_multiple_of(100) || last_prog.elapsed().as_secs() >= 5 {
            info!(
                root = %root.display(),
                done = n_done,
                total = total_live,
                hashed = n_hashed,
                cache_hit = n_cache_hit,
                elapsed_s = t_eff.elapsed().as_secs_f64(),
                "scan progress"
            );
            last_prog = std::time::Instant::now();
        }
    }
    // prune DB rows for files no longer on disk (or now excluded)
    let mut pruned = 0usize;
    if let Some(w) = batch.as_mut() {
        let existing = w.load_all()?;
        for rel in existing.keys() {
            if !live_set.contains(rel) {
                w.remove(rel)?;
                pruned += 1;
                trace!(rel = %rel, "prune cache");
            }
        }
        // Final commit: the durability point for the whole scan.
        let w = batch.take().unwrap();
        w.commit()?;
    }
    let n_files = eff.values().filter(|r| r.kind == "file").count();
    let n_dirs = eff.values().filter(|r| r.kind == "dir").count();
    info!(
        root = %root.display(),
        files = n_files,
        dirs = n_dirs,
        hashed = n_hashed,
        cache_hit = n_cache_hit,
        pruned,
        elapsed_s = elapsed_s(t_eff),
        "effective done"
    );
    Ok(eff)
}

/// Effective map for a record input: the DB read as-is, with no FS access and
/// no writes. Cache rows and filtered paths are dropped.
pub fn load_record_side(db_path: &Path, common: &CommonOpts) -> Result<HashMap<String, EffRec>> {
    let cache = CacheDb::open_record(db_path)?;
    load_record_side_from(&cache, common, &db_path.display().to_string())
}

/// [`load_record_side`] against an already-open cache.
///
/// Split out so a caller that needs the record view *and* a disk scan of the
/// same folder can do both from one handle. That is not a lock requirement —
/// two read-only handles share the file — it is a cost one: each `CacheDb`
/// builds its own copy of the file's tables, so opening twice doubles the
/// memory a large cache occupies. The same reasoning applies harder to a
/// writable handle, where the second open would be refused outright.
///
/// `label` names the record in error messages; the caller knows the path.
pub fn load_record_side_from(
    cache: &CacheDb,
    common: &CommonOpts,
    label: &str,
) -> Result<HashMap<String, EffRec>> {
    let all = cache.load_all()?;
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
                    label,
                    v.join(" vs ")
                );
            }
        }
    }
    Ok(out)
}

/// One side of a `compare`: either a folder root or a record file.
#[derive(Debug)]
pub enum Side {
    Folder(PathBuf),
    Record(PathBuf),
}

impl Side {
    /// The cache file this side reads from or writes to: a record side *is* its
    /// cache, a folder side is backed by the `<root>/girpr-cache` inside it.
    pub fn cache_path(&self) -> PathBuf {
        match self {
            Side::Record(p) => p.clone(),
            Side::Folder(root) => root.join(CACHE_PREFIX),
        }
    }
}

/// Classify an input path by basename: `girpr-cache*` means record, else folder.
pub fn classify(p: &Path) -> Side {
    if is_record_path(p) {
        Side::Record(p.to_path_buf())
    } else {
        Side::Folder(p.to_path_buf())
    }
}

/// Reject two sides that would resolve to the same cache file.
///
/// redb permits one live handle per file, so a run must never name the same
/// cache twice. This covers every shape of the collision in one predicate: the
/// same folder on both sides, the same record on both sides, and a folder
/// paired with the record that lives inside it. The identity compared is the
/// *canonical* cache path, so two spellings of one target (`F/sub/..` vs `F`)
/// collide too. Canonicalization falls back to the raw path when it fails, so a
/// missing folder still reaches `load_side` and reports "not found" rather than
/// a canonicalize error.
///
/// Beyond the handle clash, a self-collision makes the run meaningless. A
/// folder side populates its cache as a side effect of scanning, so a record
/// compared against its own folder is diffed against a view the very same run
/// is still mutating: a path reported as drifted on one line is written into the
/// record before the next, and a follow-up run over the same pair comes back
/// clean. The audit silently repairs the drift it was asked to report, which is
/// exactly the property a comparison is supposed to lack.
pub fn ensure_distinct_sides(src: &Side, dst: &Side) -> Result<()> {
    let a = cache_identity(src);
    let b = cache_identity(dst);
    if a == b {
        bail!("src and dst resolve to the same cache {}", display_path(&a));
    }
    Ok(())
}

/// Canonical cache path of a side. A path that does not exist yet cannot
/// collide with anything but an identically-spelled one, so the raw path is a
/// good enough identity.
fn cache_identity(side: &Side) -> PathBuf {
    let raw = side.cache_path();
    raw.canonicalize().unwrap_or(raw)
}

/// Render a path for humans, dropping the `\\?\` verbatim prefix that
/// `canonicalize` adds on Windows — a user never types it, so echoing it back
/// only makes the message harder to match against a flag.
fn display_path(p: &Path) -> String {
    let s = p.display().to_string();
    s.strip_prefix(r"\\?\").unwrap_or(&s).to_string()
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
                let db = open_db(
                    &db_path,
                    common.case_sensitive,
                    CacheOpen::ReadWrite {
                        ignore_cache: false,
                        backup_first: false,
                    },
                )?;
                let eff = build_effective_folder(root, &db, common, mode)?;
                return Ok(eff);
            }
            match open_db(
                &db_path,
                common.case_sensitive,
                CacheOpen::ReadWrite {
                    ignore_cache: common.ignore_cache,
                    backup_first: !mode.dry_run,
                },
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
