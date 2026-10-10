//! What the diff concluded about each pair, and the tag that says so.
//!
//! `--why` prints a tag per record, so a tag is a **contract**: a parser splits it
//! on the first `:` and switches on what is left. That makes the vocabulary worth
//! pinning per pair state, which is what the table below does — each row names a
//! path, the pair state it is put in, and the one classification and tag that state
//! must produce.
//!
//! ## Why a tag per *decision* and not per check
//!
//! `diff` is a short circuit: a trusted stat field settles the pair and no digest is
//! read at all. So the honest reason is whatever *settled* it. That makes "never
//! report previous checks' efforts" fall out for free — there is no earlier evidence
//! to accumulate — instead of needing enforcement.
//!
//! ## Why the table, and not a transcript
//!
//! Each row is a claim about intent that survives a change of implementation, fails
//! naming the path, and adds behaviour as a row rather than as an edit to a
//! recorded vector.
//!
//! ## The rows that earn their place
//!
//! - `digest-differs:md5+sha256` is the one that pins **which** algorithms a tag
//!   names. `--hash-all-of md5 sha256` asks for both, so a reason built from the
//!   *request* would look identical to one built from the *disagreement* — and the
//!   difference is the whole usefulness of the field.
//! - The `any-of` row is the same claim against a different mechanism. There the run
//!   asked for two algorithms and the planner picked one; naming the request would
//!   say `md5+sha256` for a pair only `md5` ever touched, and `pick_one`'s tier
//!   logic makes the wrong answer entirely plausible.
//! - `unverifiable:*` is pinned because it means **"cannot tell"**, not "differs".
//!   Folding it into a stat tag would report a difference the run explicitly refused
//!   to accept as evidence.

mod common;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use common::{TempRoot, age, sync_mtime, wfile};
use girsync::config::ScanMode;
use girsync::diff::{StatField, Verdict, hashes_differ, pair_verdict};
use girsync::effective::{classify, ensure_distinct_sides, open_side, resolve_side};
use girsync::planner::{HashMode, PairPlan, Required, SideRequest, StatTrust, plan_pairs};
use girsync::{CommonOpts, EffRec, cmd_update};

// -- the table -----------------------------------------------------------------

/// One row: a path in a fixture, and what the diff must conclude about it.
struct Row {
    /// What the row is about. The failure message, and the test's own index.
    name: &'static str,
    /// The path this row is about — the key the table is indexed by.
    rel: &'static str,
    /// Build the fixture, in a fresh [`TempRoot`] of its own.
    ///
    /// A closure rather than shared state because resolving a folder side **writes
    /// to its cache**: measuring two configurations against one fixture makes the
    /// second find whatever the first backfilled, and the tag under test is a
    /// function of what was *not* read.
    build: fn(&TempRoot) -> (PathBuf, PathBuf),
    /// The run's options for this row.
    common: fn() -> CommonOpts,
    /// The classification the pair must carry.
    verdict: Verdict,
    /// The tag [`Verdict::why_tag`] must render for it — the contract a parser sees.
    tag: &'static str,
}

/// Every pair state the diff can reach, and the tag each must print.
///
/// Built as rows rather than as one fixture with many paths because the rows do not
/// share a configuration: three need different algorithms, one needs a record side,
/// and one needs stat trust switched off. A single fixture would need one
/// configuration that satisfies none of them faithfully.
fn rows() -> Vec<Row> {
    vec![
        // -- settled by a trusted stat field, with no digest read at all --------
        Row {
            name: "size differs, mtime agrees",
            rel: "a.txt",
            build: |t| pinned(t, "a.txt", b"short", b"a considerably longer body"),
            common: md5,
            verdict: Verdict::Stat {
                field: StatField::Size,
                both: false,
            },
            tag: "stat-size",
        },
        Row {
            name: "size agrees, mtime differs",
            rel: "b.txt",
            build: |t| {
                let (s, d) = pinned(t, "b.txt", b"identical bytes", b"identical bytes");
                // Identical bytes and identical size, so the timestamp is the only
                // thing left that differs.
                age(&d, "b.txt", 60);
                (s, d)
            },
            common: md5,
            verdict: Verdict::Stat {
                field: StatField::Mtime,
                both: false,
            },
            tag: "stat-mtime",
        },
        Row {
            name: "both differ",
            rel: "c.txt",
            build: |t| {
                let (s, d) = pinned(t, "c.txt", b"short", b"a considerably longer body");
                // Explicitly, rather than relying on the two writes landing on
                // different timestamps: a pair that differs in mtime as well would
                // report both fields, and the row under test is the one that differs
                // in size *and* in mtime on purpose.
                age(&d, "c.txt", 60);
                (s, d)
            },
            common: md5,
            verdict: Verdict::Stat {
                field: StatField::Size,
                both: true,
            },
            tag: "stat-size+stat-mtime",
        },
        // -- settled by digests -------------------------------------------------
        Row {
            name: "stat agrees, md5 differs",
            rel: "d.txt",
            // Same length, different bytes: nothing but a digest can decide it.
            build: |t| pinned(t, "d.txt", b"aaaa", b"bbbb"),
            common: md5,
            verdict: Verdict::Differs {
                algos: vec!["md5".into()],
            },
            tag: "digest-differs:md5",
        },
        Row {
            // The row that pins *which* algorithms a tag names: both were asked for
            // and both disagreed, and both are named — in the order they were asked.
            name: "stat agrees, md5 and sha256 differ",
            rel: "d.txt",
            build: |t| pinned(t, "d.txt", b"aaaa", b"bbbb"),
            common: md5_and_sha256,
            verdict: Verdict::Differs {
                algos: vec!["md5".into(), "sha256".into()],
            },
            tag: "digest-differs:md5+sha256",
        },
        Row {
            name: "stat agrees, md5 agrees",
            rel: "e.txt",
            build: |t| pinned(t, "e.txt", b"same", b"same"),
            common: md5,
            verdict: Verdict::Matches {
                algos: vec!["md5".into()],
            },
            tag: "digest-matches:md5",
        },
        // -- no digest consulted, and nothing left to consult --------------------
        Row {
            // `--hash-all-of none` on a stat-equal pair: the digest branch had
            // nothing to compare, and this run trusts both stat fields, so there was
            // no difference to distrust either. "No evidence of difference" here is
            // genuinely "no difference" — which is the one case where the same shape
            // of answer is honest.
            name: "stat agrees, no digest requested",
            rel: "f.txt",
            build: |t| pinned(t, "f.txt", b"same", b"same"),
            common: none,
            verdict: Verdict::StatMatch,
            tag: "stat-match",
        },
        Row {
            // The distinction that is easy to lose: `none` **with** size distrusted
            // is not `stat-match`. There was a difference, it was one this run
            // refused to accept as evidence, and nothing was left to decide the pair
            // by — so the honest answer is "cannot tell", which is `CHANGED` and not
            // "in step". Reporting it `stat-match` would be a false clean.
            name: "hash none, size differs and size distrusted",
            rel: "g.txt",
            build: |t| pinned(t, "g.txt", b"short", b"a considerably longer body"),
            common: none_without_trusting_size,
            verdict: Verdict::Unverifiable {
                fields: vec![StatField::Size],
            },
            tag: "unverifiable:size",
        },
        Row {
            // Both distrusted fields differ at once, so the detail list has to
            // combine rather than pick one.
            name: "hash none, both fields differ and both are distrusted",
            rel: "h.txt",
            build: |t| {
                let (s, d) = pinned(t, "h.txt", b"short", b"a considerably longer body");
                age(&d, "h.txt", 60);
                (s, d)
            },
            common: none_without_trusting_anything,
            verdict: Verdict::Unverifiable {
                fields: vec![StatField::Size, StatField::Mtime],
            },
            tag: "unverifiable:size+mtime",
        },
        // -- presence only -------------------------------------------------------
        Row {
            name: "directory on both sides",
            rel: "d",
            build: |t| {
                let s = t.mkdirs("src");
                let d = t.mkdirs("dst");
                std::fs::create_dir_all(s.join("d")).unwrap();
                std::fs::create_dir_all(d.join("d")).unwrap();
                (s, d)
            },
            common: md5,
            verdict: Verdict::DirPresent,
            tag: "dir-present",
        },
        // -- the pick, not the request --------------------------------------------
        Row {
            // Two algorithms asked for; the planner could only pick `md5`, because a
            // record side cannot read `sha256` and does not hold it. A tag built from
            // the request would name both, and would be wrong about the one algorithm
            // that actually settled the pair.
            name: "hash-any-of md5 sha256, record holds only md5",
            rel: "a.txt",
            build: |t| {
                let old = t.mkdirs("old");
                let live = t.mkdirs("live");
                wfile(&old, "a.txt", b"aaaa");
                // Warm only md5, so the record is short of sha256 exactly as an
                // older run would leave it.
                cmd_update(
                    girsync::UpdateOpts {
                        dir: old.clone(),
                        common: md5(),
                    },
                    &common::log(),
                )
                .unwrap();
                wfile(&live, "a.txt", b"bbbb");
                // Pinned, so the pair is undecided and the planner is consulted at all.
                sync_mtime(&old.join("a.txt"), &live.join("a.txt"));
                (old.join(girsync::cache::CACHE_PREFIX), live)
            },
            common: any_of_md5_sha256,
            verdict: Verdict::Differs {
                algos: vec!["md5".into()],
            },
            tag: "digest-differs:md5",
        },
    ]
}

// -- configurations ------------------------------------------------------------

fn md5() -> CommonOpts {
    common::opts()
}

fn md5_and_sha256() -> CommonOpts {
    common::with_algos(&["md5", "sha256"])
}

fn none() -> CommonOpts {
    common::with_algos(&[])
}

fn none_without_trusting_size() -> CommonOpts {
    CommonOpts {
        algos: vec![],
        stat: StatTrust::default().without_size(),
        ..common::opts()
    }
}

fn none_without_trusting_anything() -> CommonOpts {
    CommonOpts {
        algos: vec![],
        stat: StatTrust::default().without_size().without_mtime(),
        ..common::opts()
    }
}

fn any_of_md5_sha256() -> CommonOpts {
    CommonOpts {
        algos: vec!["md5".to_string(), "sha256".to_string()],
        hash_mode: HashMode::AnyOf,
        ..common::opts()
    }
}

// -- fixture -------------------------------------------------------------------

/// Two folders holding `rel` with the given bytes on each side, dst's mtime pinned
/// to src's.
///
/// The pin is load-bearing wherever a *rewrite* is the fixture: writing moves the
/// mtime, and a pair whose mtime also differs is settled by a stat field instead of
/// by the digest this row is about — so the row would appear not to work, for a
/// reason that has nothing to do with the code.
fn pinned(t: &TempRoot, rel: &str, src_bytes: &[u8], dst_bytes: &[u8]) -> (PathBuf, PathBuf) {
    let src = t.mkdirs("src");
    let dst = t.mkdirs("dst");
    wfile(&src, rel, src_bytes);
    wfile(&dst, rel, dst_bytes);
    sync_mtime(&src.join(rel), &dst.join(rel));
    (src, dst)
}

// -- harness -------------------------------------------------------------------

/// The two effective maps and the planner's answer, from the exact phase sequence
/// `cmd_compare` runs.
///
/// Driven here rather than delegated to so a row can read the *pair's* classification
/// rather than only the buckets: `--why` prints what the diff concluded, and a row
/// that could only see `CHANGED` could not tell a stat verdict from a digest one.
/// `temp_tag` names the fixture so a panic points at a directory that still exists.
fn resolved(
    temp_tag: &str,
    src: &Path,
    dst: &Path,
    common: &CommonOpts,
) -> (HashMap<String, EffRec>, HashMap<String, EffRec>, PairPlan) {
    let (s_side, d_side) = (classify(src), classify(dst));
    ensure_distinct_sides(&s_side, &d_side)
        .unwrap_or_else(|e| panic!("{temp_tag}: the two sides name one cache: {e:#}"));
    let mode = ScanMode {
        no_trust_cached_hashes: false,
        dry_run: false,
    };
    let mut s =
        open_side(&s_side, common, mode).unwrap_or_else(|e| panic!("{temp_tag}: src side: {e:#}"));
    let mut d =
        open_side(&d_side, common, mode).unwrap_or_else(|e| panic!("{temp_tag}: dst side: {e:#}"));
    let plans = plan_pairs(
        SideRequest {
            entries: &s.phase_a.map,
            algos: &common.algos,
            no_trust: false,
            cap: s.cap,
            label: &format!("src {}", s_side.cache_path().display()),
        },
        SideRequest {
            entries: &d.phase_a.map,
            algos: &common.algos,
            no_trust: false,
            cap: d.cap,
            label: &format!("dst {}", d_side.cache_path().display()),
        },
        common.case_sensitive,
        common.hash_mode,
        common.stat,
    )
    .unwrap_or_else(|e| panic!("{temp_tag}: this pair is answerable: {e:#}"));
    let sm = resolve_side(&mut s, mode, &plans.src)
        .unwrap_or_else(|e| panic!("{temp_tag}: resolving src: {e:#}"));
    let dm = resolve_side(&mut d, mode, &plans.dst)
        .unwrap_or_else(|e| panic!("{temp_tag}: resolving dst: {e:#}"));
    (sm.map, dm.map, plans)
}

// -- the cases -----------------------------------------------------------------

/// **The whole table.** One classification and one tag per pair state, with the
/// verdict and the tag asserted separately — a tag can be right for a wrong verdict
/// and the other way round, and `--why` ships both.
#[test]
fn every_pair_state_has_one_classification_and_one_tag() {
    for row in rows() {
        let t = TempRoot::new(&format!("why_{}", row.name.replace(' ', "_")));
        let (src, dst) = (row.build)(&t);
        let common = (row.common)();
        let (sm, dm, plans) = resolved(row.name, &src, &dst, &common);

        let s = sm
            .get(row.rel)
            .unwrap_or_else(|| panic!("{}: src has no entry for {}", row.name, row.rel));
        let d = dm
            .get(row.rel)
            .unwrap_or_else(|| panic!("{}: dst has no entry for {}", row.name, row.rel));

        let got = pair_verdict(plans.stat, &plans.required, s, d, row.rel);
        assert_eq!(
            got, row.verdict,
            "{}: {} is classified wrongly",
            row.name, row.rel
        );
        assert_eq!(
            got.why_tag(),
            row.tag,
            "{}: {} must report this tag, and the classification above must be why",
            row.name,
            row.rel
        );
    }
}

/// The two buckets the classification feeds are complements, and neither is a
/// default. `CHANGED` reads `is_changed` and `--show-identical` reads
/// `is_identical`, so a third state — or a state in both — would put a pair in two
/// buckets or none, and `Diff::total()` feeds the exit code.
#[test]
fn the_two_buckets_are_complementary_and_exhaustive() {
    for row in rows() {
        assert!(
            row.verdict.is_changed() != row.verdict.is_identical(),
            "{}: {:?} is in both buckets or neither",
            row.name,
            row.verdict
        );
    }
}

/// The table has to keep covering the vocabulary, or it stops being a specification
/// and becomes a sample: losing the `dir-present` row would leave that tag pinned
/// only by the one case that happens to reach it.
///
/// Asserted over the shape rather than over a list of names, so a new variant is a
/// failing line here rather than a silently untested one.
#[test]
fn the_table_covers_every_classification() {
    use std::collections::BTreeSet;
    let mut seen: BTreeSet<&'static str> = BTreeSet::new();
    for row in rows() {
        let name = match &row.verdict {
            Verdict::Stat { both: true, .. } => "stat (both fields)",
            Verdict::Stat { .. } => "stat (one field)",
            Verdict::Differs { .. } => "differs",
            Verdict::Matches { .. } => "matches",
            Verdict::StatMatch => "stat-match",
            Verdict::Unverifiable { .. } => "unverifiable",
            Verdict::DirPresent => "dir-present",
        };
        seen.insert(name);
    }
    for required in [
        "stat (one field)",
        "stat (both fields)",
        "differs",
        "matches",
        "stat-match",
        "unverifiable",
        "dir-present",
    ] {
        assert!(
            seen.contains(required),
            "no row covers `{required}`, so that classification is no longer pinned \
             by this table: {seen:?}"
        );
    }
}

/// Two algorithms that disagree are both named, and in the order they were asked
/// for. `required.by_rel` is written in `algos` order, so a tag built from the
/// disagreement inherits it — and a tag built from the *request* would be
/// indistinguishable from it here, which is why the `any-of` row exists as well.
///
/// Pinned as its own case because it is the one field a parser cannot reconstruct.
#[test]
fn a_tag_names_the_algorithms_that_disagreed_in_the_order_they_were_asked_for() {
    let t = TempRoot::new("why_algo_order");
    let (src, dst) = pinned(&t, "a.txt", b"aaaa", b"bbbb");
    let common = md5_and_sha256();
    let (sm, dm, plans) = resolved("algo order", &src, &dst, &common);

    let got = pair_verdict(
        plans.stat,
        &plans.required,
        &sm["a.txt"],
        &dm["a.txt"],
        "a.txt",
    );
    assert_eq!(
        got,
        Verdict::Differs {
            algos: vec!["md5".into(), "sha256".into()]
        }
    );
    assert_eq!(got.why_tag(), "digest-differs:md5+sha256");
}

// -- the disagreement, not the request -----------------------------------------

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
        hashes: hashes
            .iter()
            .map(|a| (a.to_string(), vec![a.len() as u8; 16]))
            .collect(),
    }
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
        hashes: [("md5".to_string(), vec![9u8; 16])].into_iter().collect(),
        ..rec(&[])
    };
    let required = Required {
        by_rel: [(
            "a.txt".to_string(),
            vec!["md5".to_string(), "sha256".to_string()],
        )]
        .into_iter()
        .collect(),
        fallback: vec![],
    };
    let differs = hashes_differ(&s.hashes, &d.hashes, required.of("a.txt"));
    let got = pair_verdict(StatTrust::default(), &required, &s, &d, "a.txt");

    assert_eq!(differs, vec!["md5".to_string()], "only md5 was comparable");
    assert_eq!(
        got,
        Verdict::Differs {
            algos: vec!["md5".into()]
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
/// disagreement. `hashes_differ` is silent either way, so a reader of its `bool`
/// could not tell the two apart — which is the `Required::fallback` trap this crate
/// has already paid for once.
#[test]
fn a_skipped_algorithm_is_never_reported_as_a_disagreement() {
    let s = rec(&["md5", "sha256"]);
    let required = Required {
        by_rel: [(
            "a.txt".to_string(),
            vec!["md5".to_string(), "sha256".to_string()],
        )]
        .into_iter()
        .collect(),
        fallback: vec![],
    };
    // Same shape, but the digests both sides actually hold agree.
    let agreeing = EffRec {
        hashes: [
            ("md5".to_string(), vec![3u8; 16]),
            ("sha256".to_string(), vec![6u8; 16]),
        ]
        .into_iter()
        .collect(),
        ..rec(&[])
    };
    assert_eq!(
        hashes_differ(&s.hashes, &agreeing.hashes, required.of("a.txt")),
        Vec::<String>::new(),
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
