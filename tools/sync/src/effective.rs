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

use crate::cache::{CACHE_PREFIX, CacheDb, CacheOpen, CacheWrite, FileRec, open_db};
use crate::config::{CommonOpts, ScanMode};
use crate::filter::is_excluded;
use crate::hash::hash_file;
use crate::scan::{check_mixed_case, walk_live};
use crate::util::{elapsed_s, is_cache_rel, is_record_path};

/// What one side's scan actually did.
///
/// Counters, not a verdict: they describe how much work the run performed. They
/// are the only honest measure of laziness — the diff output is identical either
/// way, so a run that hashes nothing and a run that hashes everything print the
/// same thing. They used to be local to the scan and emitted as log fields only,
/// which made that distinction untestable; W2's whole performance claim rests on
/// asserting them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScanStats {
    /// Entries the walk found, before glob filters.
    pub live: usize,
    /// Files in the resulting map.
    pub files: usize,
    /// Dirs in the resulting map.
    pub dirs: usize,
    /// Files whose *bytes* were read to compute a digest. A dir never counts, and
    /// neither does a file served entirely from a stat-matching cached row.
    pub hashed: usize,
    /// Files answered wholly from the cache: stat matched and every requested
    /// algo was already stored, so the file was never opened.
    pub cache_hit: usize,
    /// Cache rows dropped for paths no longer live (or now filtered out).
    pub pruned: usize,
}

/// One side's resolved map, plus the counters describing how it was built.
///
/// The two travel together because the counters are only meaningful next to the
/// map they were produced for, and a caller that drops the map has no use for
/// them either.
#[derive(Debug, Default)]
pub struct SideScan {
    /// The effective map: kind, stat, and the digests that were actually needed.
    pub map: HashMap<String, EffRec>,
    pub stats: ScanStats,
}

impl SideScan {
    /// Count a map's files and dirs, leaving the work counters at zero.
    ///
    /// For a record side, which is read with no filesystem access: there is no
    /// hashing to report and nothing to prune, so zero is the truth rather than
    /// a stand-in for "unknown".
    fn of_map(map: HashMap<String, EffRec>) -> Self {
        let stats = ScanStats {
            files: map.values().filter(|r| r.kind == "file").count(),
            dirs: map.values().filter(|r| r.kind == "dir").count(),
            ..ScanStats::default()
        };
        Self { map, stats }
    }
}

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

/// True when `prior` still describes a file at exactly this stat.
///
/// This is the *only* definition of row freshness in the crate, and it is
/// deliberately also the only thing [`merge_row`] consults. A run that decides
/// "is this row reusable?" by one rule and decides "what do I store?" by another
/// can write back a digest it just decided was stale.
fn stat_matches(prior: Option<&FileRec>, size: u64, mtime_ns: i64) -> bool {
    prior
        .map(|c| c.kind == "file" && c.size == size && c.mtime_ns == mtime_ns)
        .unwrap_or(false)
}

/// The row a scan stores for one file: W1's merge-write, as a pure function.
///
/// `computed` is what this run hashed (empty for `--hash none` and for a
/// stat-only pass); `prior` is whatever the cache held, or `None`.
///
/// **The write shape.** What the row keeps afterwards depends only on whether
/// the stat still matches — never on what this run computed, and never on
/// whether the run trusted the cache:
///
/// | stat            | digests stored                                    |
/// | --------------- | ------------------------------------------------- |
/// | changed         | only the algos computed this run (all old ones are suspect) |
/// | unchanged       | those, merged over the algos already on the row  |
///
/// Trust is absent from that table on purpose. Distrusting the cache means
/// "re-read the file", not "forget what we already know about it"; keying the
/// write on the trust flag would make every `update` run (which always distrusts)
/// discard digests it never recomputed.
///
/// The unchanged branch is what keeps a digest-free scan from destroying good
/// digests: `--hash none` and stat-only recording both land here with an empty
/// `computed`, so they record the new stat and leave the rest of the row alone.
///
/// It is kept as a named function rather than inlined in the scan loop because
/// it is a cache-durability rule, not a scan detail, and two scan
/// implementations would be two chances to spell it differently.
fn merge_row(
    size: u64,
    mtime_ns: i64,
    computed: HashMap<String, Vec<u8>>,
    prior: Option<&FileRec>,
) -> FileRec {
    let hashes = match prior.filter(|c| stat_matches(Some(c), size, mtime_ns)) {
        Some(c) => {
            let mut merged = c.hashes.clone();
            merged.extend(computed);
            merged
        }
        None => computed,
    };
    FileRec {
        kind: "file".into(),
        size,
        mtime_ns,
        hashes,
    }
}

/// Record a directory as presence-only, leaving an existing dir row alone.
///
/// A dir row is fully determined by its key, so rewriting an identical one per
/// scan would be pure write amplification on a tree with many directories. The
/// write is still made when the row is absent or is a *file* row: a file that
/// became a directory must lose its digests, or a later scan would read a
/// directory's stat against a digest taken from the file that used to be there.
fn put_dir_row(w: &mut CacheWrite<'_>, rel: &str) -> Result<()> {
    if w.get(rel)?.map(|c| c.kind != "dir").unwrap_or(true) {
        w.put(rel, &FileRec::dir())?;
    }
    Ok(())
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
/// Returns the map with the counters describing how it was built; see
/// [`ScanStats`] for why those are part of the result rather than a log line.
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
) -> Result<SideScan> {
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
        // `live_set` was built from this same predicate over this same vec, so
        // membership here is the exclusion test — and a hash lookup rather than
        // re-running every glob against the path.
        if !live_set.contains(&e.rel) {
            continue;
        }
        if e.is_dir {
            eff.insert(e.rel.clone(), EffRec::dir());
            if let Some(w) = batch.as_mut() {
                put_dir_row(w, &e.rel)?;
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
        let fresh = stat_matches(cached.as_ref(), e.size, e.mtime_ns);
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
            let row = merge_row(e.size, e.mtime_ns, computed, cached.as_ref());
            if let Some(w) = batch.as_mut() {
                w.put(&e.rel, &row)?;
            }
            eff.insert(
                e.rel.clone(),
                EffRec {
                    kind: "file".into(),
                    size: row.size,
                    mtime_ns: row.mtime_ns,
                    hashes: row.hashes,
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
    let stats = ScanStats {
        live: total_live,
        files: eff.values().filter(|r| r.kind == "file").count(),
        dirs: eff.values().filter(|r| r.kind == "dir").count(),
        hashed: n_hashed,
        cache_hit: n_cache_hit,
        pruned,
    };
    info!(
        root = %root.display(),
        files = stats.files,
        dirs = stats.dirs,
        hashed = stats.hashed,
        cache_hit = stats.cache_hit,
        pruned = stats.pruned,
        elapsed_s = elapsed_s(t_eff),
        "effective done"
    );
    Ok(SideScan { map: eff, stats })
}

/// Effective map for a record input: the DB read as-is, with no FS access and
/// no writes. Cache rows and filtered paths are dropped.
pub fn load_record_side(db_path: &Path, common: &CommonOpts) -> Result<SideScan> {
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
) -> Result<SideScan> {
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
    Ok(SideScan::of_map(out))
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
/// A folder side populates and may leave untouched entries stale — by design,
/// since the cache is a cache and disk is the truth.
pub fn load_side(side: &Side, common: &CommonOpts, mode: ScanMode) -> Result<SideScan> {
    match side {
        Side::Record(dbp) => {
            info!(record = %dbp.display(), "load record side");
            let m = load_record_side(dbp, common)?;
            info!(record = %dbp.display(), entries = m.map.len(), "record loaded");
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A file row carrying `md5` and `sha256` digests, distinguishable by a
    /// single byte so an assertion cannot pass on length alone.
    fn row(size: u64, mtime_ns: i64, md5: u8, sha256: u8) -> FileRec {
        FileRec {
            kind: "file".into(),
            size,
            mtime_ns,
            hashes: [
                ("md5".to_string(), vec![md5; 16]),
                ("sha256".to_string(), vec![sha256; 32]),
            ]
            .into_iter()
            .collect(),
        }
    }

    fn digests(algos: &[(&str, u8)]) -> HashMap<String, Vec<u8>> {
        algos
            .iter()
            .map(|(a, b)| (a.to_string(), vec![*b; 16]))
            .collect()
    }

    fn algos_of(rec: &FileRec) -> Vec<String> {
        let mut v: Vec<String> = rec.hashes.keys().cloned().collect();
        v.sort();
        v
    }

    // The whole write rule is one question: does the stat still match?

    #[test]
    fn stat_matches_only_an_identical_file_row() {
        let r = row(10, 99, 1, 2);
        assert!(stat_matches(Some(&r), 10, 99), "same size and mtime");
        assert!(!stat_matches(Some(&r), 11, 99), "size moved");
        assert!(!stat_matches(Some(&r), 10, 100), "mtime moved");
        assert!(!stat_matches(None, 10, 99), "no prior row");
    }

    /// A dir row is presence-only, so its stat fields are all zero. Without the
    /// kind check, a file that replaced a directory of the same name could
    /// inherit the directory's (empty) digest set as a "fresh" match, and a
    /// 0-byte file at mtime 0 would be judged fresh against it forever.
    #[test]
    fn stat_matches_rejects_a_dir_row() {
        assert!(!stat_matches(Some(&FileRec::dir()), 0, 0));
    }

    #[test]
    fn merge_row_without_a_prior_keeps_only_what_was_computed() {
        let out = merge_row(10, 99, digests(&[("md5", 7)]), None);
        assert_eq!((out.size, out.mtime_ns), (10, 99));
        assert_eq!(algos_of(&out), vec!["md5"]);
        assert_eq!(out.hashes["md5"], vec![7u8; 16]);
    }

    /// Stat changed: the content is assumed to have changed with it, so every
    /// stored digest is suspect — including `sha256`, which this run never asked
    /// for and therefore never had reason to distrust. This is the drop that
    /// keeps a changed file from being judged against its pre-change digest.
    #[test]
    fn merge_row_drops_unrequested_digests_when_the_stat_changed() {
        let prior = row(10, 99, 1, 2);
        let out = merge_row(10, 100, digests(&[("md5", 7)]), Some(&prior));
        assert_eq!(algos_of(&out), vec!["md5"], "sha256 was never recomputed");
        assert_eq!(
            out.hashes["md5"],
            vec![7u8; 16],
            "and md5 took the new value"
        );
    }

    /// Stat unchanged: digests this run did not ask for are still valid and ride
    /// along, while the ones it did ask for are replaced.
    #[test]
    fn merge_row_merges_over_the_prior_row_when_the_stat_holds() {
        let prior = row(10, 99, 1, 2);
        let out = merge_row(10, 99, digests(&[("md5", 7)]), Some(&prior));
        assert_eq!(algos_of(&out), vec!["md5", "sha256"], "sha256 rides along");
        assert_eq!(out.hashes["md5"], vec![7u8; 16], "md5 was recomputed");
        assert_eq!(out.hashes["sha256"], vec![2u8; 32], "sha256 was not");
    }

    /// `--hash none` with an unchanged stat: stat is recorded, digests survive.
    /// This is the case that a digest-free scan must get right, since it is the
    /// only thing standing between `girsync update --hash none` and destroying
    /// every digest in the cache.
    #[test]
    fn merge_row_with_nothing_computed_preserves_digests_when_the_stat_holds() {
        let prior = row(10, 99, 1, 2);
        let out = merge_row(10, 99, HashMap::new(), Some(&prior));
        assert_eq!(algos_of(&out), vec!["md5", "sha256"]);
        assert_eq!(out.hashes["md5"], vec![1u8; 16]);
    }

    /// `--hash none` with a *changed* stat: the row becomes stat-only. A row
    /// with a stat and no digest is a normal, valid state, and under W2 it is
    /// the expected one for every file a lazy scan decided by stat.
    #[test]
    fn merge_row_with_nothing_computed_drops_digests_when_the_stat_changed() {
        let prior = row(10, 99, 1, 2);
        let out = merge_row(10, 100, HashMap::new(), Some(&prior));
        assert!(out.hashes.is_empty(), "stat-only row");
        assert_eq!((out.size, out.mtime_ns), (10, 100));
    }

    #[test]
    fn put_dir_row_writes_when_absent_or_wrong_kind() {
        let db = CacheDb::open_temp(true).unwrap();
        let mut w = db.begin_write().unwrap();
        // Absent -> written.
        put_dir_row(&mut w, "a").unwrap();
        assert!(w.get("a").unwrap().unwrap().is_dir());
        // Already a dir -> left alone, so a dir-heavy tree is not rewritten per
        // scan for no change.
        w.put("b", &FileRec::dir()).unwrap();
        put_dir_row(&mut w, "b").unwrap();
        assert!(w.get("b").unwrap().unwrap().is_dir());
        // A *file* row must lose its digests: a directory replaced it, and a
        // later scan must not read the directory's stat against them.
        w.put("c", &row(10, 99, 1, 2)).unwrap();
        put_dir_row(&mut w, "c").unwrap();
        let c = w.get("c").unwrap().unwrap();
        assert!(c.is_dir());
        assert!(c.hashes.is_empty());
    }
}
