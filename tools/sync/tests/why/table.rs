//! The pair-state table: one classification and one tag per state.
//!
//! The harness and the fixture builders are in the parent module.

use super::*;

/// One row: a path in a fixture, and what the diff must conclude about it.
struct Row {
    /// What the row is about. The failure message, and the test's own index.
    name: &'static str,
    /// The path this row is about — the key the table is indexed by.
    rel: &'static str,
    /// Build the fixture under `root`, which is the row's own directory inside the
    /// table's shared temp root.
    ///
    /// A closure rather than shared state because resolving a folder side **writes
    /// to its cache**: measuring two configurations against one fixture makes the
    /// second find whatever the first backfilled, and the tag under test is a
    /// function of what was *not* read.
    build: fn(&Path) -> (PathBuf, PathBuf),
    /// The run's options for this row.
    common: fn() -> CommonOpts,
    /// The classification the pair must carry.
    verdict: Verdict,
    /// The tag [`Verdict::why_tag`] must render for it — the contract a parser sees.
    tag: &'static str,
}

/// `rel` under the row's `root`, with its parents created.
///
/// Rows build their fixtures through this rather than through [`TempDir::subdir`]
/// because the whole table shares one [`TempDir`]: the per-row isolation comes from
/// the distinct `root` each row is handed.
fn subdir(root: &Path, rel: &str) -> PathBuf {
    let p = root.join(rel);
    std::fs::create_dir_all(&p).expect("create the row's subdir");
    p
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
                algos: AlgoSet::one(HashAlg::Md5),
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
                algos: AlgoSet::from_iter([HashAlg::Md5, HashAlg::Sha256]),
            },
            tag: "digest-differs:md5+sha256",
        },
        Row {
            name: "stat agrees, md5 agrees",
            rel: "e.txt",
            build: |t| pinned(t, "e.txt", b"same", b"same"),
            common: md5,
            verdict: Verdict::Matches {
                algos: AlgoSet::one(HashAlg::Md5),
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
                let s = subdir(t, "src");
                let d = subdir(t, "dst");
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
                let old = subdir(t, "old");
                let live = subdir(t, "live");
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
                algos: AlgoSet::one(HashAlg::Md5),
            },
            tag: "digest-differs:md5",
        },
    ]
}

/// **The whole table.** One classification and one tag per pair state, with the
/// verdict and the tag asserted separately — a tag can be right for a wrong verdict
/// and the other way round, and `--why` ships both.
#[test]
fn every_pair_state_has_one_classification_and_one_tag() {
    // One temp root for the whole table: 22 roots would be 22 trees to create and
    // remove. Each row still gets a subdirectory of its own, so no row can see
    // another's fixture or the cache that resolving it wrote.
    let t = temp_root("why_pair_states");
    for (i, row) in rows().into_iter().enumerate() {
        let root = t.subdir(&format!("row_{i:02}"));
        let (src, dst) = (row.build)(&root);
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
            "no row covers `{required}`, so that classification is not pinned \
             by this table: {seen:?}"
        );
    }
}

/// Two algorithms that disagree are both named. A tag built from the
/// *disagreement* rather than the request — which is why this is not
/// automatically the same as "everything that was asked for", and why the
/// `any-of` row above exists as well.
///
/// Pinned as its own case because it is the one field a parser cannot
/// reconstruct.
#[test]
fn a_tag_names_the_algorithms_that_disagreed() {
    let t = temp_root("why_algo_order");
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
            algos: AlgoSet::from_iter([HashAlg::Md5, HashAlg::Sha256])
        }
    );
    assert_eq!(got.why_tag(), "digest-differs:md5+sha256");
}

/// **The tag's order is the disagreement's, not the flag order's.** Same fixture
/// as above with the two `--hash` flags the other way round: the pair still
/// disagrees on both algorithms, and the tag is unchanged.
///
/// This is the one deliberate change to what a run prints. `--hash-all-of
/// sha256 md5` used to report `digest-differs:sha256+md5`; it now reports
/// `digest-differs:md5+sha256`. A tag that depends on how two flags were ordered
/// is a tag a parser has to normalise before it can compare it with anything, and
/// the disagreement itself has no order to depend on.
#[test]
fn a_tag_names_the_algorithms_in_a_fixed_order_not_the_flag_order() {
    let t = temp_root("why_algo_fixed_order");
    let (src, dst) = pinned(&t, "a.txt", b"aaaa", b"bbbb");
    let common = common::with_algos(&["sha256", "md5"]);
    let (sm, dm, plans) = resolved("algo fixed order", &src, &dst, &common);

    let got = pair_verdict(
        plans.stat,
        &plans.required,
        &sm["a.txt"],
        &dm["a.txt"],
        "a.txt",
    );
    assert_eq!(
        got.why_tag(),
        "digest-differs:md5+sha256",
        "the same disagreement, named the same way whichever order the flags were typed"
    );
}
