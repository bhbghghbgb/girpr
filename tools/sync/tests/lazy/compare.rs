//! `compare`: per-pair laziness, asserted as verdict **and** read count.
//!
//! Each case states what the run should conclude and how much it should read to
//! get there. The two are asserted together because the diff output alone cannot
//! tell them apart: a run that hashes everything and a run that hashes nothing
//! print the same verdicts. Only the counter distinguishes them.

use super::*;

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
        r.records(),
        vec![
            json!({"event": "changed", "path": "a.txt"}),
            json!({"event": "changed", "path": "b.txt"}),
            json!({"event": "changed", "path": "sub/c.txt"}),
            json!({"event": "changed", "path": "sub/deep/d.txt"}),
            json!({"event": "summary", "missing": 0, "extra": 0, "changed": 4,
                   "type_conflict": 0, "case_mismatch": 0, "total_diff": 4}),
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
        r.records(),
        vec![
            json!({"event": "summary", "missing": 0, "extra": 0, "changed": 0,
                    "type_conflict": 0, "case_mismatch": 0, "total_diff": 0})
        ]
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
        r.records(),
        vec![
            json!({"event": "changed", "path": "diff1.txt"}),
            json!({"event": "changed", "path": "diff2.txt"}),
            json!({"event": "summary", "missing": 0, "extra": 0, "changed": 2,
                   "type_conflict": 0, "case_mismatch": 0, "total_diff": 2}),
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
/// neither — the shape a record written with `--hash-all-of md5` leaves behind.
/// Only the side that lacks the digest pays for it, and the pair still resolves.
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
        r.records(),
        vec![
            json!({"event": "changed", "path": "a.txt"}),
            json!({"event": "summary", "missing": 0, "extra": 0, "changed": 1,
                   "type_conflict": 0, "case_mismatch": 0, "total_diff": 1}),
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
    assert_eq!(r.hashed, 0, "an empty plan means no read");
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
        r.records(),
        vec![
            json!({"event": "missing", "path": "only_src.txt"}),
            json!({"event": "missing", "path": "only_src_dir"}),
            json!({"event": "missing", "path": "only_src_dir/inner.txt"}),
            json!({"event": "extra", "path": "only_dst.txt"}),
            json!({"event": "summary", "missing": 3, "extra": 1, "changed": 0,
                   "type_conflict": 0, "case_mismatch": 0, "total_diff": 4}),
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
        r.records(),
        vec![
            json!({"event": "type-conflict", "path": "clash"}),
            json!({"event": "summary", "missing": 0, "extra": 0, "changed": 0,
                   "type_conflict": 1, "case_mismatch": 0, "total_diff": 1}),
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
        r.records(),
        vec![
            json!({"event": "case-mismatch", "src": "a.txt", "dst": "A.txt"}),
            json!({"event": "summary", "missing": 0, "extra": 0, "changed": 0,
                   "type_conflict": 0, "case_mismatch": 1, "total_diff": 0}),
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
        r.records(),
        vec![
            json!({"event": "changed", "path": "a.txt"}),
            json!({"event": "case-mismatch", "src": "a.txt", "dst": "A.txt"}),
            json!({"event": "summary", "missing": 0, "extra": 0, "changed": 1,
                   "type_conflict": 0, "case_mismatch": 1, "total_diff": 1}),
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
        r.records(),
        vec![
            json!({"event": "changed", "path": "a.txt"}),
            json!({"event": "summary", "missing": 0, "extra": 0, "changed": 1,
                   "type_conflict": 0, "case_mismatch": 0, "total_diff": 1}),
        ]
    );
    assert_eq!(r.hashed, 0, "size already differs");
    // The real point: the stale digest is *dropped*, not merely unread. A row left
    // holding a pre-change digest would survive a run that never hashes this file,
    // and phase A's carry rule is what prevents that.
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
