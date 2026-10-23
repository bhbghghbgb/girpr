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
}

/// **The console is `info` and the log file is `debug`, by default.**
///
/// The two levels are separate knobs and the defaults are one step apart: the
/// console is for the person watching the run, the file for the person reading
/// it afterwards. The report events are `debug`, so they are the evidence —
/// they are in the file and not on the console.
///
/// This is the same property the test above pins from the other side (the
/// file wants them), read here as the *difference* between the two sinks:
/// a console at `info` must not carry them, or the two sinks would be one
/// sink with two names.
#[test]
fn the_file_is_louder_than_the_console_by_default() {
    let t = temp_root("out_file_vs_console");
    let (src, dst) = fixture(&t);
    let logfile = t.subdir("logs").join("run.json");
    let mut args = compare_args(&src, &dst, &["--output", "json"]);
    args.push("--log-file".into());
    args.push(logfile.display().to_string());

    let r = run(&args);
    assert_eq!(r.code, 4, "differences exit 4: {}", r.show());

    let logged = parse_ndjson(&std::fs::read(&logfile).expect("the run wrote its log"));
    let report_events = logged
        .iter()
        .filter(|e| e["fields"]["message"] == "report")
        .count();
    assert_eq!(
        report_events,
        r.records.len(),
        "the file's default level is `debug`, so every record is in it"
    );
    assert!(
        !strip_ansi(&r.stderr).contains(" DEBUG "),
        "the console's default level is `info`, so none of them is on it:\n{}",
        r.stderr
    );
}

/// `--log-file-level` narrows the log file.
///
/// The flag is the file's own level, and it narrows *only* the file: the
/// console keeps its own. A file at `error` is the record of what failed and
/// nothing else.
#[test]
fn the_log_file_level_flag_narrows_the_file() {
    let t = temp_root("out_file_level_flag");
    let (src, dst) = fixture(&t);
    let logfile = t.subdir("logs").join("run.json");
    let mut args = compare_args(&src, &dst, &["--output", "json"]);
    args.push("--log-file".into());
    args.push(logfile.display().to_string());
    args.push("--log-file-level".into());
    args.push("error".into());

    let r = run(&args);
    assert_eq!(r.code, 4, "differences exit 4: {}", r.show());

    let logged = parse_ndjson(&std::fs::read(&logfile).expect("the run wrote its log"));
    assert_eq!(
        logged
            .iter()
            .filter(|e| e["fields"]["message"] == "report")
            .count(),
        0,
        "the file is at `error`, so it kept no debug record: {}",
        serde_json::to_string(&logged).unwrap()
    );
}

/// The environment sets the log file's level, when the flag does not.
///
/// The one seam of the policy the flag table cannot show: the flag wins, the
/// environment answers for a flag that was never given, and `debug` — the
/// default — answers for both.
#[test]
fn the_environment_sets_the_log_files_level() {
    let t = temp_root("out_env_log_file_level");
    let (src, dst) = fixture(&t);
    let logfile = t.subdir("logs").join("run.json");
    let mut args = compare_args(&src, &dst, &["--output", "json"]);
    args.push("--log-file".into());
    args.push(logfile.display().to_string());

    let out = std::process::Command::new(env!("CARGO_BIN_EXE_girsync"))
        .args(&args)
        .env("GIRSYNC_LOG_FILE_LEVEL", "error")
        .output()
        .expect("spawn girsync");
    assert_eq!(out.status.code(), Some(4));

    let logged = parse_ndjson(&std::fs::read(&logfile).expect("the run wrote its log"));
    assert_eq!(
        logged
            .iter()
            .filter(|e| e["fields"]["message"] == "report")
            .count(),
        0,
        "the file is at `error`, so it kept no debug record: {}",
        serde_json::to_string(&logged).unwrap()
    );
}

/// `RUST_LOG` adds to the console; it cannot silence the program, and it
/// cannot change the program's own level either.
///
/// A `RUST_LOG` naming an unrelated crate used to replace the console's whole
/// spec, and the program then said nothing at all. The program's own level is
/// `--log-level`, and `RUST_LOG` only adds to it.
#[test]
fn an_unrelated_rust_log_adds_to_the_console_without_silencing_it() {
    let t = temp_root("out_rust_log");
    let (src, dst) = fixture(&t);

    // The environment names another crate: the program still speaks, at its
    // own level.
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_girsync"))
        .args(compare_args(&src, &dst, &[]))
        .env("RUST_LOG", "redb=debug")
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

/// `--no-log-file` turns the file off, and it beats the environment.
///
/// "Write no log file" is the more specific thing to have said, so it wins
/// over a path the shell happened to export. The file is not written at
/// all — which is the whole point, and which is why the assertion is
/// `exists()` on a known path rather than a search of a directory.
#[test]
fn no_log_file_turns_the_file_off_even_when_the_environment_names_one() {
    let t = temp_root("out_no_log_file");
    let (src, dst) = fixture(&t);
    let logfile = t.subdir("logs").join("from-env.json");

    let out = std::process::Command::new(env!("CARGO_BIN_EXE_girsync"))
        .args(compare_args(&src, &dst, &["--no-log-file"]))
        .env("GIRSYNC_LOG_FILE", &logfile)
        .output()
        .expect("spawn girsync");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(out.status.code(), Some(4), "{stderr}");
    assert!(
        !logfile.exists(),
        "no log file was written: {}",
        logfile.display()
    );
    assert!(
        !stderr.contains(&format!("log file: {}", logfile.display())),
        "and the path was not announced:\n{stderr}"
    );
}

/// `--no-log-file` and `--log-file` say opposite things, so they are a usage
/// error rather than one of them silently winning.
#[test]
fn no_log_file_and_log_file_together_are_a_usage_error() {
    let t = temp_root("out_no_log_file_conflict");
    let (src, dst) = fixture(&t);
    let logfile = t.subdir("logs").join("both.json");

    let out = std::process::Command::new(env!("CARGO_BIN_EXE_girsync"))
        .args(compare_args(
            &src,
            &dst,
            &["--no-log-file", "--log-file", logfile.to_str().unwrap()],
        ))
        .output()
        .expect("spawn girsync");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    // clap's own exit code for an argument error, which is what `girsync`
    // uses: it parses with `Cli::parse()`, so a bad flag is clap's `2` and
    // not the `1` the repair binary remaps it to.
    assert_eq!(out.status.code(), Some(2), "{stderr}");
    assert!(
        stderr.contains("cannot be used with"),
        "the conflict is reported:\n{stderr}"
    );
    assert!(!logfile.exists(), "and no file was written");
}

/// The environment variable names the log file when the flag does not.
///
/// Same precedence as everywhere else in the workspace: the flag wins, the
/// environment answers for a flag that was never given, and the timestamped
/// default answers for both.
#[test]
fn the_environment_names_the_log_file_when_the_flag_does_not() {
    let t = temp_root("out_env_log_file");
    let (src, dst) = fixture(&t);
    let logfile = t.subdir("logs").join("from-env.json");

    let out = std::process::Command::new(env!("CARGO_BIN_EXE_girsync"))
        .args(compare_args(&src, &dst, &[]))
        .env("GIRSYNC_LOG_FILE", &logfile)
        .output()
        .expect("spawn girsync");
    assert_eq!(out.status.code(), Some(4));
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        stderr.contains(&format!("log file: {}", logfile.display())),
        "the path is echoed on stderr, because the caller named it:\n{stderr}"
    );
    let text = std::fs::read_to_string(&logfile).expect("the environment named the file");
    assert!(
        text.contains("\"log start\""),
        "and it holds the run's own records:\n{text}"
    );
}

/// The directory the executable is in — where the default log file lands.
fn exe_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_BIN_EXE_girsync"))
        .parent()
        .expect("the executable has a directory")
        .to_path_buf()
}
