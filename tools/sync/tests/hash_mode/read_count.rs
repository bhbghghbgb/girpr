//! `any-of` changes the read count, which is the whole reason it exists.
//!
//! Tier 1 measured rather than read off the plan: two folders warmed with md5 only,
//! asked for both, and the count is what the tier is worth.

use super::*;

/// **Tier 1 measured, not read off the plan.** Two folders warmed with md5 only,
/// asked for md5 *and* sha256: `all-of` backfills sha256 on both sides, `any-of`
/// settles every pair with the md5 both sides already hold and reads nothing.
///
/// A separate fixture per mode, for the reason [`run_pair`] gives. Measuring both on
/// one pair would report `any-of` as reading nothing for the wrong reason — the
/// `all-of` run had just backfilled sha256 into both caches.
///
/// The `all-of` half is the control and is deliberately first: a test asserting only
/// "`any-of` reads 0" would also pass against a planner that never reads anything.
#[test]
fn any_of_avoids_backfilling_an_algorithm_the_pair_already_shares() {
    // `pair`'s last argument is the set each cache is warmed with, so md5 on both
    // sides and nothing else — the tier-1 shape.
    let spec: &[common::Spec] = &[
        ("a.txt", Some(b"alpha"), Some(b"alpha")),
        ("b.txt", Some(b"bravo"), Some(b"bravo")),
    ];
    let t_all = TempRoot::new("hm_tier1_all");
    let t_any = TempRoot::new("hm_tier1_any");
    let (a_src, a_dst) = pair(&t_all, spec, &["md5"], true);
    let (b_src, b_dst) = pair(&t_any, spec, &["md5"], true);

    assert_eq!(
        run_pair(&a_src, &a_dst, &asking(&["md5", "sha256"], HashMode::AllOf)),
        (4, 4),
        "all-of backfills sha256: 2 files x 2 sides, one digest each"
    );
    assert_eq!(
        run_pair(&b_src, &b_dst, &asking(&["md5", "sha256"], HashMode::AnyOf)),
        (0, 0),
        "any-of settles every pair with the md5 both sides already hold"
    );
}

/// And when the pair shares **nothing**, `any-of` is not free — it settles by
/// hashing, on both sides, exactly once. This is the counterweight to the tier-1
/// case: `any-of` narrows the *request*, it does not skip the work.
#[test]
fn any_of_still_hashes_when_the_pair_shares_no_algorithm() {
    // Cold on both sides: `pair`'s last argument is the set each cache is warmed
    // with, so an empty set leaves stat-only rows. The pair is stat-equal and
    // nothing is available anywhere, which is the only shape where every tier costs
    // a read.
    //
    // **A fresh fixture per measurement**, for the reason `run_pair` gives. Measuring
    // both modes against one fixture would make the `any-of` number come out 0 —
    // not because the mode is free, but because the `all-of` run before it had
    // already backfilled both algorithms into both caches, so the second run found
    // tier 1.
    let t_all = TempRoot::new("hm_tier3_all");
    let t_any = TempRoot::new("hm_tier3_any");
    let spec: &[common::Spec] = &[("a.txt", Some(b"alpha"), Some(b"alpha"))];
    let (a_src, a_dst) = pair(&t_all, spec, &[], true);
    let (b_src, b_dst) = pair(&t_any, spec, &[], true);

    assert_eq!(
        run_pair(&a_src, &a_dst, &asking(&["md5", "sha256"], HashMode::AllOf)),
        (2, 4),
        "all-of: one file opened per side, both algorithms computed in that pass"
    );
    assert_eq!(
        run_pair(&b_src, &b_dst, &asking(&["md5", "sha256"], HashMode::AnyOf)),
        (2, 2),
        "any-of reads the same file - it narrows the request, it does not skip the work -
         and the difference is the second digest"
    );
}

/// One record, two algorithms requested, one of them present. `any-of` reads the
/// one it picked and leaves the other alone.
///
/// This is the case the whole mode exists for in practice: a record written by an
/// older run, where the tree is too large to repopulate just to make a comparison
/// possible.
#[test]
fn any_of_reads_one_algorithm_against_a_record_that_holds_the_other() {
    let t = TempRoot::new("hm_audit_any_of");
    let rec_home = t.mkdirs("rec");
    let folder = t.mkdirs("live");
    for dir in [&rec_home, &folder] {
        wfile(dir, "a.txt", b"hello");
    }
    sync_mtime(&rec_home.join("a.txt"), &folder.join("a.txt"));
    cmd_update(
        girsync::UpdateOpts {
            dir: rec_home.clone(),
            common: asking(&["md5"], HashMode::AllOf),
        },
        &log(),
    )
    .unwrap();
    cmd_update(
        girsync::UpdateOpts {
            dir: folder.clone(),
            common: asking(&["md5", "sha256"], HashMode::AllOf),
        },
        &log(),
    )
    .unwrap();

    let (hashed, code) = self_audit_count(&folder, &asking(&["md5", "sha256"], HashMode::AnyOf));
    assert_eq!(code, 0, "the audit answered, so it did not degrade to stat");
    assert_eq!(
        hashed, 1,
        "one file, one algorithm: the record's md5 settled it, and sha256 was \
         never computed"
    );
}
