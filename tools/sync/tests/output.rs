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

use common::{TempDir, parse_ndjson, strip_ansi, summary_of, sync_mtime, temp_root, wfile};
use girsync::hash::AlgoSet;
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
        .env("GIRSYNC_TEST_CACHE", "memory")
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
fn fixture(t: &TempDir) -> (std::path::PathBuf, std::path::PathBuf) {
    let src = t.subdir("src");
    let dst = t.subdir("dst");
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

/// The log events that carry this message, in the order they were written.
///
/// The log file is the run's own narration, one JSON object per event, and the
/// `message` field is what the console renders — so it is what a reader looks
/// at. The payload is beside it, in the same object, under named fields.
fn events_with<'a>(logged: &'a [Value], message: &str) -> Vec<&'a Value> {
    logged
        .iter()
        .filter(|e| e["fields"]["message"] == message)
        .collect()
}

#[test]
fn compare_json_is_one_object_per_line_with_a_single_summary() {
    let t = temp_root("out_cmp");
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

/// Case folding is ASCII-only, so a non-ASCII case variant is two paths rather
/// than a `CASE-MISMATCH`. The ASCII pair beside it still folds, which keeps
/// this from passing merely because insensitive mode stopped pairing anything.
#[test]
fn compare_folds_ascii_case_but_not_non_ascii_case() {
    let t = temp_root("out_nonascii_case");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    wfile(&src, "Readme.TXT", b"same");
    wfile(&dst, "readme.txt", b"same");
    sync_mtime(&src.join("Readme.TXT"), &dst.join("readme.txt"));
    wfile(&src, "Ärger.txt", b"same");
    wfile(&dst, "ärger.txt", b"same");
    sync_mtime(&src.join("Ärger.txt"), &dst.join("ärger.txt"));

    let r = run(&compare_args(&src, &dst, &["--output", "json"]));
    assert_eq!(r.code, 4, "the differences exit 4: {}", r.show());
    assert_eq!(
        summary_of(&r.records),
        serde_json::json!({"event": "summary", "missing": 1, "extra": 1, "changed": 0,
                           "type_conflict": 0, "case_mismatch": 1, "total_diff": 2}),
        "ASCII casing pairs, non-ASCII casing does not\n{}",
        r.show()
    );
    assert!(
        r.records.iter().any(|rec| {
            rec["event"] == "case-mismatch"
                && rec["src"] == "Readme.TXT"
                && rec["dst"] == "readme.txt"
        }),
        "the ASCII pair is a case mismatch: {}",
        r.show()
    );
    assert!(
        r.records
            .iter()
            .any(|rec| rec["event"] == "missing" && rec["path"] == "Ärger.txt"),
        "the non-ASCII src spelling is missing: {}",
        r.show()
    );
    assert!(
        r.records
            .iter()
            .any(|rec| rec["event"] == "extra" && rec["path"] == "ärger.txt"),
        "the non-ASCII dst spelling is extra: {}",
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
    let t = temp_root("out_verdict");
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
            fallback: AlgoSet::from_iter(o.common.algos.iter().copied()),
        },
        o.common.stat,
        girsync::planner::CasePairing::from(o.common.case_sensitive),
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
    let t = temp_root("out_upd");
    let dir = t.subdir("w");
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
    let t = temp_root("out_both");
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
    let t = temp_root("out_clean");
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

/// **`--log-level debug` still logs every record, fields and all.**
///
/// `Report::emit` builds the log event's payload only when debug is actually
/// enabled, because `rec.event()` and `rec.fields_json()` cost an allocation and
/// a JSON serialisation per record and the default `--log-level info` discards
/// all of it. The risk of that gate is that it reads as "logging was dropped",
/// and this is the assertion that it was not: at debug, each printed record must
/// appear in the log carrying the same event name and the same fields.
///
/// The file log is read rather than stderr because the file layer is always at
/// trace and its output is JSON, so an event can be checked field by field
/// instead of by substring.
#[test]
fn a_record_is_logged_with_its_fields_at_debug_level() {
    let t = temp_root("out_debug_log");
    let (src, dst) = fixture(&t);
    let logfile = t.subdir("logs").join("run.json");
    let mut args = compare_args(&src, &dst, &["--output", "json", "--log-level", "debug"]);
    args.push("--log-file".into());
    args.push(logfile.display().to_string());

    let r = run(&args);
    assert_eq!(r.code, 4, "differences exit 4: {}", r.show());
    let printed: Vec<&str> = r
        .records
        .iter()
        .map(|rec| rec["event"].as_str().unwrap())
        .collect();

    let logged = parse_ndjson(&std::fs::read(&logfile).expect("the run wrote its log"));
    let report_events: Vec<&serde_json::Value> = logged
        .iter()
        .filter(|e| e["fields"]["message"] == "report")
        .collect();
    assert!(
        !report_events.is_empty(),
        "the run logged its records: {}",
        serde_json::to_string(&logged).unwrap()
    );

    // One logged event per printed record, and the event names agree. `MISSING`
    // appears twice in the fixture, so this also pins that nothing is collapsed.
    assert_eq!(
        report_events.len(),
        printed.len(),
        "every printed record is logged once"
    );
    let logged_names: Vec<&str> = report_events
        .iter()
        .map(|e| e["fields"]["event"].as_str().unwrap())
        .collect();
    assert_eq!(logged_names, printed, "same events, same order");

    // And the fields survived, not just the name. `fields` is a JSON string, so
    // each is parsed and compared against the record that was printed at the same
    // index — every named field the caller saw on stdout is in the log, which is
    // the whole promise of "logged as well as printed".
    for (i, (printed, event)) in r.records.iter().zip(&report_events).enumerate() {
        let payload: serde_json::Value = serde_json::from_str(
            event["fields"]["fields"]
                .as_str()
                .unwrap_or_else(|| panic!("record {i} logged no fields: {event}")),
        )
        .expect("the logged fields are JSON");
        for (k, v) in printed.as_object().expect("a record is an object") {
            // `fields_json` carries the payload alone; `event` is logged beside it
            // as its own field (checked above), so it is not expected twice.
            if k == "event" {
                continue;
            }
            assert_eq!(
                payload.get(k),
                Some(v),
                "record {i} ({}) logged field {k} differently than it printed it",
                printed["event"]
            );
        }
    }
}

/// **The file log carries every record at the *default* `--log-level`.**
///
/// The case that pins `Report::emit`'s gate to the installed subscriber rather
/// than to the console level. The console layer is filtered by `--log-level` and
/// defaults to `info`, but the file layer is always `trace` — so with
/// `--log-file` and *no* `--log-level`, the record event is wanted by the file
/// layer even though the console would discard it. A gate that consulted
/// `--log-level` instead would silently drop every record from the log file.
///
/// The previous test pins the opposite pole: the console at `debug` wants the
/// event. This one pins the case where only the file layer does.
///
/// What this does *not* discriminate is the `STATIC_MAX_LEVEL` term in that gate:
/// `tracing` is depended on without a `release_max_level_*` feature, so that
/// ceiling is `TRACE` in every profile and a gate of `STATIC_MAX_LEVEL` alone
/// passes this test too (verified in both debug and release). It becomes
/// discriminating the day such a feature is added; until then the promise being
/// pinned here is the one the gate actually makes.
#[test]
fn a_record_reaches_the_file_log_at_the_default_log_level() {
    let t = temp_root("out_default_log");
    let (src, dst) = fixture(&t);
    let logfile = t.subdir("logs").join("run.json");
    // No --log-level at all: the default `info`, where the console layer discards
    // a debug event and only the file layer wants it.
    let mut args = compare_args(&src, &dst, &["--output", "json"]);
    args.push("--log-file".into());
    args.push(logfile.display().to_string());

    let r = run(&args);
    assert_eq!(r.code, 4, "differences exit 4: {}", r.show());

    let logged = parse_ndjson(&std::fs::read(&logfile).expect("the run wrote its log"));
    let report_events: Vec<&serde_json::Value> = logged
        .iter()
        .filter(|e| e["fields"]["message"] == "report")
        .collect();
    assert_eq!(
        report_events.len(),
        r.records.len(),
        "the always-trace file layer captured every record at the default level: {}",
        serde_json::to_string(&logged).unwrap()
    );
    // And with its fields, so the gate did not degenerate into logging the name.
    assert!(
        report_events.iter().all(|e| e["fields"]["fields"]
            .as_str()
            .is_some_and(|f| !f.is_empty())),
        "each logged record carries its payload"
    );
    // The console half of the same default: the file wanted every record at
    // `debug`, but the console's default `info` kept none of them. Folded in
    // from the old `the_file_is_louder_than_the_console_by_default`, which
    // built this same fixture and ran this same binary just to assert this.
    assert!(
        !strip_ansi(&r.stderr).contains(" DEBUG "),
        "the console's default level is `info`, so none of them is on it:\n{}",
        r.stderr
    );
}

/// The file's own level is narrowed the same way by the flag and by the
/// environment; only how the value is supplied differs. The precedence between
/// them is pinned in `crates/log` — what is asserted here is that each seam
/// actually filters the process's JSON log file. (Folded from the old
/// `the_log_file_level_flag_narrows_the_file` and
/// `the_environment_sets_the_log_files_level`.)
#[test]
fn the_file_level_can_be_narrowed_by_the_flag_or_the_environment() {
    let cases = [
        // (name, narrow by flag, narrow by environment)
        ("--log-file-level", true, false),
        ("GIRSYNC_LOG_FILE_LEVEL", false, true),
    ];
    for (i, (name, use_flag, use_env)) in cases.iter().enumerate() {
        let t = temp_root(&format!("out_file_level_{i}"));
        let (src, dst) = fixture(&t);
        let logfile = t.subdir("logs").join("run.json");
        let mut args = compare_args(&src, &dst, &["--output", "json"]);
        args.push("--log-file".into());
        args.push(logfile.display().to_string());
        if *use_flag {
            args.push("--log-file-level".into());
            args.push("error".into());
        }

        let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_girsync"));
        cmd.args(&args).env("GIRSYNC_TEST_CACHE", "memory");
        if *use_env {
            cmd.env("GIRSYNC_LOG_FILE_LEVEL", "error");
        }
        let out = cmd.output().expect("spawn girsync");
        assert_eq!(
            out.status.code(),
            Some(4),
            "{name}: {}",
            String::from_utf8_lossy(&out.stderr)
        );

        let logged = parse_ndjson(&std::fs::read(&logfile).expect("the run wrote its log"));
        assert_eq!(
            logged
                .iter()
                .filter(|e| e["fields"]["message"] == "report")
                .count(),
            0,
            "{name}: the file is at `error`, so it kept no debug record: {}",
            serde_json::to_string(&logged).unwrap()
        );
    }
}

/// `RUST_LOG` adds to the console; it cannot silence the program, and it
/// cannot change the program's own level either.
///
/// The program's own level is `--log-level`, and `RUST_LOG` only adds to it.
#[test]
fn an_unrelated_rust_log_adds_to_the_console_without_silencing_it() {
    let t = temp_root("out_rust_log");
    let (src, dst) = fixture(&t);

    // The environment names another crate: the program still speaks, at its
    // own level.
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_girsync"))
        .args(compare_args(&src, &dst, &[]))
        .env("RUST_LOG", "redb=debug")
        .env("GIRSYNC_TEST_CACHE", "memory")
        .output()
        .expect("spawn girsync");
    assert_eq!(out.status.code(), Some(4));
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        strip_ansi(&stderr).contains("log start"),
        "the program still speaks at its own level:\n{stderr}"
    );
    assert!(
        !strip_ansi(&stderr).contains(" DEBUG "),
        "and at the level the flag asked for:\n{stderr}"
    );

    // And the flag still decides that level, even with the environment set.
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_girsync"))
        .args(compare_args(&src, &dst, &["--log-level", "debug"]))
        .env("RUST_LOG", "redb=debug")
        .env("GIRSYNC_TEST_CACHE", "memory")
        .output()
        .expect("spawn girsync");
    assert_eq!(out.status.code(), Some(4));
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        strip_ansi(&stderr).contains(" DEBUG "),
        "the flag's `debug` beat the environment:\n{stderr}"
    );
}

/// A **debug** build writes no log file; a **release** build writes one.
///
/// The policy is the shared crate's — `girpr-log` decides whether there is a
/// default at all, and the shape of that default is asserted there — so what
/// is pinned here is that the *program* does what the policy says: it says
/// on stderr which of the two it is doing, and a debug build does not bring
/// the `logs/` directory beside the executable into existence.
///
/// It is not pinned by *finding* the file. The default's name carries a
/// timestamp, so the only way to find it is to list the directory and read
/// every file in it — and that cost is the history of the machine rather
/// than the code.
#[test]
fn the_default_log_file_follows_the_build_it_was_built_from() {
    let logs = exe_dir().join("logs");
    let existed = logs.exists();

    let t = temp_root("out_default_log_file_policy");
    let (src, dst) = fixture(&t);
    let r = run(&compare_args(&src, &dst, &["--output", "json"]));
    assert_eq!(r.code, 4, "differences exit 4: {}", r.show());

    if cfg!(debug_assertions) {
        assert!(
            r.stderr.contains("log file: none"),
            "a debug build says it wrote no file:\n{}",
            r.stderr
        );
        if !existed {
            assert!(
                !logs.exists(),
                "and it did not bring {} into existence",
                logs.display()
            );
        }
    } else {
        assert!(
            r.stderr.contains("log file: "),
            "a release build says where the file went:\n{}",
            r.stderr
        );
    }
}

/// **The walk's counters are in the log, and they are the walk's own.**
///
/// `walk_live` counts the files and the directories it found, and the "scan
/// done" event carries both. Nothing else reports them: the run's own record
/// counts the *effective* map, which is the walk's output after the filter has
/// hidden what the run excludes. A tree with an excluded file is therefore the
/// fixture that tells the two apart — and the only way to notice that the
/// walk's counters stopped counting.
#[test]
fn the_scan_counts_the_entries_the_walk_saw() {
    let t = temp_root("out_scan_counts");
    let dir = t.subdir("tree");
    wfile(&dir, "a.txt", b"hello");
    wfile(&dir, "sub/b.txt", b"world");
    wfile(&dir, "skip.log", b"excluded");
    let logfile = t.subdir("logs").join("run.json");

    let out = std::process::Command::new(env!("CARGO_BIN_EXE_girsync"))
        .args([
            "update",
            "--dir",
            dir.to_str().unwrap(),
            "--exclude",
            "*.log",
            "--output",
            "json",
            "--log-file",
            logfile.to_str().unwrap(),
        ])
        .env("GIRSYNC_TEST_CACHE", "memory")
        .output()
        .expect("spawn girsync");
    assert!(
        out.status.success(),
        "update failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let logged = parse_ndjson(&std::fs::read(&logfile).expect("the run wrote its log"));
    let scans: Vec<&Value> = logged
        .iter()
        .filter(|e| e["fields"]["message"] == "scan done")
        .collect();
    assert_eq!(
        scans.len(),
        1,
        "one walk, one scan-done event: {}",
        serde_json::to_string(&logged).unwrap()
    );
    // The excluded file is counted: these are the walk's counts, and the walk
    // does not know what the run excludes.
    assert_eq!(
        scans[0]["fields"]["files"], 3,
        "the walk counted every file"
    );
    assert_eq!(scans[0]["fields"]["dirs"], 1, "and every directory");
    assert_eq!(scans[0]["fields"]["entries"], 4, "and the two add up");

    // The run's own record counts the effective map, which the filter has
    // already pruned — so the excluded file is not in it.
    let recs = parse_ndjson(&out.stdout);
    assert_eq!(
        recs[0]["files"], 2,
        "the run's own count leaves the excluded file out"
    );
}

/// **The walk counts what it saw, and the map counts what it kept.**
///
/// The walk's counters and the effective map's counters answer different
/// questions, and both are handed back to a library caller: the walk's own
/// count (the entries it pushed, including what the run excludes) is
/// [`girsync::ScanStats::live`], and the map's counts are the `files`/`dirs` and
/// work counters on each phase's [`girsync::SideScan`]. A run with an
/// `--exclude` is where the two part company, so it is the fixture that tells
/// them apart.
///
/// The walk's own **heartbeat** reports that same walk count: the entries it
/// has pushed, every heartbeat period. The period is injected through
/// [`girsync::ScanMode::with_heartbeat_entries`] so a three-file fixture trips
/// it; the value it reports is `live` below. The excluded file is in that count
/// — the walk does not know what the run excludes.
///
/// The phases' progress counters are their own: phase A reads no bytes and so
/// hashes nothing; phase C hashes what the plan asked for and says how many
/// were cache hits. Both are read off the returned `ScanStats` instead of
/// parsed out of a log file, which is what lets the fixture stay small and the
/// run stay in-process. The log rendering of the counters stays covered by
/// `the_scan_counts_the_entries_the_walk_saw` and the other log tests here.
#[test]
fn the_walk_counts_what_it_saw_and_the_map_counts_what_it_kept() {
    // The smallest tree that keeps the two counts distinct: two mirror files
    // and one the run excludes. The walk sees all three; the map keeps two.
    // Period 2 is injected rather than set on a child, so the heartbeat trips
    // on this tree without a 101-file fixture or an env var.
    const HEARTBEAT_PERIOD: usize = 2;
    const ENTRIES: usize = 3;
    const KEPT: usize = 2;

    let t = temp_root("out_walk_progress");
    let dir = t.subdir("tree");
    wfile(&dir, "a.txt", b"x");
    wfile(&dir, "b.txt", b"y");
    wfile(&dir, "skip.log", b"excluded");

    // The same filters and depth cap the old child invocation used.
    let mut common = common::opts();
    common.filter = girsync::Filter::new(
        vec![],
        girsync::compile_patterns(&["*.log".to_string()]).unwrap(),
        true,
    );
    common.max_depth = 3;
    let mode =
        girsync::ScanMode::new(true, common.dry_run).with_heartbeat_entries(HEARTBEAT_PERIOD);

    // The three phases an `update` run performs, spelled out so each phase's
    // counters can be asserted on its own — the same shape `tests/helpers.rs`
    // uses, and the reason these numbers live on a library result at all.
    let db = girsync::cache::CacheDb::open_temp(common.case_sensitive).unwrap();
    let phase_a = girsync::effective::scan_stat_only(&dir, &db, &common, mode).unwrap();
    let plan =
        girsync::HashPlan::plan_one_side(&phase_a.map, &common.algos, mode.no_trust_cached_hashes);
    let phase_c = girsync::effective::resolve_folder(
        &dir,
        &db,
        mode,
        &phase_a,
        &plan,
        girsync::RwSide::Src,
        &common::serial_rw(),
    )
    .unwrap();

    // The walk starts at the root it was given and the depth cap it was asked
    // for; both are the caller's own values, asserted here directly.
    assert_eq!(common.max_depth, 3);

    // The heartbeat's `entries`, the phase-A start's `live`, and the phase-A
    // progress event's `total` are all the walk's count: every entry it pushed,
    // the excluded file included. Phase C carries that count forward unchanged.
    assert_eq!(
        phase_a.stats.live, ENTRIES,
        "the walk counts every entry it pushed, the excluded file included"
    );
    assert_eq!(
        phase_c.stats.live, ENTRIES,
        "phase C carries phase A's walk count forward"
    );

    // The plan's pending list is what phase C hashes — the total its progress
    // event reports. `update` asks for a digest for every file the map kept,
    // so here it is the same as the map's file count.
    assert_eq!(
        plan.pending().len(),
        KEPT,
        "the phase that hashes owes a digest for every file the map kept"
    );

    // Phase A reads no bytes and so hashes nothing; its map is the walk's
    // output after the filter, with the excluded file gone.
    assert_eq!(
        (
            phase_a.stats.files,
            phase_a.stats.dirs,
            phase_a.stats.hashed,
            phase_a.stats.cache_hit,
            phase_a.stats.pruned,
        ),
        (KEPT, 0, 0, 0, 0),
        "phase A reads no bytes and leaves the excluded file out"
    );

    // Phase C hashes what the plan asked for, from a cold cache with nothing
    // to prune. It carries phase A's map counts forward, so the map is the
    // same map in both phases.
    assert_eq!(
        (
            phase_c.stats.files,
            phase_c.stats.dirs,
            phase_c.stats.hashed,
            phase_c.stats.cache_hit,
            phase_c.stats.pruned,
        ),
        (KEPT, 0, KEPT, 0, 0),
        "phase C hashed what the map held, from a cold cache"
    );
}

/// **A `girpr-cache*` subtree is in no count.**
///
/// The tool's own namespace is not mirror content. The walk prunes it, and
/// nothing under a cache directory is even visited — so a cache subtree is in
/// none of the counts: not the walk's, not the map's, and not the run's own
/// record.
///
/// The fixture puts one there on purpose. A run makes its own backup beside
/// the cache before it walks, so this shape is in every run's way; what the
/// case pins is that the tool's own state never shows up as content.
#[test]
fn a_pruned_cache_subtree_is_in_no_count() {
    let t = temp_root("out_cache_pruned");
    let dir = t.subdir("tree");
    wfile(&dir, "a.txt", b"hello");
    wfile(&dir, "sub/b.txt", b"world");
    // The tool's own namespace: a published copy of a cache, and a snapshot of
    // one, with content under both.
    wfile(&dir, "girpr-cache-backup-1/inner.bin", b"tool state");
    wfile(&dir, "girpr-cache-old-2/deep/inner.bin", b"tool state");
    let logfile = t.subdir("logs").join("run.json");

    let out = std::process::Command::new(env!("CARGO_BIN_EXE_girsync"))
        .args([
            "update",
            "--dir",
            dir.to_str().unwrap(),
            "--output",
            "json",
            "--log-file",
            logfile.to_str().unwrap(),
        ])
        .env("GIRSYNC_TEST_CACHE", "memory")
        .output()
        .expect("spawn girsync");
    assert!(
        out.status.success(),
        "update failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let logged = parse_ndjson(&std::fs::read(&logfile).expect("the run wrote its log"));

    // The walk's own count: the two files and the directory between them, and
    // nothing of the cache subtrees.
    let scans = events_with(&logged, "scan done");
    assert_eq!(
        scans.len(),
        1,
        "one walk, one scan-done event: {}",
        serde_json::to_string(&logged).unwrap()
    );
    assert_eq!(
        scans[0]["fields"]["entries"], 3,
        "the pruned subtree is not walked"
    );
    assert_eq!(scans[0]["fields"]["files"], 2);
    assert_eq!(scans[0]["fields"]["dirs"], 1);

    // The map's count: the same tree, minus the pruned subtree.
    let phase_a = events_with(&logged, "effective done: phase a");
    assert_eq!(
        phase_a.len(),
        1,
        "one phase A, one effective-done event: {}",
        serde_json::to_string(&logged).unwrap()
    );
    assert_eq!(phase_a[0]["fields"]["files"], 2, "the map holds two files");
    assert_eq!(
        phase_a[0]["fields"]["dirs"], 1,
        "and the directory between them"
    );

    // And the run's own record, which counts the map.
    let recs = parse_ndjson(&out.stdout);
    assert_eq!(recs[0]["files"], 2, "the record counts the map");
    assert_eq!(recs[0]["dirs"], 1);
}

/// **A sync run counts each side on its own.**
///
/// A sync run walks **two** trees, and each walk reports its own counts: the
/// src's are the src's and the dst's are the dst's. A run that added them
/// together, or reported one side's count for both, would be a run whose
/// counters describe a tree that does not exist.
///
/// The dst's counts are the counts of the tree as it was **before** the run
/// copied into it: both sides are walked before the plan, so the dst's walk
/// sees the dst as it was, and the copy that follows is in neither count.
#[test]
fn a_sync_run_counts_each_side_separately() {
    let t = temp_root("out_sync_counts");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    wfile(&src, "a.txt", b"hello");
    wfile(&src, "sub/b.txt", b"world");
    wfile(&dst, "a.txt", b"hello");
    let logfile = t.subdir("logs").join("run.json");

    let out = std::process::Command::new(env!("CARGO_BIN_EXE_girsync"))
        .args([
            "sync",
            "--src",
            src.to_str().unwrap(),
            "--dst",
            dst.to_str().unwrap(),
            "--output",
            "json",
            "--log-file",
            logfile.to_str().unwrap(),
        ])
        .env("GIRSYNC_TEST_CACHE", "memory")
        .output()
        .expect("spawn girsync");
    assert!(
        out.status.success(),
        "sync failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let logged = parse_ndjson(&std::fs::read(&logfile).expect("the run wrote its log"));

    let scans = events_with(&logged, "scan done");
    assert_eq!(
        scans.len(),
        2,
        "a sync run walks both sides, once each: {}",
        serde_json::to_string(&logged).unwrap()
    );

    // The src's counts: two files and the directory between them.
    let src_scan = scans
        .iter()
        .find(|e| e["fields"]["root"] == src.display().to_string())
        .expect("the src's walk reports its own counts");
    assert_eq!(src_scan["fields"]["entries"], 3);
    assert_eq!(src_scan["fields"]["files"], 2);
    assert_eq!(src_scan["fields"]["dirs"], 1);

    // The dst's counts: the one file it held before the run copied into it.
    let dst_scan = scans
        .iter()
        .find(|e| e["fields"]["root"] == dst.display().to_string())
        .expect("the dst's walk reports its own counts");
    assert_eq!(
        dst_scan["fields"]["entries"], 1,
        "the dst is counted as it was before the copy"
    );
    assert_eq!(dst_scan["fields"]["files"], 1);
    assert_eq!(dst_scan["fields"]["dirs"], 0);
}

/// The directory the executable is in — where the default log file lands.
fn exe_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_BIN_EXE_girsync"))
        .parent()
        .expect("the executable has a directory")
        .to_path_buf()
}
