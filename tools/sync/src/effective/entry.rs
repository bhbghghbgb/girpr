//! What the two phases exchange: the entry types, the counters, and the write rule.
//!
//! Three kinds of thing live here, and they belong together because a reader of
//! either phase needs all three:
//!
//! - **the entries** — [`SideEntry`] is what phase A reports, [`EffRec`] is what
//!   phase C produces. They are deliberately *not* the same type, and
//!   [`SideEntry`]'s doc says why.
//! - **the counters** — [`ScanStats`], which travel with the map they describe in
//!   [`SideScan`].
//! - **the write rule** — [`merge_row`], plus the [`stat_matches`] predicate it
//!   keys on. This is a cache-durability rule rather than a scan detail, so it has
//!   exactly one definition whatever calls it.

use std::collections::HashMap;

use anyhow::Result;

use crate::cache::{CacheDb, CacheWrite, FileRec};
use crate::config::ScanMode;
use crate::hash::DigestSet;

/// What one side's scan actually did.
///
/// Counters, not a verdict: they describe how much work the run performed. They
/// are the only honest measure of laziness — the diff output is identical either
/// way, so a run that hashes nothing and a run that hashes everything print the
/// same thing. They are returned from the scan rather than emitted as log fields
/// alone, because a distinction nothing can assert is a distinction nothing
/// distinguishes.
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
    ///
    /// "Wholly from the cache" is literal — a path the plan left alone *and* the
    /// cache had nothing for is [`stat_only`](Self::stat_only), not a hit. An
    /// empty algo set over a cold tree produces no hits at all, which is the
    /// honest answer: nothing was served from a cache, because nothing was
    /// needed and nothing was read.
    pub cache_hit: usize,
    /// Files that needed no digest at all: either the request set was empty
    /// (`--hash none`), or the pair was settled by stat before a digest was ever
    /// asked for.
    ///
    /// Distinct from `cache_hit` because a run that read nothing and served nothing
    /// is a different claim from one that read nothing and served everything, and
    /// merging the two would let a run look efficient for the wrong reason.
    pub stat_only: usize,
    /// Cache rows dropped for paths no longer live (or now filtered out).
    pub pruned: usize,
}

/// A side's map at one phase, plus the counters describing how it was built.
///
/// The two travel together because the counters are only meaningful next to the
/// map they were produced for, and a caller that drops the map has no use for
/// them either. Generic over the entry type because both phases produce a map:
/// phase A an [`SideEntry`] per path, phase C an [`EffRec`] per path.
#[derive(Debug, Default)]
pub struct SideScan<T = EffRec> {
    /// The map, keyed by relative path.
    pub map: HashMap<String, T>,
    pub stats: ScanStats,
}

impl SideScan<EffRec> {
    /// Count a resolved map's files and dirs, leaving the work counters at zero.
    ///
    /// For a record side, which is read with no filesystem access: there is no
    /// hashing to report and nothing to prune, so zero is the truth rather than
    /// a stand-in for "unknown".
    pub(super) fn of_map(map: HashMap<String, EffRec>) -> Self {
        let stats = ScanStats {
            files: map.values().filter(|r| r.kind == "file").count(),
            dirs: map.values().filter(|r| r.kind == "dir").count(),
            ..ScanStats::default()
        };
        Self { map, stats }
    }
}

impl SideScan<SideEntry> {
    /// Count an entry set's files and dirs, leaving the work counters at zero.
    pub(super) fn of_entries(map: HashMap<String, SideEntry>) -> Self {
        let stats = ScanStats {
            files: map.values().filter(|e| e.is_file()).count(),
            dirs: map.values().filter(|e| !e.is_file()).count(),
            ..ScanStats::default()
        };
        Self { map, stats }
    }
}

/// A resolved path: kind, stat data, and whatever hashes were needed.
/// Hash values are raw digest bytes (see [`crate::hash::hash_file`]).
///
/// `PartialEq` exists so a test can assert that two runs produced *the same
/// effective map*, which is the strongest available statement of the dry-run
/// contract. Comparing printed verdicts is weaker: two runs can agree on every
/// `CHANGED` line while one of them hashed a different set of files and reached
/// the same conclusion by luck. Equality of the map cannot be faked that way,
/// because the map holds the digests themselves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffRec {
    /// "file" or "dir"
    pub kind: String,
    pub size: u64,
    pub mtime_ns: i64,
    pub hashes: DigestSet,
    /// True when the live path is itself a link or junction. Always false for a
    /// record side, which has no filesystem to ask.
    ///
    /// Comparison ignores it - the walk follows links, so the kind and stat are
    /// the target's - but the deletion plan must not: a path *under* a link
    /// resolves through it, and removing it removes a file in the target. See
    /// [`crate::link`] and `Plan::delete_files`.
    pub is_link: bool,
}

impl EffRec {
    pub(super) fn dir(is_link: bool) -> Self {
        Self {
            kind: "dir".into(),
            size: 0,
            mtime_ns: 0,
            hashes: DigestSet::default(),
            is_link,
        }
    }
}

/// True when `prior` still describes a file at exactly this stat.
///
/// This is the *only* definition of row freshness in the crate, and it is
/// deliberately also the only thing [`merge_row`] consults. A run that decides
/// "is this row reusable?" by one rule and decides "what do I store?" by another
/// can write back a digest it just decided was stale.
pub(super) fn stat_matches(prior: Option<&FileRec>, size: u64, mtime_ns: i64) -> bool {
    prior
        .map(|c| c.kind == "file" && c.size == size && c.mtime_ns == mtime_ns)
        .unwrap_or(false)
}

/// The row a scan stores for one file: the merge-write rule, as a pure function.
///
/// `computed` is what this run hashed (empty for `--hash none` and for a
/// stat-only pass); `prior` is whatever the cache held, or `None`.
///
/// **The write shape.** See [`crate::effective::scan_stat_only`] for the table; the property worth
/// naming here is that trust is absent from it. Distrusting the cache means
/// "re-read the file", not "forget what we already know about it"; keying the
/// write on the trust flag would make every `update` run (which always distrusts)
/// discard digests it never recomputed.
///
/// The unchanged branch is what keeps a digest-free scan from destroying good
/// digests: `--hash none` and stat-only recording both land here with an empty
/// `computed`, so they record the new stat and leave the rest of the row alone.
///
/// A named function rather than an expression inside the scan loop because it is a
/// cache-durability rule, not a scan detail, and the rule belongs in one place
/// whatever calls it.
pub(super) fn merge_row(
    size: u64,
    mtime_ns: i64,
    computed: DigestSet,
    prior: Option<&FileRec>,
) -> FileRec {
    let hashes = match prior.filter(|c| stat_matches(Some(c), size, mtime_ns)) {
        Some(c) => {
            // `merge` is `other` into `self` with `self` winning, so the prior row is
            // `self` and this run's computed digests are `other` — computed replaces,
            // everything else rides along.
            let mut merged = c.hashes.clone();
            merged.merge(&computed);
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

/// One side's contribution to a pair decision: what phase A learned about a
/// path without reading its bytes.
///
/// This is phase A's whole output, and it is deliberately *not* an [`EffRec`].
/// An `EffRec` claims to answer a diff question, and answering it is phase C's
/// job; an entry here only says what the path is and what the cache already
/// holds for it. Keeping them apart is what lets the planner see a side's full
/// digest availability before committing to a read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SideEntry {
    /// "file" or "dir"
    pub kind: String,
    pub size: u64,
    pub mtime_ns: i64,
    /// Digests carried **only** when the stat still matches the row, and
    /// **every** algorithm on that row — requested or not, because the planner
    /// needs the whole availability picture to choose one per path.
    ///
    /// Empty for a dir, for a new path, and for a path whose stat moved. That
    /// last case is the load-bearing one: a stale row's digests must never reach
    /// the planner, or a changed file gets judged against its pre-change digest.
    pub cached: DigestSet,
    /// Whether the cache row's stat still matches disk.
    ///
    /// Not derivable from `cached` being empty, because a fresh row can carry no
    /// digests at all — that is what a stat-only `--hash-all-of none` scan records,
    /// and it is a real state. Phase C needs the distinction to reconstruct the prior
    /// row and hand it to `merge_row`, so it is carried explicitly rather than
    /// inferred.
    pub fresh: bool,
    /// True when the live path is itself a link or junction. Carried from the
    /// walk ([`crate::scan::LiveEnt::is_link`]) so phase C can put it on the
    /// resolved [`EffRec`]; a record side has no filesystem and leaves it false.
    pub is_link: bool,
}

impl SideEntry {
    pub(super) fn dir(is_link: bool) -> Self {
        Self {
            kind: "dir".into(),
            size: 0,
            mtime_ns: 0,
            cached: DigestSet::default(),
            fresh: true,
            is_link,
        }
    }

    /// True when this entry is a file rather than a directory. Dirs are compared
    /// by presence alone and never need a digest.
    pub fn is_file(&self) -> bool {
        self.kind == "file"
    }

    /// The cache row as phase C needs it back, or `None` when the stat moved —
    /// which is what makes [`merge_row`] able to distinguish a merge over the
    /// stored digests from a replacement of them.
    pub(super) fn prior(&self) -> Option<FileRec> {
        self.fresh.then(|| FileRec {
            kind: "file".into(),
            size: self.size,
            mtime_ns: self.mtime_ns,
            hashes: self.cached.clone(),
        })
    }
}

/// What a side is *able* to do — two independent bits, not one property of the
/// cache handle.
///
/// `CacheDb::is_read_only` answers "can this handle write?". It does not answer
/// "can this side produce a digest?", and the two come apart in both directions
/// within a single command:
///
/// | side                          | handle     | can_hash_from_disk | can_write_cache |
/// | ----------------------------- | ---------- | ------------------ | --------------- |
/// | folder, normal run            | ReadWrite  | yes                | yes             |
/// | folder, `sync --dry-run`, `compare-self` | ReadOnly | yes      | no              |
/// | record                        | ReadOnly  | no                 | no              |
///
/// Reading a read-only handle as "cannot hash" would make the planner refuse to
/// hash a dry run's folder — the one case where hashing is the only thing left to
/// do. Reading it as "can hash and write" would try to hash a record, which has no
/// filesystem to hash.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SideCapability {
    /// Can produce a digest that is not already cached, by reading the file.
    pub can_hash_from_disk: bool,
    /// Can persist what it produces.
    pub can_write_cache: bool,
}

impl SideCapability {
    /// A folder side. Full filesystem access regardless of how the cache was
    /// opened, so it can always hash; persisting needs both a writable handle
    /// and a run that is permitted to write.
    pub fn for_folder(cache: &CacheDb, mode: ScanMode) -> Self {
        Self {
            can_hash_from_disk: true,
            can_write_cache: !mode.dry_run && !cache.is_read_only(),
        }
    }

    /// A record side: no filesystem access, so it can neither produce a digest
    /// it does not already hold nor store one.
    pub fn record() -> Self {
        Self {
            can_hash_from_disk: false,
            can_write_cache: false,
        }
    }
}

/// Record a directory as presence-only, leaving an existing dir row alone.
///
/// A dir row is fully determined by its key, so rewriting an identical one per
/// scan would be pure write amplification on a tree with many directories. The
/// write is still made when the row is absent or is a *file* row: a file that
/// became a directory must lose its digests, or a later scan would read a
/// directory's stat against a digest taken from the file that preceded it.
pub(super) fn put_dir_row(w: &mut CacheWrite<'_>, rel: &str) -> Result<()> {
    if w.get(rel)?.map(|c| c.kind != "dir").unwrap_or(true) {
        w.put(rel, &FileRec::dir())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file row carrying `md5` and `sha256` digests, distinguishable by a
    /// single byte so an assertion cannot pass on length alone.
    fn row(size: u64, mtime_ns: i64, md5: u8, sha256: u8) -> FileRec {
        let mut hashes = DigestSet::default();
        hashes.insert_named("md5", &[md5; 16]);
        hashes.insert_named("sha256", &[sha256; 32]);
        FileRec {
            kind: "file".into(),
            size,
            mtime_ns,
            hashes,
        }
    }

    /// One digest per named algorithm, all holding `byte` — so two sets built
    /// from the same call differ exactly where a caller makes them differ.
    ///
    /// Every digest is 16 bytes whatever the name, so a name this build knows but
    /// the wrong length for lands in `foreign`. That is deliberate for `blake3`
    /// (unreachable) and pointed for `sha256` (reachable), and it is what the
    /// foreign cases below need.
    fn digests(algos: &[(&str, u8)]) -> DigestSet {
        let mut set = DigestSet::default();
        for (a, b) in algos {
            set.insert_named(a, &[*b; 16]);
        }
        set
    }

    /// Every name on the row, sorted — known and foreign alike, so a dropped
    /// digest cannot hide behind the iteration order of one arm.
    fn algos_of(rec: &FileRec) -> Vec<String> {
        let mut v: Vec<String> = rec
            .hashes
            .iter()
            .map(|(a, _)| a.as_str().to_string())
            .collect();
        v.extend(rec.hashes.foreign_iter().map(|(n, _)| n.to_string()));
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
        assert_eq!(out.hashes.get_named("md5"), Some(&[7u8; 16][..]));
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
            out.hashes.get_named("md5"),
            Some(&[7u8; 16][..]),
            "and md5 took the new value"
        );
    }

    /// **A foreign digest is dropped on a stat change too.** `blake3` has no context
    /// in this build, so nothing this run did could have re-derived it — which is
    /// exactly why it must go: the rule is that a stat change makes *every* stored
    /// digest suspect, and keeping the ones this build cannot check would leave the
    /// row asserting something about content nobody here can verify.
    ///
    /// The unchanged-stat half is stated in the same breath, because the two halves
    /// are one rule and the interesting half of this is that it is not
    /// "drop what we did not recompute" — it is "drop everything, then merge".
    #[test]
    fn merge_row_drops_a_foreign_digest_when_the_stat_changed() {
        let prior = digests(&[("md5", 1), ("blake3", 2)]);
        let prior_rec = FileRec {
            kind: "file".into(),
            size: 10,
            mtime_ns: 99,
            hashes: prior,
        };
        assert_eq!(
            algos_of(&prior_rec),
            vec!["blake3", "md5"],
            "the fixture really does carry a name this build does not know"
        );

        let changed = merge_row(10, 100, digests(&[("md5", 7)]), Some(&prior_rec));
        assert_eq!(
            algos_of(&changed),
            vec!["md5"],
            "blake3 was never recomputed, and a stat change makes it suspect anyway"
        );

        let same = merge_row(10, 99, digests(&[("md5", 7)]), Some(&prior_rec));
        assert_eq!(
            algos_of(&same),
            vec!["blake3", "md5"],
            "and with the stat holding it rides along, exactly like sha256 does"
        );
        assert_eq!(same.hashes.get_named("blake3"), Some(&[2u8; 16][..]));
    }

    /// Stat unchanged: digests this run did not ask for are still valid and ride
    /// along, while the ones it did ask for are replaced.
    #[test]
    fn merge_row_merges_over_the_prior_row_when_the_stat_holds() {
        let prior = row(10, 99, 1, 2);
        let out = merge_row(10, 99, digests(&[("md5", 7)]), Some(&prior));
        assert_eq!(algos_of(&out), vec!["md5", "sha256"], "sha256 rides along");
        assert_eq!(
            out.hashes.get_named("md5"),
            Some(&[7u8; 16][..]),
            "md5 was recomputed"
        );
        assert_eq!(
            out.hashes.get_named("sha256"),
            Some(&[2u8; 32][..]),
            "sha256 was not"
        );
    }

    /// `--hash none` with an unchanged stat: stat is recorded, digests survive.
    /// This is the case that a digest-free scan must get right, since it is the
    /// only thing standing between `girsync update --hash none` and destroying
    /// every digest in the cache.
    #[test]
    fn merge_row_with_nothing_computed_preserves_digests_when_the_stat_holds() {
        let prior = row(10, 99, 1, 2);
        let out = merge_row(10, 99, DigestSet::default(), Some(&prior));
        assert_eq!(algos_of(&out), vec!["md5", "sha256"]);
        assert_eq!(out.hashes.get_named("md5"), Some(&[1u8; 16][..]));
    }

    /// `--hash none` with a *changed* stat: the row becomes stat-only. A row with
    /// a stat and no digest is a normal, valid state, and it is the expected one for
    /// every file a lazy scan decided by stat.
    #[test]
    fn merge_row_with_nothing_computed_drops_digests_when_the_stat_changed() {
        let prior = row(10, 99, 1, 2);
        let out = merge_row(10, 100, DigestSet::default(), Some(&prior));
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
