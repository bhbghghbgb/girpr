//! Both reporting flags, through the reporter and through the real binary.
//!
//! The harness and the fixture builders are in the parent module.

use super::*;

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
    // One pair that comes out **equal**, so `--show-identical` has something to
    // report. Without it the flag is untestable here: `identical=0` and no records
    // is the correct output for a tree with nothing in step, and a case asserting
    // "records appear" would pass for the wrong reason.
    wfile(&src, "in_step.txt", b"identical");
    wfile(&dst, "in_step.txt", b"identical");
    sync_mtime(&src.join("in_step.txt"), &dst.join("in_step.txt"));
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
    girsync::verdict(diff, why, false)
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
        "unflagged, byte for byte what this tool prints without the flag"
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

/// **The flag through the real binary: default off, and orthogonal to `--why`.**
///
/// Each row is a legal combination, and all four are asserted rather than the two
/// extremes — a flag pair that only worked together would still show both extremes
/// behaving correctly.
#[test]
fn show_identical_is_default_off_and_independent_of_why() {
    let t = TempRoot::new("show_identical_surface");
    let (src, dst) = every_bucket(&t);
    let cases: [(&[&str], bool, bool); 4] = [
        (&[], false, false),
        (&["--why"], true, false),
        (&["--show-identical"], false, true),
        (&["--why", "--show-identical"], true, true),
    ];
    for (extra, want_why, want_identical) in cases {
        let mut args = vec!["--case-sensitive"];
        args.extend_from_slice(extra);
        let (code, text) = bin(&compare_args(&src, &dst, &args));
        assert_eq!(code, 4, "{extra:?} must not change the exit code:\n{text}");
        assert_eq!(
            text.lines().any(|l| l.starts_with("IDENTICAL ")),
            want_identical,
            "{extra:?}: IDENTICAL records appear only with the flag:\n{text}"
        );
        assert_eq!(
            text.lines().any(|l| l.contains(" why=")),
            want_why,
            "{extra:?}: reasons appear only with --why:\n{text}"
        );
        assert_eq!(
            text.lines().any(|l| l.contains(" identical=")),
            want_identical,
            "{extra:?}: the summary field appears only with the flag:\n{text}"
        );
    }
}

/// **The equal set is exactly the pairs that came out equal**, one record each, each
/// with its own reason.
///
/// Both flags on, so the reason is exercised too — an `IDENTICAL` with no tag would
/// leave a parser unable to tell *why* a pair is equal, which is the whole point of
/// the second bucket having its own vocabulary.
#[test]
fn every_equal_pair_is_reported_once_with_its_reason() {
    let t = TempRoot::new("show_identical_set");
    let (src, dst) = every_bucket(&t);
    wfile(&src, "same_file.txt", b"identical");
    wfile(&dst, "same_file.txt", b"identical");
    sync_mtime(&src.join("same_file.txt"), &dst.join("same_file.txt"));
    // A second directory, distinct from `every_bucket`'s `a_dir` — that one is a
    // *file* on dst, so it is a type conflict and cannot also be a matching dir.
    std::fs::create_dir_all(src.join("shared_dir")).unwrap();
    std::fs::create_dir_all(dst.join("shared_dir")).unwrap();

    let (_code, text) = bin(&compare_args(
        &src,
        &dst,
        &["--why", "--show-identical", "--case-sensitive"],
    ));
    let identical: Vec<&str> = text
        .lines()
        .filter(|l| l.starts_with("IDENTICAL "))
        .collect();
    assert_eq!(
        identical,
        vec![
            "IDENTICAL in_step.txt why=digest-matches:md5",
            "IDENTICAL same_file.txt why=digest-matches:md5",
            "IDENTICAL shared_dir why=dir-present",
        ],
        "both equal files and the directory, each once, each with its own reason:\n{text}"
    );
}

/// **`--show-identical` on a clean tree still exits 0.** The one number a caller is
/// most likely to act on, and the one a new bucket is most able to break: adding a
/// record per file to a report must not make "nothing differs" look like a failure.
#[test]
fn show_identical_does_not_change_the_exit_code_on_a_clean_tree() {
    let t = TempRoot::new("show_identical_exit");
    let src = t.mkdirs("src");
    let dst = t.mkdirs("dst");
    wfile(&src, "a.txt", b"same");
    wfile(&dst, "a.txt", b"same");
    sync_mtime(&src.join("a.txt"), &dst.join("a.txt"));

    let off = bin(&compare_args(&src, &dst, &["--case-sensitive"]));
    let on = bin(&compare_args(
        &src,
        &dst,
        &["--show-identical", "--case-sensitive"],
    ));
    assert_eq!(off.0, 0, "a clean tree:\n{}", off.1);
    assert_eq!(on.0, 0, "and still clean with the flag on:\n{}", on.1);
    assert!(
        !on.1.contains("CHANGED"),
        "the flag must not manufacture a difference:\n{}",
        on.1
    );
    assert!(
        on.1.contains("IDENTICAL a.txt"),
        "while still reporting the equal pair:\n{}",
        on.1
    );
    assert_eq!(
        summary_line(&on.1),
        "SUMMARY missing=0 extra=0 changed=0 type_conflict=0 case_mismatch=0 identical=1 total_diff=0",
        "total_diff stays 0 no matter how many pairs were equal"
    );
}

/// The `SUMMARY` line of a text run.
fn summary_line(text: &str) -> String {
    text.lines()
        .find(|l| l.starts_with("SUMMARY "))
        .unwrap_or_else(|| panic!("no summary in:\n{text}"))
        .to_string()
}

