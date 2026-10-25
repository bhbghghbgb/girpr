//! What `--no-trust-size` is actually for: repairing a stuck cache row.
//!
//! A size difference settles a pair for free, so a size-changed row — whose digests
//! were all dropped as suspect — stays stat-only forever and can never become
//! comparable again. This is invisible in both the counters and the verdicts.

use super::*;

/// **The cache repair, which is the reason `no-trust-size` exists and the reason no
/// counter or verdict would ever reveal it.**
///
/// A row whose size no longer matches disk arrives in phase A with its digests
/// *dropped* — a stale row's are all suspect, that is the carry rule — and it is
/// stat-only from then on. Under the short circuit nothing ever asks for those digests
/// again, because a size-changed pair is always settled for free. So the row is stuck:
/// permanently stat-only, and the content it describes is unverifiable.
///
/// `--no-trust-size` puts it back in the undecided set, so the digest is recomputed and
/// stored.
///
/// Asserted on the **row**, which is the whole point: both runs report the same verdict
/// and the same `CHANGED`, and only one of them repairs the cache.
#[test]
fn no_trust_size_rehashes_a_stale_row_that_the_short_circuit_would_leave_stat_only() {
    // dst starts as a copy of src, so the row is fresh and both sides have a digest.
    let t = temp_root("st_repair");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    wfile(&src, "a.txt", b"original");
    wfile(&dst, "a.txt", b"original");
    sync_mtime(&src.join("a.txt"), &dst.join("a.txt"));
    cmd_update(update_mem(src.clone()), &log()).unwrap();
    cmd_update(update_mem(dst.clone()), &log()).unwrap();
    assert!(
        !recs_of(&dst)["a.txt"].hashes.is_empty(),
        "the row starts warm, so the repair has something to repair"
    );

    // dst's content changes length. Phase A drops dst's digest for being stale, and
    // the pair is now settled by size for free — so nothing rehashes it.
    //
    // The mtime is **re-pinned after the rewrite**, which is load-bearing: a rewrite
    // moves the mtime too, and a pair whose mtime *also* differs is still settled by
    // the field we are trusting, so `without_size()` alone would widen nothing and the
    // fixture would appear not to work. That is correct behaviour — the flag only acts
    // when the distrusted field is the *only* difference — and
    // `distrusting_mtime_leaves_a_size_difference_settling_the_pair` states the
    // converse in the planner's tests.
    wfile(&dst, "a.txt", b"original but longer now");
    sync_mtime(&src.join("a.txt"), &dst.join("a.txt"));
    let (d_src, d_dst) = (src.clone(), dst.clone());
    cmd_compare(
        girsync::CompareOpts {
            src: d_src,
            dst: d_dst,
            trust: Default::default(),
            common: distrusting(&["md5"], StatTrust::default()),
        },
        &log(),
    )
    .unwrap();
    assert!(
        recs_of(&dst)["a.txt"].hashes.is_empty(),
        "under the short circuit the row is left stat-only, and stays that way"
    );

    // Now the same edit, with the flag: the digest comes back.
    let t2 = temp_root("st_repair2");
    let src2 = t2.subdir("src");
    let dst2 = t2.subdir("dst");
    wfile(&src2, "a.txt", b"original");
    wfile(&dst2, "a.txt", b"original");
    sync_mtime(&src2.join("a.txt"), &dst2.join("a.txt"));
    cmd_update(update_mem(src2.clone()), &log()).unwrap();
    cmd_update(update_mem(dst2.clone()), &log()).unwrap();
    wfile(&dst2, "a.txt", b"original but longer now");
    // Re-pinned for the same reason as the first half: size must be the only
    // difference, or the trusted mtime settles the pair and the flag does nothing.
    sync_mtime(&src2.join("a.txt"), &dst2.join("a.txt"));

    cmd_compare(
        girsync::CompareOpts {
            src: src2,
            dst: dst2.clone(),
            trust: Default::default(),
            common: distrusting(&["md5"], StatTrust::default().without_size()),
        },
        &log(),
    )
    .unwrap();
    let repaired = &recs_of(&dst2)["a.txt"];
    assert!(
        !repaired.hashes.is_empty(),
        "with --no-trust-size the digest is recomputed and stored: {repaired:?}"
    );
    assert_eq!(
        repaired.size, 23,
        "and the row describes the file as it is now, not as it was"
    );
}
