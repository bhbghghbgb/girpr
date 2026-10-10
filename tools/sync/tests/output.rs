//! The stdout contract: `--output json` is the same records `--output text`
//! renders, and it is the only thing on stdout.
//!
//! Everything else in the suite reads records, not lines. This file is the one
//! place that runs the *binary* against the flag, because the flag's whole
//! journey — clap, `LogCtx`, the `Report` every command holds — exists only in
//! the real process. A record asserted in `report.rs` can be right while the flag
//! never reaches the printer; a verdict asserted against `verdict()` can be right
//! while the binary renders something else entirely.
//!
//! The property that makes this worth having is the last test in the set: with
//! `--output json`, stdout is a clean NDJSON stream at *any* `--log-level`.
//! Diagnostics are the other channel, so a caller can pipe stdout straight into a
//! parser without filtering chatter out of the stream it asked for.

mod common;

use common::{TempRoot, parse_ndjson, summary_of, sync_mtime, wfile};
use serde_json::Value;
use std::path::Path;

/// One invocation of the real binary: exit code, parsed stdout, raw stderr.
struct Run {
    code: i32,
    records: Vec<Value>,
    stderr: String,
}

impl Run {
    /// Every record, rendered back for a failure message.
    fn show(&self) -> String {
        self.records
            .iter()
            .map(|r| r.to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn run(args: &[String]) -> Run {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_girsync"))
        .args(args)
        .output()
        .expect("spawn girsync");
    Run {
        code: out.status.code().expect("girsync exits with a code"),
        records: parse_ndjson(&out.stdout),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

/// A src/dst pair with one of each difference class, so a report has something to
/// say in every bucket. Casing is set up so `case-mismatch` fires too.
fn fixture(t: &TempRoot) -> (std::path::PathBuf, std::path::PathBuf) {
    let src = t.mkdirs("src");
    let dst = t.mkdirs("dst");
    wfile(&src, "same.txt", b"identical");
    wfile(&dst, "same.txt", b"identical");
    sync_mtime(&src.join("same.txt"), &dst.join("same.txt"));
    wfile(&src, "changed.txt", b"src-version-longer");
    wfile(&dst, "changed.txt", b"dst");
    wfile(&src, "Data.txt", b"payload");
    wfile(&dst, "data.txt", b"payload");
    sync_mtime(&src.join("Data.txt"), &dst.join("data.txt"));
    wfile(&src, "only_src.txt", b"only here");
    wfile(&dst, "only_dst.txt", b"only there");
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

#[test]
fn compare_json_is_one_object_per_line_with_a_single_summary() {
    let t = TempRoot::new("out_cmp");
    let (src, dst) = fixture(&t);
    let r = run(&compare_args(&src, &dst, &["--output", "json"]));
    assert_eq!(r.code, 4, "differences exit 4: {}", r.show());
    // Every line parsed — `parse_ndjson` would have panicked otherwise — and every
    // one names its event, so a consumer dispatches on a single key.
    assert!(
        r.records.iter().all(|rec| rec["event"].is_string()),
        "every record has an event: {}",
        r.show()
    );
    assert_eq!(
        summary_of(&r.records),
        serde_json::json!({"event": "summary", "missing": 2, "extra": 1, "changed": 1,
                           "type_conflict": 1, "case_mismatch": 1, "total_diff": 5}),
        "the default is insensitive mode, so the case-only pair is a case-mismatch \
         rather than a missing plus an extra; and a_dir's contents are src-only \
         while a_dir itself is a kind conflict\n{}",
        r.show()
    );
}

/// The report the binary printed is the report `verdict` builds.
///
/// `verdict` is the single definition of a diff's records, and this is what keeps
/// `--output json` honest: the flag chooses a *rendering*, so the objects on stdout
/// must be the ones the library hands out. Two derivations would drift, and only
/// one of them is what `compare`, `compare-self` and the tests read.
#[test]
fn compare_json_is_the_verdict_the_library_builds() {
    let t = TempRoot::new("out_verdict");
    let (src, dst) = fixture(&t);
    // `--case-sensitive` because the library-side harness below is case-sensitive,
    // and the two must be answering the same question for the comparison to mean
    // anything.
    let r = run(&compare_args(
        &src,
        &dst,
        &["--output", "json", "--case-sensitive"],
    ));

    // The same pair, through the library, resolving the way `cmd_compare` does.
    let o = common::compare(src, dst);
    let (sm, dm) = common::resolve_both(&o.src, &o.dst, o.trust);
    let diff = girsync::diff::diff_maps(
        &sm.map,
        &dm.map,
        &girsync::planner::Required {
            by_rel: Default::default(),
            fallback: o.common.algos.clone(),
        },
        o.common.stat,
        o.common.case_sensitive,
        false,
    );
    let lib: Vec<Value> = girsync::report::verdict(&diff, false, false)
        .iter()
        .map(|rec| rec.json())
        .collect();

    assert_eq!(r.code, 4, "the fixture must produce a verdict");
    assert_eq!(
        r.records, lib,
        "the binary's JSON is the library's verdict, record for record"
    );
}

#[test]
fn update_json_names_the_directory_it_refreshed() {
    let t = TempRoot::new("out_upd");
    let dir = t.mkdirs("w");
    wfile(&dir, "a.txt", b"hello");
    wfile(&dir, "sub/b.txt", b"world");
    let r = run(&[
        "update".to_string(),
        "--dir".to_string(),
        dir.display().to_string(),
        "--output".to_string(),
        "json".to_string(),
    ]);
    assert_eq!(r.code, 0);
    assert_eq!(
        r.records.len(),
        1,
        "update reports one record: {}",
        r.show()
    );
    assert_eq!(
        r.records[0],
        serde_json::json!({"event": "update", "dir": dir.display().to_string(),
                           "files": 2, "dirs": 1, "algos": ["md5"]})
    );
}

/// Both formats carry the same records in the same order, so a caller can pick one
/// per invocation without the tool behaving differently.
#[test]
fn the_two_formats_report_the_same_records_in_the_same_order() {
    let t = TempRoot::new("out_both");
    let (src, dst) = fixture(&t);
    let mut base: Vec<String> = vec![
        "sync".into(),
        "--src".into(),
        src.display().to_string(),
        "--dst".into(),
        dst.display().to_string(),
        "--dry-run".into(),
    ];

    let text = {
        let mut a = base.clone();
        a.push("--output".into());
        a.push("text".into());
        std::process::Command::new(env!("CARGO_BIN_EXE_girsync"))
            .args(&a)
            .output()
            .expect("spawn girsync")
    };
    assert!(text.status.success());
    let lines: Vec<String> = String::from_utf8(text.stdout)
        .expect("utf-8")
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.to_string())
        .collect();

    base.push("--output".into());
    base.push("json".into());
    let r = run(&base);

    assert_eq!(
        lines.len(),
        r.records.len(),
        "one line per record in both formats\ntext: {lines:?}\njson: {}",
        r.show()
    );
    // Each line leads with the event's text label, which is the record's own name
    // rather than a parallel vocabulary: `fix-dir` prints `FIX-DIR`, and the event
    // name is that label lowercased.
    for (line, rec) in lines.iter().zip(&r.records) {
        let label = line.split_whitespace().next().unwrap();
        assert_eq!(
            label.to_lowercase(),
            rec["event"].as_str().unwrap(),
            "the text label and the JSON event are the same name: {line} / {rec}"
        );
    }
}

/// The point of the flag: stdout is a clean NDJSON stream even while the run
/// narrates.
///
/// The default `--log-level` is `info`, so this run *does* emit log events. If
/// any of them landed on stdout, every consumer would have to filter the stream
/// it asked for, and `--output json` would be a formatting preference rather than
/// a machine interface.
#[test]
fn json_stdout_stays_clean_while_the_run_narrates() {
    let t = TempRoot::new("out_clean");
    let (src, dst) = fixture(&t);
    // No --log-level: the default narrates at info, the noisiest ordinary setting.
    let r = run(&compare_args(&src, &dst, &["--output", "json"]));
    assert_eq!(r.code, 4);
    assert!(
        r.records.len() > 1,
        "the run reported something: {}",
        r.show()
    );
    assert!(
        !r.stderr.is_empty(),
        "and it narrated on stderr, so the assertion above is not vacuous"
    );
}

/// An unknown `--output` is a CLI error, so it exits `2` like any other clap
/// mistake rather than silently falling back to text — a caller asking for JSON
/// must never get text and have to notice.
#[test]
fn an_unknown_output_format_is_rejected_by_the_parser() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_girsync"))
        .args(["compare", "--src", "a", "--dst", "b", "--output", "yaml"])
        .output()
        .expect("spawn girsync");
    assert_eq!(
        out.status.code(),
        Some(2),
        "a bad --output is a parse error, not a fallback"
    );
}
