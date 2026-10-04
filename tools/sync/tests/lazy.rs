//! Lazy digest resolution: how much a run reads, and what it concludes.
//!
//! Each case states its own expected verdict and its own expected `hashed`
//! count. There is no differential oracle and no generator: for a fixture built
//! by hand the answer is known before the run, and writing it down pins *more*
//! than comparing two runs would — the counters as well as the verdict.
//!
//! The `hashed` expectations in the doc comment on each test are the **lazy**
//! numbers. The suite was written against the eager code first, with the eager
//! numbers, and flipped once the short circuit landed; the diff of that flip is
//! the record of what laziness actually changed.

mod common;

use std::path::Path;

use common::*;
use girsync::cache::CACHE_PREFIX;
use girsync::commands::verdict;
use girsync::config::ScanMode;
use girsync::diff::{Diff, diff_maps};
use girsync::effective::{
    Side, classify, ensure_distinct_sides, open_folder_cache, open_side, resolve_folder,
    resolve_side, scan_stat_only,
};
use girsync::planner::{SideRequest, plan_pairs};
use girsync::{CommonOpts, TrustOpts, UpdateOpts, cmd_sync};

/// What one two-sided run produced: the verdict, and how much it read.
struct Run {
    diff: Diff,
    /// Summed over both sides. Read per-side where a case needs to say which
    /// side paid.
    hashed: usize,
    src_hashed: usize,
    dst_hashed: usize,
}

impl Run {
    /// The lines a user would see, in report order.
    fn lines(&self) -> Vec<String> {
        verdict(&self.diff).into_iter().map(|(_, l)| l).collect()
    }

    fn exit(&self) -> i32 {
        if self.diff.is_empty() { 0 } else { 4 }
    }
}

/// Resolve both sides and diff them, the way `cmd_compare` does.
///
/// The phases are driven here rather than through `cmd_compare` because
/// `cmd_compare` returns only an exit code, and the counter is the thing under
/// test. `cmd_compare_reports_the_same_verdict_and_its_exit_code` covers the
/// wiring.
fn run_pair(src: &Path, dst: &Path, common: &CommonOpts, trust: TrustOpts) -> Run {
    let s_side = classify(src);
    let d_side = classify(dst);
    ensure_distinct_sides(&s_side, &d_side).unwrap();
    let mode = |no_trust: bool| ScanMode {
        no_trust_cached_hashes: no_trust,
        dry_run: false,
    };
    // Both handles stay live across plan and resolve.
    let mut s = open_side(&s_side, common, mode(trust.no_trust_src)).unwrap();
    let mut d = open_side(&d_side, common, mode(trust.no_trust_dst)).unwrap();

    let plans = plan_pairs(
        SideRequest {
            entries: &s.phase_a.map,
            algos: &common.algos,
            no_trust: trust.no_trust_src,
        },
        SideRequest {
            entries: &d.phase_a.map,
            algos: &common.algos,
            no_trust: trust.no_trust_dst,
        },
        common.case_sensitive,
    );
    let sm = resolve_side(&mut s, mode(trust.no_trust_src), &plans.src).unwrap();
    let dm = resolve_side(&mut d, mode(trust.no_trust_dst), &plans.dst).unwrap();
    Run {
        diff: diff_maps(&sm.map, &dm.map, &common.algos, common.case_sensitive),
        hashed: sm.stats.hashed + dm.stats.hashed,
        src_hashed: sm.stats.hashed,
        dst_hashed: dm.stats.hashed,
    }
}

/// Four file pairs, each dst content a different length from src's, so every
/// pair differs in size and therefore in stat.
fn four_differing(t: &TempRoot) -> (std::path::PathBuf, std::path::PathBuf) {
    pair(
        t,
        &[
            ("a.txt", Some(b"alpha"), Some(b"a")),
            ("b.txt", Some(b"bravo-longer"), Some(b"bb")),
            ("sub/c.txt", Some(b"charlie-longer"), Some(b"ccc")),
            ("sub/deep/d.txt", Some(b"delta-longer"), Some(b"dddd")),
        ],
        &["md5"],
        false,
    )
}

/// A `SyncOpts` for the `sync` cases. Case-sensitive by default, so the counts are
/// not perturbed by a rename pass; a case that wants insensitive sets it.
fn sync_opts(src: std::path::PathBuf, dst: std::path::PathBuf) -> girsync::SyncOpts {
    let mut common = opts();
    common.case_sensitive = true;
    girsync::SyncOpts {
        src,
        dst,
        trust: TrustOpts::default(),
        missing_only: false,
        keep_extra: false,
        jobs: 1,
        common,
    }
}

// ---------------------------------------------------------------------------
// The cases
// ---------------------------------------------------------------------------

/// **cold, all stat-differ.** The headline: size alone decides all four pairs, so
/// nothing needs a digest and nothing is read.
///
/// Eager baseline: 8. This is the win.
#[test]
fn cold_all_stat_differing_pairs_read_nothing() {
    let t = TempRoot::new("lz_cold_differ");
    let (src, dst) = four_differing(&t);
    let r = run_pair(&src, &dst, &opts(), TrustOpts::default());
    assert_eq!(
        r.lines(),
        vec![
            "CHANGED a.txt",
            "CHANGED b.txt",
            "CHANGED sub/c.txt",
            "CHANGED sub/deep/d.txt",
            "SUMMARY missing=0 extra=0 changed=4 type_conflict=0 case_mismatch=0 total_diff=4",
        ]
    );
    assert_eq!(r.hashed, 0, "the verdict needed no bytes");
}

/// **cold, all stat-equal.** The other end of the same axis: mtimes pinned, so
/// stat agrees and the digest is the only thing that can decide. All eight files
/// are read — laziness is not "never hash".
#[test]
fn cold_all_stat_equal_pairs_are_all_hashed() {
    let t = TempRoot::new("lz_cold_equal");
    let (src, dst) = pair(
        &t,
        &[
            ("a.txt", Some(b"alpha"), Some(b"alpha")),
            ("b.txt", Some(b"bravo"), Some(b"bravo")),
            ("sub/c.txt", Some(b"charlie"), Some(b"charlie")),
            ("sub/deep/d.txt", Some(b"delta"), Some(b"delta")),
        ],
        &["md5"],
        false,
    );
    let r = run_pair(&src, &dst, &opts(), TrustOpts::default());
    assert_eq!(
        r.lines(),
        vec!["SUMMARY missing=0 extra=0 changed=0 type_conflict=0 case_mismatch=0 total_diff=0"]
    );
    assert_eq!(r.exit(), 0);
    assert_eq!(
        (r.src_hashed, r.dst_hashed),
        (4, 4),
        "every pair is undecided"
    );
}

/// **mixed.** Two pairs undecided, two decided by stat. Exactly the undecided
/// half is read — the count is 4, not 8 and not 0.
#[test]
fn mixed_tree_reads_only_the_stat_equal_half() {
    let t = TempRoot::new("lz_mixed");
    let (src, dst) = pair(
        &t,
        &[
            // stat-equal: same length, mtime pinned
            ("same1.txt", Some(b"equal1"), Some(b"equal1")),
            ("same2.txt", Some(b"equal2"), Some(b"equal2")),
            // stat-differs: different length
            ("diff1.txt", Some(b"different-one"), Some(b"d1")),
            ("diff2.txt", Some(b"different-two"), Some(b"d2")),
        ],
        &["md5"],
        false,
    );
    let r = run_pair(&src, &dst, &opts(), TrustOpts::default());
    assert_eq!(
        r.lines(),
        vec![
            "CHANGED diff1.txt",
            "CHANGED diff2.txt",
            "SUMMARY missing=0 extra=0 changed=2 type_conflict=0 case_mismatch=0 total_diff=2",
        ]
    );
    assert_eq!(
        (r.src_hashed, r.dst_hashed),
        (2, 2),
        "only the undecided pairs"
    );
}

/// **warm, complete.** Both caches already hold md5 for stat-matching rows, so
/// neither side opens a file.
#[test]
fn warm_complete_caches_read_nothing() {
    let t = TempRoot::new("lz_warm");
    let (src, dst) = pair(
        &t,
        &[
            ("a.txt", Some(b"alpha"), Some(b"alpha")),
            ("b.txt", Some(b"bravo"), Some(b"bravo")),
            ("sub/c.txt", Some(b"charlie"), Some(b"charlie")),
        ],
        &["md5"],
        true,
    );
    let r = run_pair(&src, &dst, &opts(), TrustOpts::default());
    assert_eq!(r.exit(), 0);
    assert_eq!((r.hashed, r.src_hashed, r.dst_hashed), (0, 0, 0));
}

/// **stat-equal, one side missing an algo.** md5 is cached on both, sha256 on
/// neither — the shape a `--hash md5` history leaves behind. Only the side that
/// lacks the digest pays for it, and the pair still resolves.
///
/// The verdict stays clean because the content is identical: this case is about
/// the count, not about backfilling correctly.
#[test]
fn stat_equal_pair_costs_only_the_side_missing_the_algorithm() {
    let t = TempRoot::new("lz_missing_algo");
    let both = &["md5", "sha256"];
    let (src, dst) = pair(
        &t,
        &[
            ("a.txt", Some(b"alpha"), Some(b"alpha")),
            ("b.txt", Some(b"bravo"), Some(b"bravo")),
        ],
        both,
        true,
    );
    strip_algo(&dst, "a.txt", "sha256");
    strip_algo(&dst, "b.txt", "sha256");

    let r = run_pair(&src, &dst, &with_algos(both), TrustOpts::default());
    assert_eq!(r.exit(), 0, "identical content, so the pair is equal");
    assert_eq!((r.src_hashed, r.dst_hashed), (0, 2), "only dst pays");
    // And the gap is closed: dst's rows now hold both algorithms.
    assert_eq!(digests_of(&dst, "a.txt"), vec!["md5", "sha256"]);
}

/// **stat-equal, src's digest poisoned.** Size and mtime match, so only the
/// content disagrees — and the cache says so, so no read is needed to find out.
///
/// This is the case that keeps laziness honest. A run that read nothing *and*
/// concluded "CHANGED" is only correct because the cached digests disagree; the
/// verdict came from the cache, not from stat.
#[test]
fn stat_equal_pair_with_a_poisoned_digest_needs_no_read() {
    let t = TempRoot::new("lz_poison");
    let (src, dst) = pair(
        &t,
        &[("a.txt", Some(b"alpha"), Some(b"alpha"))],
        &["md5"],
        true,
    );
    poison_digest(&src, "a.txt", "md5");

    let r = run_pair(&src, &dst, &opts(), TrustOpts::default());
    assert_eq!(
        r.lines(),
        vec![
            "CHANGED a.txt",
            "SUMMARY missing=0 extra=0 changed=1 type_conflict=0 case_mismatch=0 total_diff=1",
        ]
    );
    assert_eq!(r.hashed, 0, "the cached digests already disagree");
}

/// **`--hash none`.** No digest is required, so nothing is read, whatever the
/// tree looks like.
#[test]
fn hash_none_reads_nothing_on_a_differing_tree() {
    let t = TempRoot::new("lz_hash_none");
    let (src, dst) = four_differing(&t);
    let r = run_pair(&src, &dst, &with_algos(&[]), TrustOpts::default());
    assert_eq!(r.hashed, 0, "post-stage-2: an empty plan means no read");
    assert_eq!(r.diff.changed.len(), 4, "still four differences, by size");
}

/// **no-trust, stat-equal.** Distrusting the cache must cost the read even
/// against a complete one — that is the flag's whole purpose. Four pairs, both
/// sides, eight files.
#[test]
fn no_trust_on_stat_equal_pairs_forces_the_read() {
    let t = TempRoot::new("lz_notrust_equal");
    let (src, dst) = pair(
        &t,
        &[
            ("a.txt", Some(b"alpha"), Some(b"alpha")),
            ("b.txt", Some(b"bravo"), Some(b"bravo")),
            ("sub/c.txt", Some(b"charlie"), Some(b"charlie")),
            ("sub/deep/d.txt", Some(b"delta"), Some(b"delta")),
        ],
        &["md5"],
        true,
    );
    let r = run_pair(
        &src,
        &dst,
        &opts(),
        TrustOpts {
            no_trust_src: true,
            no_trust_dst: true,
        },
    );
    assert_eq!(r.exit(), 0);
    assert_eq!(
        (r.src_hashed, r.dst_hashed),
        (4, 4),
        "distrust overrides the cache"
    );
}

/// **no-trust, stat-differing.** The documented no-op: distrusting the cache
/// does not make an unequal size uncertain. Nothing is read, despite the flag on
/// both sides and a cold cache.
#[test]
fn no_trust_on_stat_differing_pairs_is_a_no_op() {
    let t = TempRoot::new("lz_notrust_differ");
    let (src, dst) = four_differing(&t);
    let r = run_pair(
        &src,
        &dst,
        &opts(),
        TrustOpts {
            no_trust_src: true,
            no_trust_dst: true,
        },
    );
    assert_eq!(r.diff.changed.len(), 4);
    assert_eq!(r.hashed, 0, "the verdict needed no bytes");
}

/// **src-only / dst-only, plus a dir.** A path on one side only is already
/// decided — `MISSING` or `EXTRA` — so it needs no digest on either side. The
/// dst-only dir likewise.
#[test]
fn one_sided_paths_and_dirs_need_no_digest() {
    let t = TempRoot::new("lz_onesided");
    let (src, dst) = pair(
        &t,
        &[
            ("only_src.txt", Some(b"source-only"), None),
            ("only_dst.txt", None, Some(b"dest-only")),
            ("shared.txt", Some(b"shared"), Some(b"shared")),
            ("only_src_dir/inner.txt", Some(b"inner"), None),
            ("shared_dir", Some(b""), Some(b"")),
        ],
        &["md5"],
        false,
    );
    let r = run_pair(&src, &dst, &opts(), TrustOpts::default());
    assert_eq!(
        r.lines(),
        vec![
            "MISSING only_src.txt",
            "MISSING only_src_dir",
            "MISSING only_src_dir/inner.txt",
            "EXTRA only_dst.txt",
            "SUMMARY missing=3 extra=1 changed=0 type_conflict=0 case_mismatch=0 total_diff=4",
        ]
    );
    // Only `shared.txt` is undecided. Eager baseline: every file on both sides,
    // including the three that are one-sided (3 on src, 2 on dst).
    assert_eq!(
        (r.src_hashed, r.dst_hashed),
        (1, 1),
        "only the shared, stat-equal file"
    );
}

/// **kind conflict.** A file on one side, a directory on the other: decided by
/// presence, so no digest.
#[test]
fn a_kind_conflict_needs_no_digest() {
    let t = TempRoot::new("lz_kind");
    let (src, dst) = pair(
        &t,
        &[("clash", Some(b"iamafile"), Some(b""))],
        &["md5"],
        false,
    );
    let r = run_pair(&src, &dst, &opts(), TrustOpts::default());
    assert_eq!(
        r.lines(),
        vec![
            "TYPE-CONFLICT clash",
            "SUMMARY missing=0 extra=0 changed=0 type_conflict=1 case_mismatch=0 total_diff=1"
        ]
    );
    assert_eq!(r.hashed, 0, "a kind conflict is decided by presence");
}

/// **case-only difference, identical content.** In insensitive mode the pair is
/// matched by lowercase, so it is compared rather than reported as
/// missing+extra — and since its stat agrees, the digest decides, which means
/// both files are read. This is the case a planner that pairs by exact key gets
/// wrong: it would plan nothing and the `CASE-MISMATCH` would be the only line.
#[test]
fn a_case_only_difference_with_equal_content_is_still_compared() {
    let t = TempRoot::new("lz_case_equal");
    let src = t.mkdirs("src");
    let dst = t.mkdirs("dst");
    wfile(&src, "a.txt", b"alpha");
    wfile(&dst, "A.txt", b"alpha");
    sync_mtime(&src.join("a.txt"), &dst.join("A.txt"));

    let insensitive = CommonOpts {
        case_sensitive: false,
        ..opts()
    };
    let r = run_pair(&src, &dst, &insensitive, TrustOpts::default());
    assert_eq!(
        r.lines(),
        vec![
            "CASE-MISMATCH a.txt <=> A.txt",
            "SUMMARY missing=0 extra=0 changed=0 type_conflict=0 case_mismatch=1 total_diff=0"
        ]
    );
    assert_eq!(
        (r.src_hashed, r.dst_hashed),
        (1, 1),
        "the pair is compared, not skipped"
    );
}

/// **case-only difference, differing content, stat equal.** Same pairing, and now
/// the digest has something to report, so the pair lands in `CHANGED` *as well
/// as* `CASE-MISMATCH`. Both come from the same `diff_maps` pass.
///
/// The content is the same *length* in different cases, which is the whole point:
/// size matches, mtime is pinned, so stat cannot decide and the digest must. A
/// case-only pair with different-length content would be decided by size, which
/// is a different test — and one that would pass with no digest at all.
#[test]
fn a_case_only_difference_with_differing_content_is_changed_too() {
    let t = TempRoot::new("lz_case_diff");
    let src = t.mkdirs("src");
    let dst = t.mkdirs("dst");
    wfile(&src, "a.txt", b"alpha");
    wfile(&dst, "A.txt", b"ALPHA");
    sync_mtime(&src.join("a.txt"), &dst.join("A.txt"));

    let insensitive = CommonOpts {
        case_sensitive: false,
        ..opts()
    };
    let r = run_pair(&src, &dst, &insensitive, TrustOpts::default());
    assert_eq!(
        r.lines(),
        vec![
            "CHANGED a.txt",
            "CASE-MISMATCH a.txt <=> A.txt",
            "SUMMARY missing=0 extra=0 changed=1 type_conflict=0 case_mismatch=1 total_diff=1",
        ]
    );
    assert_eq!(
        (r.src_hashed, r.dst_hashed),
        (1, 1),
        "unchanged by laziness"
    );
}

/// A stat-differing pair whose stale row held digests must be decided by stat,
/// not by those digests. The row is poisoned *and* the file aged, so a planner
/// that carried a stale row would report `CHANGED` for the wrong reason — and a
/// planner that carried nothing would report it for the right one. Same verdict,
/// so the case is really about the cache: after the run the stale digests are
/// gone, replaced by a stat-only row.
#[test]
fn a_stat_differing_pair_is_decided_by_stat_and_its_stale_digests_are_dropped() {
    let t = TempRoot::new("lz_stale");
    let (src, dst) = pair(
        &t,
        &[("a.txt", Some(b"alpha"), Some(b"alpha"))],
        &["md5"],
        true,
    );
    // src keeps its real content and gets a new mtime; dst keeps the old one.
    age(&src, "a.txt", 120);

    let r = run_pair(&src, &dst, &opts(), TrustOpts::default());
    assert_eq!(
        r.lines(),
        vec![
            "CHANGED a.txt",
            "SUMMARY missing=0 extra=0 changed=1 type_conflict=0 case_mismatch=0 total_diff=1"
        ]
    );
    assert_eq!(r.hashed, 0, "size already differs");
    // The real point: the stale digest is *dropped*, not merely unread. A row
    // left holding a pre-change digest is what W1 fixed, and phase A's carry
    // rule is what keeps it fixed now that the run never hashes this file.
    assert!(
        digests_of(&src, "a.txt").is_empty(),
        "the stale digest was replaced by a stat-only row"
    );
}

/// The wiring, not the counters: `cmd_compare` reaches the same verdict through
/// the phases. Guards the case where the engine is right and the command does
/// not call it.
#[test]
fn cmd_compare_reports_the_same_verdict_and_its_exit_code() {
    let t = TempRoot::new("lz_cmd");
    let (src, dst) = four_differing(&t);
    let o = girsync::CompareOpts {
        src: src.clone(),
        dst: dst.clone(),
        trust: TrustOpts::default(),
        common: opts(),
    };
    assert_eq!(girsync::cmd_compare(o, &log()).unwrap(), 4);

    // A MISSING and an EXTRA, so the buckets are not all `changed`.
    wfile(&src, "brand_new.txt", b"new");
    let o = girsync::CompareOpts {
        src: src.clone(),
        dst: dst.clone(),
        trust: TrustOpts::default(),
        common: opts(),
    };
    assert_eq!(girsync::cmd_compare(o, &log()).unwrap(), 4);
}

// ---------------------------------------------------------------------------
// `sync`
// ---------------------------------------------------------------------------
//
// `cmd_sync` returns an exit code and nothing else, so the counters are not
// reachable from outside it. Asserting them would mean either scraping the log —
// which pins a log format as a contract — or adding a return value for a test.
//
// So `sync` gets two layers instead. The harness below pins what the *phases*
// decide, which is where the number is defined; and `a_lazy_sync_records_no_digest
// _it_never_needed` pins that the *command* reaches those phases, by looking at
// the cache a run leaves behind. The second is the one that would actually fail if
// `cmd_sync` kept calling the monolithic scan.

/// Per-side `hashed` for a `sync`, computed the way `cmd_sync` computes it.
fn sync_counts(src: &Path, dst: &Path, common: &CommonOpts, trust: TrustOpts) -> (usize, usize) {
    let mode = |no_trust| ScanMode {
        no_trust_cached_hashes: no_trust,
        dry_run: false,
    };
    let src_db = open_folder_cache(src, common, mode(trust.no_trust_src), false).unwrap();
    let dst_db = open_folder_cache(dst, common, mode(trust.no_trust_dst), false).unwrap();
    let src_a = scan_stat_only(src, &src_db, common, mode(trust.no_trust_src)).unwrap();
    let dst_a = scan_stat_only(dst, &dst_db, common, mode(trust.no_trust_dst)).unwrap();
    // `common.case_sensitive`, not `true`: the planner runs *before* the rename
    // pass, so it has to pair the sides as they are on disk. See
    // `a_case_only_difference_in_content_is_copied_not_merely_renamed`.
    let plans = plan_pairs(
        SideRequest {
            entries: &src_a.map,
            algos: &common.algos,
            no_trust: trust.no_trust_src,
        },
        SideRequest {
            entries: &dst_a.map,
            algos: &common.algos,
            no_trust: trust.no_trust_dst,
        },
        common.case_sensitive,
    );
    let sm = resolve_folder(src, &src_db, mode(trust.no_trust_src), &src_a, &plans.src).unwrap();
    let dm = resolve_folder(dst, &dst_db, mode(trust.no_trust_dst), &dst_a, &plans.dst).unwrap();
    (sm.stats.hashed, dm.stats.hashed)
}

/// A cold tree where every pair differs in size costs the scan nothing. This is
/// the same shape as `cold_all_stat_differing_pairs_read_nothing`, and it is the
/// larger win: `sync` is where the I/O volume lives.
#[test]
fn a_sync_scan_reads_nothing_when_every_pair_differs_in_size() {
    let t = TempRoot::new("lz_sync_differ");
    let (src, dst) = four_differing(&t);
    assert_eq!(
        sync_counts(&src, &dst, &opts(), TrustOpts::default()),
        (0, 0),
        "size decides every pair, so no digest is needed on either side"
    );
}

/// A src-only file is copied without ever needing a digest at scan time: the pair
/// is `MISSING`, which presence settles, and `copy_one` rehashes both sides to
/// verify the copy and returns the record it caches. So a src-only file costs the
/// *scan* nothing even though the copy itself reads it twice.
#[test]
fn a_src_only_file_costs_the_sync_scan_no_digest() {
    let t = TempRoot::new("lz_sync_srconly");
    let (src, dst) = pair(
        &t,
        &[
            ("shared.txt", Some(b"shared"), Some(b"shared")),
            ("only_src.txt", Some(b"only-here"), None),
            ("only_dst.txt", None, Some(b"only-there")),
        ],
        &["md5"],
        false,
    );
    // Cold caches, so nothing is cached and only the shared stat-equal pair counts.
    assert_eq!(
        sync_counts(&src, &dst, &opts(), TrustOpts::default()),
        (1, 1),
        "only the shared stat-equal pair is undecided"
    );
}

/// The end-to-end proof that `cmd_sync` reached the planner, and the gate that
/// would fail if it did not.
///
/// Laziness is not observable in a return value here, but it *is* observable in
/// what a run leaves in the cache. On a tree whose every pair differs in size, an
/// eager scan writes an md5 for every file it read; a lazy one decides those pairs
/// from stat, never reads them, and writes a **stat-only row** instead. The
/// fingerprints are incompatible, so this fails loudly either way.
///
/// It also pins where digests still come from, which is the part that is easy to
/// break by over-applying laziness: `copy_one` hashes both sides to verify each
/// copy and returns dst's record, so **dst** ends the run with digests even though
/// its scan read nothing.
#[test]
fn a_lazy_sync_records_no_digest_it_never_needed() {
    let t = TempRoot::new("lz_sync_lazy");
    let (src, dst) = four_differing(&t);
    let mut o = sync_opts(src.clone(), dst.clone());
    o.common.case_sensitive = true;
    assert_eq!(cmd_sync(o, &log()).unwrap(), 0);

    // Every pair differed in size, so every pair was decided by stat.
    for rel in ["a.txt", "b.txt", "sub/c.txt", "sub/deep/d.txt"] {
        assert_eq!(rfile(&dst, rel), rfile(&src, rel), "{rel} was mirrored");
        assert!(
            digests_of(&src, rel).is_empty(),
            "{rel}: src's scan decided this pair from stat, so it read nothing \
             and recorded stat only. An md5 here means the scan is still eager."
        );
    }

    // dst's digests come from `copy_one`'s verify, not from its scan — which is
    // the correct outcome, and the reason laziness does not leave dst unrecorded.
    assert_eq!(
        digests_of(&dst, "a.txt"),
        vec!["md5"],
        "the copy verified the bytes and cached the record"
    );
}

/// `update` must remain fully eager. It is the command that *defines* a complete
/// record, so a lazy `update` would leave a folder cache that the all-of rule
/// later rejects.
#[test]
fn update_still_hashes_every_file() {
    let t = TempRoot::new("lz_update");
    let dir = t.mkdirs("w");
    wfile(&dir, "a.txt", b"alpha");
    wfile(&dir, "sub/b.txt", b"bravo");
    let o = UpdateOpts {
        dir: dir.clone(),
        common: opts(),
    };
    assert_eq!(girsync::cmd_update(o, &log()).unwrap(), 0);
    assert_eq!(digests_of(&dir, "a.txt"), vec!["md5"]);
    assert_eq!(digests_of(&dir, "sub/b.txt"), vec!["md5"]);
}

/// Both cache handles stay live across all three phases now, so the
/// self-collision rule has to keep firing *before* any open. Two writable
/// handles on one file would be refused by redb with a far worse message.
#[test]
fn the_wider_handle_window_still_refuses_a_self_collision() {
    let t = TempRoot::new("lz_self");
    let dir = t.mkdirs("w");
    wfile(&dir, "a.txt", b"alpha");
    girsync::cmd_update(
        UpdateOpts {
            dir: dir.clone(),
            common: opts(),
        },
        &log(),
    )
    .unwrap();
    let o = girsync::CompareOpts {
        src: dir.clone(),
        dst: dir.join(CACHE_PREFIX),
        trust: TrustOpts::default(),
        common: opts(),
    };
    let err = girsync::cmd_compare(o, &log()).unwrap_err();
    assert!(
        format!("{:#}", err).contains("same cache"),
        "a folder against the record inside it is still refused: {:#}",
        err
    );
}

/// The `Side` re-export is what `classify` returns; assert the classification
/// itself so a case's fixture is not silently the wrong shape.
#[test]
fn fixtures_classify_as_folders() {
    let t = TempRoot::new("lz_classify");
    let dir = t.mkdirs("w");
    assert!(matches!(classify(&dir), Side::Folder(_)));
    assert!(matches!(classify(&dir.join(CACHE_PREFIX)), Side::Record(_)));
}

/// A folder side still corrects its own cache before the planner ever sees it:
/// a row whose stat no longer matches disk loses its digests, and a row whose
/// file is gone is dropped outright. Neither needs the other side.
///
/// This is the ordering that makes stage 3 safe. The planner reads `SideEntry`,
/// and if a stale digest could survive into that map the pair would be judged
/// against a pre-change hash — so the correction has to happen inside the scan,
/// not as a step someone remembers to run later.
///
/// Covers both triggers at once: `stale.txt` grows (row survives, digests
/// dropped), `gone.txt` is deleted (row dropped entirely). `keep.txt` is the
/// control — untouched, so its digests must still be there afterwards.
#[test]
fn a_folder_prunes_its_own_cache_before_the_planner_sees_it() {
    let t = TempRoot::new("lz_prune");
    let (src, dst) = pair(
        &t,
        &[
            ("keep.txt", Some(b"keep"), Some(b"keep")),
            ("stale.txt", Some(b"stale"), Some(b"stale")),
            ("gone.txt", Some(b"gone"), Some(b"gone")),
        ],
        &["md5"],
        true,
    );
    assert_eq!(digests_of(&src, "stale.txt"), ["md5"]);
    assert!(recs_of(&src).contains_key("gone.txt"));

    std::fs::write(src.join("stale.txt"), b"stale-and-longer").unwrap();
    std::fs::remove_file(src.join("gone.txt")).unwrap();

    let (sm, dm) = resolve_both(&src, &dst, TrustOpts::default());

    // The orphan is pruned, counted, and gone from the cache.
    assert_eq!(sm.stats.pruned, 1, "the deleted file's row is pruned");
    assert_eq!(dm.stats.pruned, 0, "dst was not touched");
    assert!(
        !recs_of(&src).contains_key("gone.txt"),
        "the orphan row is dropped, not merely ignored"
    );

    // The stale row survives — the file is still there — but with no digests.
    assert!(
        recs_of(&src).contains_key("stale.txt"),
        "a stat-changed row is corrected, not removed"
    );
    assert!(
        digests_of(&src, "stale.txt").is_empty(),
        "a stale row's digests are dropped before the planner can read them"
    );
    assert_eq!(
        digests_of(&src, "keep.txt"),
        ["md5"],
        "an untouched row keeps its digests"
    );

    // And the verdicts are unaffected: pruning corrected the cache, it did not
    // change what the tree says. `gone.txt` was deleted from *src*, so dst still
    // has it — `EXTRA`, and still reported even though the orphan row on the src
    // side is gone.
    assert_eq!(
        verdict(&diff_maps(&sm.map, &dm.map, &["md5".to_string()], true)),
        [
            ("extra", "EXTRA gone.txt".to_string()),
            ("changed", "CHANGED stale.txt".to_string()),
            (
                "summary",
                "SUMMARY missing=0 extra=1 changed=1 type_conflict=0 case_mismatch=0 total_diff=2"
                    .to_string()
            )
        ]
    );
}

/// Pruning is a correctness step, so `--no-trust-cached-hashes` must not be able
/// to switch it off. Distrusting a cache means "do not *reuse* it", never "keep
/// the rows that are wrong".
#[test]
fn distrusting_the_cache_does_not_suppress_pruning() {
    let t = TempRoot::new("lz_prune_nt");
    let (src, dst) = pair(
        &t,
        &[("gone.txt", Some(b"gone"), Some(b"gone"))],
        &["md5"],
        true,
    );
    std::fs::remove_file(src.join("gone.txt")).unwrap();

    let (sm, _dm) = resolve_both(
        &src,
        &dst,
        TrustOpts {
            no_trust_src: true,
            no_trust_dst: false,
        },
    );
    assert_eq!(sm.stats.pruned, 1);
    assert!(!recs_of(&src).contains_key("gone.txt"));
}
