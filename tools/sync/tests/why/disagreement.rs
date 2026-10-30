//! "What disagreed" is not "what was asked for".
//!
//! The harness and the fixture builders are in the parent module.

use super::*;
use girsync::hash::DigestSet;

/// A file record with the given stat and digests. Hand-built rather than scanned,
/// because the state under test is one a scan will not produce on request: a pair
/// whose answer was decided over an algorithm one side does not hold.
///
/// The digest bytes are keyed by algorithm, so two records naming the same algorithm
/// still disagree — a fixture that made every digest equal would report every pair in
/// step, and the case under test is a pair that is not.
fn rec(hashes: &[&str]) -> EffRec {
    EffRec {
        kind: "file".into(),
        size: 10,
        mtime_ns: 100,
        hashes: digests_of_len(hashes),
        is_link: false,
    }
}

/// One digest per name.
///
/// The lengths here are deliberately **not** each algorithm's real digest length —
/// `sha256` gets six bytes in one of these. That is the point: a known name carrying
/// an unexpected length lands in the foreign arm rather than a slot, and every lookup
/// here goes **by name**, so the fixture still behaves exactly as it did when a row
/// was a `HashMap`. A `DigestSet::get(HashAlg::Sha256)` would have missed it.
fn digests<'a, I: IntoIterator<Item = (&'a str, &'a [u8])>>(pairs: I) -> DigestSet {
    let mut set = DigestSet::default();
    for (name, digest) in pairs {
        set.insert_named(name, digest);
    }
    set
}

/// `rec`'s digests: the byte is the name's length, so two records naming the same
/// algorithm still disagree.
fn digests_of_len(names: &[&str]) -> DigestSet {
    let mut set = DigestSet::default();
    for a in names {
        set.insert_named(a, &[a.len() as u8; 16]);
    }
    set
}

/// **The one case that separates "what was asked for" from "what disagreed."**
///
/// The table above cannot: whenever two algorithms both disagree, the request and
/// the disagreement are the same list, and under `--hash-any-of` the planner has
/// already narrowed the request to the single algorithm it picked. So a reason
/// assembled from `required.of(key)` would satisfy every row there.
///
/// Here they differ. `hashes_differ` skips an algorithm one side does not hold —
/// there is nothing to compare it against — so the planner's answer names **two**
/// algorithms while only one actually disagreed. A tag built from the request says
/// `digest-differs:md5+sha256` about a pair that was never compared on sha256; the
/// honest tag is `digest-differs:md5`.
///
/// Reached through `diff_maps`'s public API rather than through a run, because the
/// coverage check makes it unreachable from the command line: a side that cannot
/// hash from disk and is short an algorithm fails the run before the diff sees it.
/// It is exactly the case `hashes_differ` documents as its skip branch, and it is
/// the reason the disagreement is returned as a *list* rather than as a `bool` that
/// no caller could name.
#[test]
fn a_tag_names_only_the_algorithms_that_were_actually_compared() {
    // src holds both algorithms; dst holds only md5, and its md5 is a *different*
    // digest — so md5 disagrees and sha256 is silent because there is nothing on the
    // other side to compare it against.
    let s = rec(&["md5", "sha256"]);
    let d = EffRec {
        hashes: digests([("md5", &[9u8; 16][..])]),
        ..rec(&[])
    };
    let required = Required {
        by_rel: [(
            "a.txt".to_string(),
            AlgoSet::from_iter([HashAlg::Md5, HashAlg::Sha256]),
        )]
        .into_iter()
        .collect(),
        fallback: AlgoSet::NONE,
    };
    let differs = hashes_differ(&s.hashes, &d.hashes, required.of("a.txt"));
    let got = pair_verdict(StatTrust::default(), &required, &s, &d, "a.txt");

    assert_eq!(
        differs,
        AlgoSet::one(HashAlg::Md5),
        "only md5 was comparable"
    );
    assert_eq!(
        got,
        Verdict::Differs {
            algos: AlgoSet::one(HashAlg::Md5)
        },
        "and only md5 is named, though the planner's answer named two"
    );
    assert_eq!(
        got.why_tag(),
        "digest-differs:md5",
        "a tag built from the request would name an algorithm nobody compared"
    );
}

/// The mirror of the above, and the reason it needed saying twice: when the
/// algorithms that *were* compared agree, silence must not be read as a
/// disagreement. `hashes_differ` is silent either way — empty covers both "they
/// agree" and "there was nothing to compare" — so the caller has to tell them
/// apart, and a `bool` return would have thrown that away. That is what
/// `Required::fallback` exists for.
#[test]
fn a_skipped_algorithm_is_never_reported_as_a_disagreement() {
    let s = rec(&["md5", "sha256"]);
    let required = Required {
        by_rel: [(
            "a.txt".to_string(),
            AlgoSet::from_iter([HashAlg::Md5, HashAlg::Sha256]),
        )]
        .into_iter()
        .collect(),
        fallback: AlgoSet::NONE,
    };
    // Same shape, but the digests both sides actually hold agree.
    let agreeing = EffRec {
        hashes: digests([("md5", &[3u8; 16][..]), ("sha256", &[6u8; 16][..])]),
        ..rec(&[])
    };
    assert_eq!(
        hashes_differ(&s.hashes, &agreeing.hashes, required.of("a.txt")),
        AlgoSet::NONE,
        "nothing disagreed"
    );
    let got = pair_verdict(StatTrust::default(), &required, &s, &agreeing, "a.txt");

    assert!(
        matches!(got, Verdict::Matches { .. }),
        "a digest that was compared and agreed is not a disagreement: {got:?}"
    );
    assert_eq!(
        got.why_tag(),
        "digest-matches:md5+sha256",
        "and both are named, because both were consulted"
    );
}
