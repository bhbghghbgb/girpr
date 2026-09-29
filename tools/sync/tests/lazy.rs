//! Laziness: what a comparison reads, and the oracle that says it read the
//! right thing.
//!
//! Two independent things are tested here, and both matter:
//!
//! - **The counts.** A planner that reads nothing gets the right answer by
//!   reading nothing at all. The counters are the only thing that distinguishes a
//!   run which read what it had to from a run that got lucky.
//! - **The oracle.** Counts alone cannot prove correctness — a planner that never
//!   hashed anything would report `hashed = 0` everywhere. The oracle is the only
//!   check that laziness changed the *cost* and not the *verdict*.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::{
    BASE_MTIME_NS, Entry, Node, Rng, SHAPES, Shape, TempRoot, log, make_entry, materialize_pair,
    opts, shape_rel, tree_pair, update,
};
use girsync::config::{CommonOpts, ScanMode, TrustOpts};
use girsync::effective::{PairResult, classify, compare_pair, open_side};
use girsync::{UpdateOpts, cmd_update};

/// Compare two sides in process and return the diff plus both sides' counters.
fn compare_sides(src: &Path, dst: &Path, common: &CommonOpts) -> PairResult {
    compare_sides_trust(src, dst, common, TrustOpts::default())
}

fn compare_sides_trust(
    src: &Path,
    dst: &Path,
    common: &CommonOpts,
    trust: TrustOpts,
) -> PairResult {
    let s = open_side(
        &classify(src),
        common,
        ScanMode {
            no_trust_cached_hashes: trust.no_trust_src,
            dry_run: false,
        },
    )
    .unwrap();
    let d = open_side(
        &classify(dst),
        common,
        ScanMode {
            no_trust_cached_hashes: trust.no_trust_dst,
            dry_run: false,
        },
    )
    .unwrap();
    compare_pair(&s, &d, common).unwrap()
}

/// Two folders built from `entries`, on a cold cache.
fn cold_pair(tag: &str, entries: &[Entry]) -> (TempRoot, PathBuf, PathBuf) {
    let t = TempRoot::new(tag);
    let src = t.mkdirs("src");
    let dst = t.mkdirs("dst");
    materialize_pair(&src, &dst, entries);
    (t, src, dst)
}

/// An entry present with identical content and stat on both sides.
fn same(rel: &str, bytes: &[u8], mtime: i64) -> Entry {
    Entry {
        rel: rel.to_string(),
        dst_rel: None,
        src: Node::File(bytes.to_vec()),
        dst: Node::File(bytes.to_vec()),
        src_mtime: mtime,
        dst_mtime: mtime,
    }
}

/// `same`, with dst's content replaced by `other` — which **must be the same
/// length**, or the pair is decided by size and the test measures the wrong
/// thing. Asserted here so a typo cannot quietly turn an undecided case into a
/// decided one.
fn same_len_diff(rel: &str, bytes: &[u8], other: &[u8], mtime: i64) -> Entry {
    assert_eq!(
        bytes.len(),
        other.len(),
        "{rel}: same-length fixtures must differ only in content"
    );
    Entry {
        dst: Node::File(other.to_vec()),
        ..same(rel, bytes, mtime)
    }
}

/// The `hashed` counter for both sides, in one tuple so every assertion reads
/// the same way.
fn reads(p: &PairResult) -> (usize, usize) {
    (p.src.hashed, p.dst.hashed)
}

// ---------------------------------------------------------------------------
// The counts
// ---------------------------------------------------------------------------

/// The headline win. Three of the four pairs differ in stat, so size or mtime
/// alone decides them and `diff_maps` would never reach `hashes_differ` — but a
/// per-side plan has to read both sides of each to discover that. A pair-aware
/// plan does not, which is the entire point of W2.
#[test]
fn a_stat_differing_pair_costs_no_read() {
    let m = BASE_MTIME_NS;
    let entries = vec![
        // The one shape that genuinely needs a digest.
        same("same.txt", b"aaaa", m),
        {
            // Different size -> decided by size.
            let mut e = same("bigger.txt", b"bbbb", m);
            e.dst = Node::File(b"bbbbbbbb".to_vec());
            e
        },
        {
            // Same size, different mtime -> decided by mtime.
            let mut e = same("newer.txt", b"cccc", m);
            e.dst_mtime = m + 1_000_000_000;
            e
        },
        {
            // src-only.
            let mut e = same("onlysrc.txt", b"dddd", m);
            e.dst = Node::Absent;
            e
        },
        {
            // dst-only.
            let mut e = same("onlydst.txt", b"eeee", m);
            e.src = Node::Absent;
            e
        },
    ];
    let (_t, src, dst) = cold_pair("lazy_sizediff", &entries);
    let r = compare_sides(&src, &dst, &opts());
    assert_eq!(
        reads(&r),
        (1, 1),
        "only the one stat-equal pair is read, on each side"
    );
    // Everything else on the src side is settled with nothing to read: the
    // size-differing file, the mtime-differing one, and the src-only path.
    assert_eq!(r.src.stat_only, 3);
    assert_eq!(r.dst.stat_only, 3);
    assert_eq!((r.diff.missing.len(), r.diff.extra.len()), (1, 1));
    assert_eq!(
        r.diff.changed.len(),
        2,
        "the size- and mtime-differing pairs"
    );
}

/// The other end of the same table: when every pair matches on size *and* mtime,
/// a digest is the only question left, so everything is read.
#[test]
fn an_all_undecided_pair_reads_everything() {
    let m = BASE_MTIME_NS;
    let mut entries = Vec::new();
    for i in 0..5 {
        entries.push(same_len_diff(
            &format!("f{i}.txt"),
            b"identical-content",
            b"IDENTICAL-CONTENT",
            m,
        ));
    }
    let (_t, src, dst) = cold_pair("lazy_allundecided", &entries);
    let r = compare_sides(&src, &dst, &opts());
    assert_eq!(reads(&r), (5, 5));
}

/// Half stat-equal, half not: exactly the stat-equal half is read, and the
/// verdict still counts all of them.
#[test]
fn only_the_stat_equal_half_is_read() {
    let m = BASE_MTIME_NS;
    let mut entries = Vec::new();
    for i in 0..4 {
        let e = if i % 2 == 0 {
            same_len_diff(
                &format!("f{i}.txt"),
                b"same-length-content",
                b"SAME-LENGTH-CONTENT",
                m,
            )
        } else {
            let mut e = same(&format!("f{i}.txt"), b"same-length-content", m);
            e.dst_mtime = m + 1_000_000_000;
            e
        };
        entries.push(e);
    }
    let (_t, src, dst) = cold_pair("lazy_half", &entries);
    let r = compare_sides(&src, &dst, &opts());
    assert_eq!(reads(&r), (2, 2), "the two even paths");
    assert_eq!(r.diff.changed.len(), 4, "all four differ, two by digest");
}

/// A warm cache holding every requested digest costs no read — and the verdict
/// still comes from the digest, not from the stat. This is the tier that makes
/// the cache worth consulting at all.
#[test]
fn a_complete_warm_cache_costs_no_read() {
    let m = BASE_MTIME_NS;
    let mut entries = Vec::new();
    for i in 0..3 {
        entries.push(same_len_diff(
            &format!("f{i}.txt"),
            b"warm-content-here",
            b"WARM-CONTENT-HERE",
            m,
        ));
    }
    let (t, src, dst) = cold_pair("lazy_warm", &entries);
    cmd_update(update(src.clone()), &log()).unwrap();
    cmd_update(update(dst.clone()), &log()).unwrap();

    let r = compare_sides(&src, &dst, &opts());
    assert_eq!(
        reads(&r),
        (0, 0),
        "the cache already holds md5 for every undecided pair"
    );
    assert_eq!((r.src.cache_hit, r.dst.cache_hit), (3, 3));
    assert_eq!(r.diff.changed.len(), 3, "still CHANGED, decided by digest");
    drop(t);
}

/// One side missing an algorithm reads only that side, and the other still
/// answers — the backfill path.
#[test]
fn a_side_missing_the_algo_reads_only_itself() {
    let m = BASE_MTIME_NS;
    let e = same_len_diff("a.txt", b"dual-algo-content", b"DUAL-ALGO-CONTENT", m);
    let (t, src, dst) = cold_pair("lazy_missing_algo", &[e]);
    // Warm one side with md5 only; then ask both sides for md5 and sha256.
    cmd_update(
        UpdateOpts {
            dir: src.clone(),
            common: CommonOpts {
                algos: vec!["md5".to_string()],
                ..opts()
            },
        },
        &log(),
    )
    .unwrap();
    let both = CommonOpts {
        algos: vec!["md5".to_string(), "sha256".to_string()],
        ..opts()
    };
    let r = compare_sides(&src, &dst, &both);
    assert_eq!(r.src.hashed, 1, "src backfills sha256");
    assert_eq!(r.dst.hashed, 1, "dst has neither");
    assert_eq!(r.diff.changed.len(), 1);
    drop(t);
}

/// `--hash none` is a distinct mode: the plan is empty, so nothing is read. It
/// must not be "all-of over an empty set that happens to pass", which would still
/// walk and stat the whole tree for no reason.
#[test]
fn hash_none_reads_nothing_on_any_tree() {
    let mut rng = Rng::new(7);
    let entries = tree_pair(&mut rng, 3);
    let (_t, src, dst) = cold_pair("lazy_hashnone", &entries);
    let r = compare_sides(
        &src,
        &dst,
        &CommonOpts {
            algos: vec![],
            ..opts()
        },
    );
    assert_eq!(reads(&r), (0, 0));
    assert!(r.src.stat_only > 0, "files were seen, just never read");
}

/// A stat-*differing* pair is a no-op for `--no-trust-cached-hashes`, and that is
/// correct rather than a bug: distrusting the cache does not make an unequal
/// size uncertain. Pinned because it reads like a missed read.
#[test]
fn no_trust_is_a_documented_no_op_for_stat_differing_pairs() {
    let m = BASE_MTIME_NS;
    let mut e = same("bigger.txt", b"short", m);
    e.dst = Node::File(b"much-longer-dst-content".to_vec());
    let (_t, src, dst) = cold_pair("lazy_notrust_statdiff", &[e]);
    let r = compare_sides_trust(
        &src,
        &dst,
        &opts(),
        TrustOpts {
            no_trust_src: true,
            no_trust_dst: true,
        },
    );
    assert_eq!(
        reads(&r),
        (0, 0),
        "size already decided it; no read either way"
    );
    assert_eq!(r.diff.changed.len(), 1);
}

/// The mirror image: on a stat-*equal* pair, distrusting a complete cache forces
/// the read the flag exists for — and changes nothing about the verdict.
#[test]
fn no_trust_reads_a_stat_equal_pair_despite_a_full_cache() {
    let m = BASE_MTIME_NS;
    let e = same_len_diff("a.txt", b"trust-me-not-content", b"TRUST-ME-NOT-CONTENT", m);
    let (t, src, dst) = cold_pair("lazy_notrust_stategal", &[e]);
    cmd_update(update(src.clone()), &log()).unwrap();
    cmd_update(update(dst.clone()), &log()).unwrap();

    let trusting = compare_sides(&src, &dst, &opts());
    assert_eq!(reads(&trusting), (0, 0));
    let same_verdict = trusting.diff.changed.len();

    let distrusting = compare_sides_trust(
        &src,
        &dst,
        &opts(),
        TrustOpts {
            no_trust_src: true,
            no_trust_dst: true,
        },
    );
    assert_eq!(
        reads(&distrusting),
        (1, 1),
        "the cache is complete and is not trusted"
    );
    assert_eq!(
        distrusting.diff.changed.len(),
        same_verdict,
        "and the verdict is unchanged"
    );
    drop(t);
}

/// A record side can never produce a digest, so a stat-differing pair against one
/// must also cost no read. This is the defect W2 §1a describes: before W2 the
/// folder side paid full read cost and the verdict fell back to size+mtime
/// anyway.
#[test]
fn a_record_side_makes_a_stat_differing_pair_free_too() {
    let m = BASE_MTIME_NS;
    let mut e = same("bigger.txt", b"short", m);
    e.dst = Node::File(b"much-longer-content-here".to_vec());
    let (t, src, dst) = cold_pair("lazy_record", &[e]);
    cmd_update(update(src.clone()), &log()).unwrap();
    cmd_update(update(dst.clone()), &log()).unwrap();

    let record = dst.join(girsync::cache::CACHE_PREFIX);
    let r = compare_sides(&src, &record, &opts());
    assert_eq!(
        r.src.hashed, 0,
        "size differs, so the folder side must not read it either"
    );
    assert_eq!(r.dst.hashed, 0, "a record can never read");
    assert_eq!(r.diff.changed.len(), 1);
    drop(t);
}

/// A case-only difference is still undecided, and must still be paid for: two
/// files whose stat agrees but whose casing differs are one path, and only a
/// digest settles whether their content matches.
#[test]
fn a_case_only_difference_is_still_undecided() {
    let mut rng = Rng::new(3);
    let e = make_entry(Shape::CaseOnly, "f0.dat", &mut rng);
    let (_t, src, dst) = cold_pair("lazy_caseonly", &[e]);
    let r = compare_sides(
        &src,
        &dst,
        &CommonOpts {
            case_sensitive: false,
            ..opts()
        },
    );
    assert_eq!(
        reads(&r),
        (1, 1),
        "the digest is the only thing that can decide this"
    );
    assert_eq!(r.diff.case_mismatch.len(), 1);
    assert_eq!(
        r.diff.missing.len(),
        0,
        "not a MISSING when matched by case"
    );
}

// ---------------------------------------------------------------------------
// The oracle
// ---------------------------------------------------------------------------

/// The oracle: for a fixed tree, a lazy comparison and a distrust-everything
/// comparison must produce identical output.
///
/// The distrust run is the eager reference. It is not a golden file, so nothing
/// has to be updated when a verdict legitimately changes, and it needs no
/// reference implementation: "read every digest" *is* the answer W2 is trying to
/// compute lazily.
///
/// Two pristine roots per shape, not one. A comparison writes cache rows, so
/// running the lazy variant first would change the state the distrust variant
/// then reads — comparing against a polluted starting point would pass for the
/// wrong reasons.
///
/// Run through the binary, so what gets compared is the real stdout and exit
/// code rather than an in-process stand-in for them.
fn oracle(seed: u64, case_sensitive: bool, extra: usize) {
    let bin = env!("CARGO_BIN_EXE_girsync");
    let run = |trust: bool| -> (String, i32) {
        let t = TempRoot::new("oracle");
        let src = t.mkdirs("src");
        let dst = t.mkdirs("dst");
        // Same seed both times, so the two variants start byte-identical.
        let mut rng = Rng::new(seed);
        let entries = tree_pair(&mut rng, extra);
        materialize_pair(&src, &dst, &entries);
        // Warm both caches, so the lazy run genuinely consults the cache instead
        // of finding nothing to consult. Without this the oracle is vacuous: on a
        // cold cache there is nothing to trust and nothing to distrust.
        for dir in [&src, &dst] {
            let out = Command::new(bin)
                .arg("update")
                .arg("--dir")
                .arg(dir)
                .output()
                .unwrap();
            assert!(out.status.success(), "update failed: {out:?}");
        }
        let mut cmd = Command::new(bin);
        cmd.arg("compare")
            .arg("--src")
            .arg(&src)
            .arg("--dst")
            .arg(&dst);
        if case_sensitive {
            cmd.arg("--case-sensitive");
        }
        if trust {
            cmd.arg("--no-trust-cached-hashes")
                .arg("src")
                .arg("--no-trust-cached-hashes")
                .arg("dst");
        }
        let out = cmd.output().unwrap();
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            out.status.code().unwrap_or(-1),
        )
    };

    let (lazy_out, lazy_code) = run(false);
    let (eager_out, eager_code) = run(true);
    let ctx = format!("seed={seed} case_sensitive={case_sensitive} extra={extra}");
    assert_eq!(lazy_code, eager_code, "exit code differs ({ctx})");
    assert_eq!(lazy_out, eager_out, "diff output differs ({ctx})");
    assert!(!lazy_out.is_empty(), "printed nothing ({ctx})");
}

#[test]
fn oracle_holds_over_seeded_trees() {
    for seed in 1..=6u64 {
        for case_sensitive in [false, true] {
            oracle(seed, case_sensitive, 2);
        }
    }
}

#[test]
fn oracle_holds_on_a_large_generated_tree() {
    for seed in 100..=103u64 {
        oracle(seed, false, 40);
    }
}

/// The same oracle over hand-built fixtures, one per shape, so a failure names
/// *which* state broke rather than a seed.
#[test]
fn oracle_holds_for_every_single_shape() {
    let render = |p: &PairResult| {
        format!(
            "M{:?} E{:?} C{:?} T{:?} cm{:?} n{}",
            p.diff.missing,
            p.diff.extra,
            p.diff.changed,
            p.diff.type_conflict,
            p.diff.case_mismatch,
            p.diff.total()
        )
    };
    for (i, shape) in SHAPES.iter().enumerate() {
        let mut rng = Rng::new(0x5EED + i as u64);
        let entry = make_entry(*shape, "probe.dat", &mut rng);
        // A read is required exactly when the pair is stat-equal *and* the two
        // sides spell the path the same way. Deriving the expectation from the
        // fixture rather than listing the shapes keeps this honest: `Identical`
        // and `Nested` need a read just as much as `SameStatDiffContent` does,
        // because stat agreeing is not the same as content agreeing.
        let expect_read = entry.is_undecided() && entry.pairs_sensitively();
        let (_t, src, dst) = cold_pair("oracle_shape", &[entry]);
        let lazy = compare_sides(&src, &dst, &opts());
        let eager = compare_sides_trust(
            &src,
            &dst,
            &opts(),
            TrustOpts {
                no_trust_src: true,
                no_trust_dst: true,
            },
        );
        assert_eq!(
            render(&lazy),
            render(&eager),
            "shape {shape:?}: verdict differs between lazy and eager"
        );
        // A read is required exactly when the pair is stat-equal *and* the two
        // sides spell the path the same way. Deriving the expectation from the
        // fixture rather than listing the shapes keeps this honest: `Identical`
        // and `Nested` need a read just as much as `SameStatDiffContent` does,
        // because stat agreeing is not the same as content agreeing.
        if expect_read {
            assert_eq!(reads(&lazy), (1, 1), "{shape:?} must be read");
        } else {
            assert_eq!(reads(&lazy), (0, 0), "{shape:?} must not need a digest");
        }
    }
}

/// Guard the guard: the oracle only means something if the fixture actually
/// contains the states the diff has to tell apart, and at least one path that
/// laziness must still read.
#[test]
fn the_oracle_fixture_covers_every_shape() {
    let mut rng = Rng::new(1);
    let entries = tree_pair(&mut rng, 2);
    assert_eq!(entries.len(), SHAPES.len() + 2);
    assert!(
        entries.iter().filter(|e| e.is_undecided()).count() >= 2,
        "at least the Identical and SameStatDiffContent cases are undecided"
    );
    for (i, shape) in SHAPES.iter().enumerate() {
        let want = shape_rel(i, *shape);
        let found = entries
            .iter()
            .any(|e| e.src_rel() == want || e.dst_rel() == want);
        assert!(found, "shape {shape:?} missing at {want}: {entries:?}");
    }
}
