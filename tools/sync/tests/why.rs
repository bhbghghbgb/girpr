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
use girsync::diff::{Diff, StatField, Verdict, hashes_differ, pair_verdict};
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

// -- the flag, end to end ------------------------------------------------------

/// One invocation of the real binary: exit code and stdout as text.
///
/// The binary rather than the library because the flag's whole journey — clap,
/// `CommonOpts::try_from`, the `Report` a command holds — exists only in the real
/// process. A tag asserted against `Verdict::why_tag` can be right while `--why`
/// never reaches the printer.
fn bin(args: &[String]) -> (i32, String) {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_girsync"))
        .args(args)
        .output()
        .expect("spawn girsync");
    (
        out.status.code().expect("girsync exits with a code"),
        String::from_utf8(out.stdout).expect("stdout is utf-8"),
    )
}

/// A src/dst pair with one of every difference class, so a report has something to
/// say in each bucket and `only_changed_carries_a_why` is not vacuous.
fn every_bucket(t: &TempRoot) -> (PathBuf, PathBuf) {
    let src = t.mkdirs("src");
    let dst = t.mkdirs("dst");
    // `stat-size`: differing length, mtime pinned so size is the only signal.
    wfile(&src, "sized.txt", b"src-version-longer");
    wfile(&dst, "sized.txt", b"dst");
    sync_mtime(&src.join("sized.txt"), &dst.join("sized.txt"));
    // `digest-differs:md5`: same length, different bytes, mtime pinned.
    wfile(&src, "digested.txt", b"aaaa");
    wfile(&dst, "digested.txt", b"bbbb");
    sync_mtime(&src.join("digested.txt"), &dst.join("digested.txt"));
    wfile(&src, "only_src.txt", b"only here");
    wfile(&dst, "only_dst.txt", b"only there");
    wfile(&src, "Data.txt", b"payload");
    wfile(&dst, "data.txt", b"payload");
    sync_mtime(&src.join("Data.txt"), &dst.join("data.txt"));
    wfile(&src, "a_dir/inner.txt", b"dir on src");
    wfile(&dst, "a_dir", b"file on dst");
    (src, dst)
}

fn compare_args<'a>(src: &'a Path, dst: &'a Path, extra: &[&'a str]) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "compare".into(),
        "--src".into(),
        src.display().to_string(),
        "--dst".into(),
        dst.display().to_string(),
    ];
    args.extend(extra.iter().map(|s| s.to_string()));
    args
}

/// **The two renderings of a `--why` record.** A reason is a contract a parser
/// splits on, so both shapes are pinned and neither is derived from the other.
///
/// `why` is a **flat string** in JSON, not an object: a parser splits on the first
/// `:` and switches on the name, and an object would invite questions
/// (`fields`? `detail`? `sources`?) that this does not need to answer.
#[test]
fn why_is_a_field_on_the_changed_record_in_both_formats() {
    let t = TempRoot::new("why_flag_shape");
    let (src, dst) = every_bucket(&t);

    let (code, text) = bin(&compare_args(&src, &dst, &["--why", "--case-sensitive"]));
    assert_eq!(code, 4, "the fixture must differ:\n{text}");
    assert!(
        text.contains("\nCHANGED sized.txt why=stat-size\n")
            || text.starts_with("CHANGED sized.txt why=stat-size\n"),
        "the tag follows the path, keyed:\n{text}"
    );
    assert!(
        text.contains("CHANGED digested.txt why=digest-differs:md5\n"),
        "and the digest case names the algorithm that disagreed:\n{text}"
    );

    let (code, json) = bin(&compare_args(
        &src,
        &dst,
        &["--why", "--case-sensitive", "--output", "json"],
    ));
    assert_eq!(code, 4);
    let recs: Vec<serde_json::Value> = json
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("ndjson"))
        .collect();
    assert_eq!(
        recs.iter()
            .find(|r| r["path"] == "sized.txt")
            .expect("a CHANGED record for sized.txt"),
        &serde_json::json!({"event": "changed", "path": "sized.txt", "why": "stat-size"}),
        "the same record in JSON, with `why` a flat string"
    );
}

/// **Only `CHANGED` carries a reason.** `MISSING`, `EXTRA`, `TYPE-CONFLICT` and
/// `CASE-MISMATCH` are self-explanatory — the bucket name *is* the reason — so a
/// `why` on one of them would be a tag inventing an explanation nobody asked for.
#[test]
fn only_changed_carries_a_why() {
    let t = TempRoot::new("why_only_changed");
    let (src, dst) = every_bucket(&t);
    let (_code, text) = bin(&compare_args(&src, &dst, &["--why"]));
    let with_why: Vec<&str> = text.lines().filter(|l| l.contains(" why=")).collect();
    assert!(
        with_why.iter().all(|l| l.starts_with("CHANGED ")),
        "only CHANGED lines carry why=:\n{text}"
    );
    assert!(
        !text.contains("MISSING only_src.txt why="),
        "a MISSING is explained by being MISSING:\n{text}"
    );
    assert!(
        !text.contains("CASE-MISMATCH") || !text.contains("CASE-MISMATCH Data.txt why="),
        "a CASE-MISMATCH names both spellings, which is the explanation:\n{text}"
    );
    // And the other buckets are actually present, so the case above is not vacuous.
    for expected in ["MISSING only_src.txt", "EXTRA only_dst.txt"] {
        assert!(
            text.contains(expected),
            "the fixture must contain {expected}:\n{text}"
        );
    }
}

/// **The other buckets cannot carry a reason even by accident.** Not a test about
/// the reporter: [`Diff::why_tag`] returns `None` for any path that was never
/// classified, and only *both-sides, same-kind* pairs are — so there is no `MISSING`
/// to attach a tag to in the first place. A reporter that leaked `why` onto the
/// other buckets would have nothing to leak, which is a stronger guarantee than a
/// test catching it afterwards.
///
/// Stated at the source rather than only on the output, because an output-only check
/// passes against an implementation that simply never reached the case.
#[test]
fn only_pairs_get_a_classification_and_only_classified_paths_get_a_tag() {
    let mut diff = Diff {
        missing: vec!["m.txt".into()],
        extra: vec!["e.txt".into()],
        changed: vec!["c.txt".into()],
        type_conflict: vec!["t.txt".into()],
        case_mismatch: vec![("a.txt".into(), "A.txt".into())],
        ..Diff::default()
    };
    diff.verdicts.insert(
        "c.txt".into(),
        Verdict::Differs {
            algos: vec!["md5".into()],
        },
    );
    assert_eq!(diff.why_tag("c.txt").as_deref(), Some("digest-differs:md5"));
    for path in ["m.txt", "e.txt", "t.txt", "a.txt"] {
        assert_eq!(
            diff.why_tag(path),
            None,
            "{path} is not a classified pair, so it has no reason to give"
        );
    }
}

/// **Default off, and the byte-identity it implies.** An unflagged run must carry no
/// `why` anywhere — the whole point of the flag being additive.
#[test]
fn an_unflagged_run_carries_no_why() {
    let t = TempRoot::new("why_default_off");
    let (src, dst) = every_bucket(&t);
    for extra in [
        vec!["--case-sensitive"],
        vec!["--case-sensitive", "--output", "json"],
    ] {
        let args = compare_args(&src, &dst, &extra);
        let (_code, out) = bin(&args);
        assert!(
            !out.contains("why"),
            "an unflagged run must not mention reasons: {args:?}\n{out}"
        );
    }
}

/// **`--why` is a reporting flag and must not move the exit code.** This is the
/// mutation most damaging if it ever happens, and the cheapest to write by accident:
/// a `CHANGED` record exists either way, and a summary that counted the reason field
/// would report a difference on a clean tree.
#[test]
fn why_does_not_change_the_exit_code() {
    for (tag, src_bytes, dst_bytes) in [
        ("differs", &b"src-version-longer"[..], &b"dst"[..]),
        ("clean", &b"same"[..], &b"same"[..]),
    ] {
        let t = TempRoot::new(&format!("why_exit_{tag}"));
        let src = t.mkdirs("src");
        let dst = t.mkdirs("dst");
        wfile(&src, "a.txt", src_bytes);
        wfile(&dst, "a.txt", dst_bytes);
        sync_mtime(&src.join("a.txt"), &dst.join("a.txt"));

        let off = bin(&compare_args(&src, &dst, &["--case-sensitive"]));
        let on = bin(&compare_args(&src, &dst, &["--why", "--case-sensitive"]));
        assert_eq!(
            off.0, on.0,
            "{tag}: --why is a reporting flag, so the exit code cannot move"
        );
        assert_eq!(
            on.0,
            if tag == "differs" { 4 } else { 0 },
            "{tag}: and it is the diff that decides, not the flag"
        );
        if tag == "clean" {
            assert!(
                !on.1.contains("CHANGED"),
                "a clean tree stays clean with the flag on:\n{}",
                on.1
            );
        }
    }
}

/// `compare-self` shares the reporter, so it inherits `--why` for free — and a drift
/// check is the more interesting consumer, because its `CHANGED` records are the
/// claim that the cache is out of step with the disk.
///
/// The mtime is restored after the rewrite because a rewrite moves it, and a pair
/// whose mtime also differs is settled by the timestamp — so the reason under test
/// would never be reached.
#[test]
fn compare_self_reports_a_reason_too() {
    let t = TempRoot::new("why_self");
    let dir = t.mkdirs("w");
    wfile(&dir, "a.txt", b"short");
    cmd_update(
        girsync::UpdateOpts {
            dir: dir.clone(),
            common: common::opts(),
        },
        &common::log(),
    )
    .unwrap();
    let stamped = std::fs::metadata(dir.join("a.txt"))
        .unwrap()
        .modified()
        .unwrap();
    wfile(&dir, "a.txt", b"a considerably longer body than before");
    std::fs::OpenOptions::new()
        .write(true)
        .open(dir.join("a.txt"))
        .unwrap()
        .set_modified(stamped)
        .unwrap();

    let run = |extra: &[&str]| {
        let mut args = vec![
            "compare-self".to_string(),
            "--dir".to_string(),
            dir.display().to_string(),
            "--case-sensitive".to_string(),
        ];
        args.extend(extra.iter().map(|s| s.to_string()));
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_girsync"))
            .args(&args)
            .output()
            .expect("spawn girsync");
        (
            out.status.code().expect("exit code"),
            String::from_utf8(out.stdout).expect("utf-8"),
        )
    };
    let (off_code, off) = run(&[]);
    let (on_code, on) = run(&["--why"]);
    assert_eq!(off_code, 4, "the cache has drifted:\n{off}");
    assert_eq!(on_code, off_code, "--why does not move the exit code");
    assert!(!off.contains("why"), "unflagged: {off}");
    assert!(
        on.contains("CHANGED a.txt why=stat-size\n"),
        "flagged, and the reason is the size difference the run trusted:\n{on}"
    );
}

/// The `--why` flag reaches the real binary on every command that reports a diff.
#[test]
fn why_is_accepted_by_the_parser() {
    let t = TempRoot::new("why_parser");
    let (src, dst) = every_bucket(&t);
    let (code, text) = bin(&compare_args(&src, &dst, &["--why"]));
    assert_eq!(code, 4, "--why is a flag, not a parse error:\n{text}");
}

// -- the reporter, in the library ----------------------------------------------

/// Every record a diff reports, rendered as text.
fn report(diff: &Diff, why: bool) -> Vec<String> {
    girsync::verdict(diff, why)
        .iter()
        .map(|r| r.text())
        .collect()
}

/// **`verdict(diff, why)` is one list of records with the flag folded in**, and the
/// only difference it makes is a `why` field on the `CHANGED` records.
///
/// This is the single definition of what a diff reports, so the flag is a parameter
/// rather than a second list of records to keep in step. A caller that renders its
/// own copy of the records is the drift this exists to prevent.
#[test]
fn the_reporter_takes_the_flag_and_adds_only_the_why_field() {
    let mut diff = Diff {
        missing: vec!["m.txt".into()],
        extra: vec!["e.txt".into()],
        changed: vec!["c.txt".into()],
        type_conflict: vec!["t.txt".into()],
        case_mismatch: vec![("a.txt".into(), "A.txt".into())],
        ..Diff::default()
    };
    diff.verdicts.insert(
        "c.txt".into(),
        Verdict::Differs {
            algos: vec!["md5".into()],
        },
    );

    assert_eq!(
        report(&diff, false),
        vec![
            "MISSING m.txt",
            "EXTRA e.txt",
            "CHANGED c.txt",
            "TYPE-CONFLICT t.txt",
            "CASE-MISMATCH a.txt <=> A.txt",
            "SUMMARY missing=1 extra=1 changed=1 type_conflict=1 case_mismatch=1 total_diff=4",
        ],
        "unflagged, byte for byte what this tool printed before the flag existed"
    );
    assert_eq!(
        report(&diff, true),
        vec![
            "MISSING m.txt",
            "EXTRA e.txt",
            "CHANGED c.txt why=digest-differs:md5",
            "TYPE-CONFLICT t.txt",
            "CASE-MISMATCH a.txt <=> A.txt",
            "SUMMARY missing=1 extra=1 changed=1 type_conflict=1 case_mismatch=1 total_diff=4",
        ],
        "flagged: one added field, on one record, and nothing else moves"
    );
}

/// **The exit code is `Diff`'s, and the flag is not a `Diff` input.** Stated as a
/// test because the failure it guards is the most damaging one in this feature: a
/// clean tree must stay exit `0` whatever the reporting flags say, and the only way
/// to guarantee that is for the flag never to reach `total()`.
#[test]
fn holding_reasons_does_not_make_a_pair_a_difference() {
    let mut equal_only = Diff {
        changed: vec![],
        ..Diff::default()
    };
    equal_only.verdicts.insert(
        "same.txt".into(),
        Verdict::Matches {
            algos: vec!["md5".into()],
        },
    );
    equal_only.verdicts.insert("d".into(), Verdict::DirPresent);
    assert_eq!(
        equal_only.total(),
        0,
        "a pair that came out equal is not a difference"
    );
    assert!(
        equal_only.is_empty(),
        "and a diff holding only equal pairs is empty"
    );
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
