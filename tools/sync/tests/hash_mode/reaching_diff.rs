//! The pick reaching the diff: a per-path answer, not a run-wide list.
//!
//! The sharpest claim the feature makes. Under `any-of` the requested list and the
//! list the diff consults are different, so a diff handed `algos` would compare
//! digests the planner never settled the pair on.

use super::*;

/// **The sharpest test in the file: the diff must consult the pick, per path.**
///
/// Both files hold md5 *and* sha256, md5 agrees on both and sha256 disagrees on
/// both. So "does this run report drift?" has one answer per path and one per mode,
/// and no part of it depends on a digest being absent — which is deliberate, because
/// a fixture that relied on absence would only prove the diff skipped a missing
/// digest, not that it read the right entry.
///
/// Under all-of both files are reported. Under any-of only `two.txt` is, because
/// only it is settled by sha256.
///
/// Stated as a verdict rather than as a plan, because a test that only checked
/// `required` would pass with a diff still ignoring it — and the all-of half is
/// what catches that: it fails if the per-path lookup is dropped.
#[test]
fn the_diff_compares_by_the_algorithms_the_plan_chose() {
    let both = |md5: &[u8], sha: &[u8]| digests(&[("md5", md5), ("sha256", sha)]);
    let src = eff(vec![
        ("one.txt", &both(b"agree", b"differs-1")),
        ("two.txt", &both(b"agree", b"differs-2")),
    ]);
    let dst = eff(vec![
        ("one.txt", &both(b"agree", b"one-side-1")),
        ("two.txt", &both(b"agree", b"one-side-2")),
    ]);
    // The answer `plan_pairs` would give for this shape under any-of: `one.txt`
    // settled by md5 (free on both sides), `two.txt` by sha256.
    let by_path = Required {
        by_rel: [
            ("one.txt".to_string(), AlgoSet::one(HashAlg::Md5)),
            ("two.txt".to_string(), AlgoSet::one(HashAlg::Sha256)),
        ]
        .into_iter()
        .collect(),
        fallback: AlgoSet::from_iter([HashAlg::Md5, HashAlg::Sha256]),
    };

    let d = diff_maps(
        &src,
        &dst,
        &by_path,
        StatTrust::default(),
        CasePairing::Exact,
        false,
    );
    assert_eq!(
        d.changed,
        vec!["two.txt"],
        "only the sha256-settled file, despite both disagreeing"
    );

    // And all-of on the same maps reports **both**. This is the half that makes the
    // assertion above mean something: the disagreement is in the data either way, so
    // if `diff_maps` were ignoring `required` altogether it would land here and fail.
    let uniform = Required {
        by_rel: HashMap::new(),
        fallback: AlgoSet::from_iter([HashAlg::Md5, HashAlg::Sha256]),
    };
    let d = diff_maps(
        &src,
        &dst,
        &uniform,
        StatTrust::default(),
        CasePairing::Exact,
        false,
    );
    assert_eq!(
        d.changed,
        vec!["one.txt", "two.txt"],
        "with no per-path answer the whole requested list is consulted"
    );
}

/// A path the plan says nothing about falls back to the whole requested list,
/// never to nothing. `hashes_differ` is silent when a digest is missing, so an
/// empty fallback would report every unplanned pair as equal.
#[test]
fn a_path_the_plan_did_not_reach_is_compared_by_the_whole_requested_list() {
    let src = eff(vec![(
        "a.txt",
        &digests(&[("md5", b"same".as_slice()), ("sha256", b"x".as_slice())]),
    )]);
    let dst = eff(vec![(
        "a.txt",
        &digests(&[("md5", b"same".as_slice()), ("sha256", b"y".as_slice())]),
    )]);
    let populated = Required {
        by_rel: HashMap::new(),
        fallback: AlgoSet::from_iter([HashAlg::Md5, HashAlg::Sha256]),
    };
    let d = diff_maps(
        &src,
        &dst,
        &populated,
        StatTrust::default(),
        CasePairing::Exact,
        false,
    );
    assert_eq!(
        d.changed,
        vec!["a.txt"],
        "sha256 disagrees and the fallback catches it"
    );
    // And with an *empty* fallback the same maps come out clean, which is the whole
    // reason the fallback is populated rather than left as `Required::default()`:
    // `hashes_differ` is silent when handed nothing, so a missing entry read as
    // "equal" would be a verdict about content nobody read.
    assert_eq!(
        diff_maps(
            &src,
            &dst,
            &Required::default(),
            StatTrust::default(),
            CasePairing::Exact,
            false
        )
        .changed,
        Vec::<String>::new(),
        "which is why `plan_pairs` always sets it"
    );
}
