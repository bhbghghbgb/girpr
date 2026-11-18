//! `compare`: per-pair laziness, asserted as verdict **and** read count.
//!
//! Each case states what the run should conclude and how much it should read to
//! get there. The two are asserted together because the diff output alone cannot
//! tell them apart: a run that hashes everything and a run that hashes nothing
//! print the same verdicts. Only the counter distinguishes them.
//!
//! The cases are a table: one builder per fixture/verdict, one driving test that
//! runs the shared `run_pair` seam over every row. A row carries the exact
//! assertions its fixture used to have as a standalone test, so merging the
//! harness loses no verdict, read count, exit code or post-run cache check. The
//! command-wiring case stays its own test: it drives `cmd_compare` (not the
//! `compare_detailed` seam behind `run_pair`) and is a distinct mechanism.

use super::*;

/// The assertions one table row carries forward from its former standalone test.
/// An `Option` is `None` only where the original test did not make that assertion;
/// every assertion it did make is preserved verbatim.
#[derive(Default)]
struct Expected {
    records: Option<Vec<Value>>,
    exit: Option<i32>,
    hashed: Option<usize>,
    src_hashed: Option<usize>,
    dst_hashed: Option<usize>,
    changed: Option<usize>,
}

/// One laziness case: its fixture, its run options and the assertions to make.
struct Case {
    name: &'static str,
    src: std::path::PathBuf,
    dst: std::path::PathBuf,
    common: CommonOpts,
    trust: TrustOpts,
    expected: Expected,
    /// A post-run check on the fixture (backfill / stale-row assertions).
    after: fn(&Path, &Path),
}

fn no_post_check(_src: &Path, _dst: &Path) {}

/// **cold, all stat-differ.** The headline: size alone decides all four pairs, so
/// nothing needs a digest and nothing is read.
///
/// Eager baseline: 8. This is the win.
fn cold_all_stat_differing(t: &TempDir) -> Case {
    let (src, dst) = four_differing(t);
    Case {
        name: "cold_all_stat_differing_pairs_read_nothing",
        src,
        dst,
        common: opts_mem(),
        trust: TrustOpts::default(),
        expected: Expected {
            records: Some(vec![
                json!({"event": "changed", "path": "a.txt"}),
                json!({"event": "changed", "path": "b.txt"}),
                json!({"event": "changed", "path": "sub/c.txt"}),
                json!({"event": "changed", "path": "sub/deep/d.txt"}),
                json!({"event": "summary", "missing": 0, "extra": 0, "changed": 4,
                       "type_conflict": 0, "case_mismatch": 0, "total_diff": 4}),
            ]),
            hashed: Some(0),
            ..Expected::default()
        },
        after: no_post_check,
    }
}

/// **cold, all stat-equal.** The other end of the same axis: mtimes pinned, so
/// stat agrees and the digest is the only thing that can decide. All eight files
/// are read — laziness is not "never hash".
fn cold_all_stat_equal(t: &TempDir) -> Case {
    let (src, dst) = pair(
        t,
        &[
            ("a.txt", Some(b"alpha"), Some(b"alpha")),
            ("b.txt", Some(b"bravo"), Some(b"bravo")),
            ("sub/c.txt", Some(b"charlie"), Some(b"charlie")),
            ("sub/deep/d.txt", Some(b"delta"), Some(b"delta")),
        ],
        &["md5"],
        false,
    );
    Case {
        name: "cold_all_stat_equal_pairs_are_all_hashed",
        src,
        dst,
        common: opts_mem(),
        trust: TrustOpts::default(),
        expected: Expected {
            records: Some(vec![
                json!({"event": "summary", "missing": 0, "extra": 0, "changed": 0,
                        "type_conflict": 0, "case_mismatch": 0, "total_diff": 0}),
            ]),
            exit: Some(0),
            src_hashed: Some(4),
            dst_hashed: Some(4),
            ..Expected::default()
        },
        after: no_post_check,
    }
}

/// **mixed.** Two pairs undecided, two decided by stat. Exactly the undecided
/// half is read — the count is 4, not 8 and not 0.
fn mixed_tree(t: &TempDir) -> Case {
    let (src, dst) = pair(
        t,
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
    Case {
        name: "mixed_tree_reads_only_the_stat_equal_half",
        src,
        dst,
        common: opts_mem(),
        trust: TrustOpts::default(),
        expected: Expected {
            records: Some(vec![
                json!({"event": "changed", "path": "diff1.txt"}),
                json!({"event": "changed", "path": "diff2.txt"}),
                json!({"event": "summary", "missing": 0, "extra": 0, "changed": 2,
                       "type_conflict": 0, "case_mismatch": 0, "total_diff": 2}),
            ]),
            src_hashed: Some(2),
            dst_hashed: Some(2),
            ..Expected::default()
        },
        after: no_post_check,
    }
}

/// **warm, complete.** Both caches already hold md5 for stat-matching rows, so
/// neither side opens a file.
fn warm_complete_caches(t: &TempDir) -> Case {
    let (src, dst) = pair_mem(
        t,
        &[
            ("a.txt", Some(b"alpha"), Some(b"alpha")),
            ("b.txt", Some(b"bravo"), Some(b"bravo")),
            ("sub/c.txt", Some(b"charlie"), Some(b"charlie")),
        ],
        &["md5"],
        true,
    );
    Case {
        name: "warm_complete_caches_read_nothing",
        src,
        dst,
        common: opts_mem(),
        trust: TrustOpts::default(),
        expected: Expected {
            exit: Some(0),
            hashed: Some(0),
            src_hashed: Some(0),
            dst_hashed: Some(0),
            ..Expected::default()
        },
        after: no_post_check,
    }
}

/// **stat-equal, one side missing an algo.** md5 is cached on both, sha256 on
/// neither — the shape a record written with `--hash-all-of md5` leaves behind.
/// Only the side that lacks the digest pays for it, and the pair still resolves.
///
/// The verdict stays clean because the content is identical: this case is about
/// the count, not about backfilling correctly.
fn stat_equal_missing_algo(t: &TempDir) -> Case {
    let both = &["md5", "sha256"];
    let (src, dst) = pair_mem(
        t,
        &[
            ("a.txt", Some(b"alpha"), Some(b"alpha")),
            ("b.txt", Some(b"bravo"), Some(b"bravo")),
        ],
        both,
        true,
    );
    strip_algo(&dst, "a.txt", "sha256");
    strip_algo(&dst, "b.txt", "sha256");
    Case {
        name: "stat_equal_pair_costs_only_the_side_missing_the_algorithm",
        src,
        dst,
        common: with_algos_mem(both),
        trust: TrustOpts::default(),
        expected: Expected {
            exit: Some(0),
            src_hashed: Some(0),
            dst_hashed: Some(2),
            ..Expected::default()
        },
        after: |_src, dst| {
            // And the gap is closed: dst's rows now hold both algorithms.
            assert_eq!(digests_of(dst, "a.txt"), vec!["md5", "sha256"]);
        },
    }
}

/// **stat-equal, src's digest poisoned.** Size and mtime match, so only the
/// content disagrees — and the cache says so, so no read is needed to find out.
///
/// This is the case that keeps laziness honest. A run that read nothing *and*
/// concluded "CHANGED" is only correct because the cached digests disagree; the
/// verdict came from the cache, not from stat.
fn poisoned_digest(t: &TempDir) -> Case {
    let (src, dst) = pair_mem(
        t,
        &[("a.txt", Some(b"alpha"), Some(b"alpha"))],
        &["md5"],
        true,
    );
    poison_digest(&src, "a.txt", "md5");
    Case {
        name: "stat_equal_pair_with_a_poisoned_digest_needs_no_read",
        src,
        dst,
        common: opts_mem(),
        trust: TrustOpts::default(),
        expected: Expected {
            records: Some(vec![
                json!({"event": "changed", "path": "a.txt"}),
                json!({"event": "summary", "missing": 0, "extra": 0, "changed": 1,
                       "type_conflict": 0, "case_mismatch": 0, "total_diff": 1}),
            ]),
            hashed: Some(0),
            ..Expected::default()
        },
        after: no_post_check,
    }
}

/// **`--hash-all-of none`.** No digest is required, so nothing is read, whatever the
/// tree looks like.
fn hash_none(t: &TempDir) -> Case {
    let (src, dst) = four_differing(t);
    Case {
        name: "hash_none_reads_nothing_on_a_differing_tree",
        src,
        dst,
        common: with_algos_mem(&[]),
        trust: TrustOpts::default(),
        expected: Expected {
            hashed: Some(0),
            changed: Some(4),
            ..Expected::default()
        },
        after: no_post_check,
    }
}

/// **no-trust, stat-equal.** Distrusting the cache must cost the read even
/// against a complete one — that is the flag's whole purpose. Four pairs, both
/// sides, eight files.
fn no_trust_stat_equal(t: &TempDir) -> Case {
    let (src, dst) = pair_mem(
        t,
        &[
            ("a.txt", Some(b"alpha"), Some(b"alpha")),
            ("b.txt", Some(b"bravo"), Some(b"bravo")),
            ("sub/c.txt", Some(b"charlie"), Some(b"charlie")),
            ("sub/deep/d.txt", Some(b"delta"), Some(b"delta")),
        ],
        &["md5"],
        true,
    );
    Case {
        name: "no_trust_on_stat_equal_pairs_forces_the_read",
        src,
        dst,
        common: opts_mem(),
        trust: TrustOpts {
            no_trust_src: true,
            no_trust_dst: true,
        },
        expected: Expected {
            exit: Some(0),
            src_hashed: Some(4),
            dst_hashed: Some(4),
            ..Expected::default()
        },
        after: no_post_check,
    }
}

/// **no-trust, stat-differing.** The documented no-op: distrusting the cache
/// does not make an unequal size uncertain. Nothing is read, despite the flag on
/// both sides and a cold cache.
fn no_trust_stat_differing(t: &TempDir) -> Case {
    let (src, dst) = four_differing(t);
    Case {
        name: "no_trust_on_stat_differing_pairs_is_a_no_op",
        src,
        dst,
        common: opts_mem(),
        trust: TrustOpts {
            no_trust_src: true,
            no_trust_dst: true,
        },
        expected: Expected {
            changed: Some(4),
            hashed: Some(0),
            ..Expected::default()
        },
        after: no_post_check,
    }
}

/// **src-only / dst-only, plus a dir.** A path on one side only is already
/// decided — `MISSING` or `EXTRA` — so it needs no digest on either side. The
/// dst-only dir likewise.
fn one_sided_paths_and_dirs(t: &TempDir) -> Case {
    let (src, dst) = pair(
        t,
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
    Case {
        name: "one_sided_paths_and_dirs_need_no_digest",
        src,
        dst,
        common: opts_mem(),
        trust: TrustOpts::default(),
        expected: Expected {
            records: Some(vec![
                json!({"event": "missing", "path": "only_src.txt"}),
                json!({"event": "missing", "path": "only_src_dir"}),
                json!({"event": "missing", "path": "only_src_dir/inner.txt"}),
                json!({"event": "extra", "path": "only_dst.txt"}),
                json!({"event": "summary", "missing": 3, "extra": 1, "changed": 0,
                       "type_conflict": 0, "case_mismatch": 0, "total_diff": 4}),
            ]),
            // Only `shared.txt` is undecided. Eager baseline: every file on both
            // sides, including the three that are one-sided (3 on src, 2 on dst).
            src_hashed: Some(1),
            dst_hashed: Some(1),
            ..Expected::default()
        },
        after: no_post_check,
    }
}

/// **kind conflict.** A file on one side, a directory on the other: decided by
/// presence, so no digest.
fn kind_conflict(t: &TempDir) -> Case {
    let (src, dst) = pair(
        t,
        &[("clash", Some(b"iamafile"), Some(b""))],
        &["md5"],
        false,
    );
    Case {
        name: "a_kind_conflict_needs_no_digest",
        src,
        dst,
        common: opts_mem(),
        trust: TrustOpts::default(),
        expected: Expected {
            records: Some(vec![
                json!({"event": "type-conflict", "path": "clash"}),
                json!({"event": "summary", "missing": 0, "extra": 0, "changed": 0,
                       "type_conflict": 1, "case_mismatch": 0, "total_diff": 1}),
            ]),
            hashed: Some(0),
            ..Expected::default()
        },
        after: no_post_check,
    }
}

/// **case-only difference, identical content.** In insensitive mode the pair is
/// matched by the folded key, so it is compared rather than reported as
/// missing+extra — and since its stat agrees, the digest decides, which means
/// both files are read. This is the case a planner that pairs by exact key gets
/// wrong: it would plan nothing and the `CASE-MISMATCH` would be the only line.
fn case_equal(t: &TempDir) -> Case {
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    wfile(&src, "a.txt", b"alpha");
    wfile(&dst, "A.txt", b"alpha");
    sync_mtime(&src.join("a.txt"), &dst.join("A.txt"));

    let mut insensitive = opts_mem();
    insensitive.set_case_sensitive(false);
    Case {
        name: "a_case_only_difference_with_equal_content_is_still_compared",
        src,
        dst,
        common: insensitive,
        trust: TrustOpts::default(),
        expected: Expected {
            records: Some(vec![
                json!({"event": "case-mismatch", "src": "a.txt", "dst": "A.txt"}),
                json!({"event": "summary", "missing": 0, "extra": 0, "changed": 0,
                       "type_conflict": 0, "case_mismatch": 1, "total_diff": 0}),
            ]),
            src_hashed: Some(1),
            dst_hashed: Some(1),
            ..Expected::default()
        },
        after: no_post_check,
    }
}

/// **case-only difference, differing content, stat equal.** Same pairing, and now
/// the digest has something to report, so the pair lands in `CHANGED` *as well
/// as* `CASE-MISMATCH`. Both come from the same `diff_maps` pass.
///
/// The content is the same *length* in different cases, which is the whole point:
/// size matches, mtime is pinned, so stat cannot decide and the digest must.
fn case_different(t: &TempDir) -> Case {
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    wfile(&src, "a.txt", b"alpha");
    wfile(&dst, "A.txt", b"ALPHA");
    sync_mtime(&src.join("a.txt"), &dst.join("A.txt"));

    let mut insensitive = opts_mem();
    insensitive.set_case_sensitive(false);
    Case {
        name: "a_case_only_difference_with_differing_content_is_changed_too",
        src,
        dst,
        common: insensitive,
        trust: TrustOpts::default(),
        expected: Expected {
            records: Some(vec![
                json!({"event": "changed", "path": "a.txt"}),
                json!({"event": "case-mismatch", "src": "a.txt", "dst": "A.txt"}),
                json!({"event": "summary", "missing": 0, "extra": 0, "changed": 1,
                       "type_conflict": 0, "case_mismatch": 1, "total_diff": 1}),
            ]),
            src_hashed: Some(1),
            dst_hashed: Some(1),
            ..Expected::default()
        },
        after: no_post_check,
    }
}

/// A stat-differing pair whose stale row held digests must be decided by stat,
/// not by those digests. The row is poisoned *and* the file aged, so a planner
/// that carried a stale row would report `CHANGED` for the wrong reason — and a
/// planner that carried nothing would report it for the right one. Same verdict,
/// so the case is really about the cache.
fn stale_digest_dropped(t: &TempDir) -> Case {
    let (src, dst) = pair_mem(
        t,
        &[("a.txt", Some(b"alpha"), Some(b"alpha"))],
        &["md5"],
        true,
    );
    // src keeps its real content and gets a new mtime; dst keeps the old one.
    age(&src, "a.txt", 120);
    Case {
        name: "a_stat_differing_pair_is_decided_by_stat_and_its_stale_digests_are_dropped",
        src,
        dst,
        common: opts_mem(),
        trust: TrustOpts::default(),
        expected: Expected {
            records: Some(vec![
                json!({"event": "changed", "path": "a.txt"}),
                json!({"event": "summary", "missing": 0, "extra": 0, "changed": 1,
                       "type_conflict": 0, "case_mismatch": 0, "total_diff": 1}),
            ]),
            hashed: Some(0),
            ..Expected::default()
        },
        after: |src, _dst| {
            // The real point: the stale digest is *dropped*, not merely unread. A
            // row left holding a pre-change digest would survive a run that never
            // hashes this file, and phase A's carry rule is what prevents that.
            assert!(
                digests_of(src, "a.txt").is_empty(),
                "the stale digest was replaced by a stat-only row"
            );
        },
    }
}

/// One driving test over the laziness table. Every row's former standalone test
/// ran these same assertions through the same `run_pair` seam; the fixture is now
/// built by the row and the verdict/count/exit assertions are applied by this loop.
#[test]
fn compare_laziness_table() {
    let builders: &[fn(&TempDir) -> Case] = &[
        cold_all_stat_differing,
        cold_all_stat_equal,
        mixed_tree,
        warm_complete_caches,
        stat_equal_missing_algo,
        poisoned_digest,
        hash_none,
        no_trust_stat_equal,
        no_trust_stat_differing,
        one_sided_paths_and_dirs,
        kind_conflict,
        case_equal,
        case_different,
        stale_digest_dropped,
    ];
    for build in builders {
        let t = temp_root("lz_table");
        let case = build(&t);
        let r = run_pair(&case.src, &case.dst, &case.common, case.trust);

        if let Some(exp) = case.expected.records {
            assert_eq!(r.records(), exp, "{}: verdict records", case.name);
        }
        if let Some(exp) = case.expected.exit {
            assert_eq!(r.exit(), exp, "{}: exit code", case.name);
        }
        if let Some(exp) = case.expected.hashed {
            assert_eq!(r.hashed, exp, "{}: total hashed", case.name);
        }
        if let Some(exp) = case.expected.src_hashed {
            assert_eq!(r.src_hashed, exp, "{}: src hashed", case.name);
        }
        if let Some(exp) = case.expected.dst_hashed {
            assert_eq!(r.dst_hashed, exp, "{}: dst hashed", case.name);
        }
        if let Some(exp) = case.expected.changed {
            assert_eq!(r.diff.changed.len(), exp, "{}: changed count", case.name);
        }
        (case.after)(&case.src, &case.dst);
    }
}

/// The wiring, not the counters: `cmd_compare` reaches the same verdict through
/// the phases. Guards the case where the engine is right and the command does
/// not call it. Kept as its own test because it drives `cmd_compare` directly,
/// not the `compare_detailed` pipeline `run_pair` uses — a distinct mechanism.
#[test]
fn cmd_compare_reports_the_same_verdict_and_its_exit_code() {
    let t = temp_root("lz_cmd");
    let (src, dst) = four_differing(&t);
    let o = girsync::CompareOpts {
        src: src.clone(),
        dst: dst.clone(),
        trust: TrustOpts::default(),
        common: opts_mem(),
    };
    assert_eq!(girsync::cmd_compare(o, &log()).unwrap(), 4);

    // A MISSING and an EXTRA, so the buckets are not all `changed`.
    wfile(&src, "brand_new.txt", b"new");
    let o = girsync::CompareOpts {
        src: src.clone(),
        dst: dst.clone(),
        trust: TrustOpts::default(),
        common: opts_mem(),
    };
    assert_eq!(girsync::cmd_compare(o, &log()).unwrap(), 4);
}
