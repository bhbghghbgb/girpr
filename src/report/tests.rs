use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::hyp::{ChunkBuild, GameBranchPackage, GameConfig};
use crate::report::emit::{progress_record, report_record, summary_record};
use crate::report::record::{OutputFormat, Record};
use crate::report::summary::{Extra, Summary, SummaryFlags, conditional_extras};

/// The two planes are two renderings of one field list. This is the
/// property the whole module exists to guarantee, so it is pinned on a
/// record with a string field, a number, and a quoted value.
#[test]
fn text_and_json_are_two_renderings_of_one_record() {
    let rec = Record::keyed("SUMMARY")
        .put("total", 4u64)
        .put("word", "a b\"c");
    assert_eq!(rec.text(), "SUMMARY total=4 word=a b\"c");
    assert_eq!(
        rec.json(),
        r#"{"event":"summary","total":4,"word":"a b\"c"}"#,
        "the event name is the label lowercased, and the order is report order"
    );
    assert_eq!(rec.event(), "summary");
    assert_eq!(rec.label(), "SUMMARY");
    assert_eq!(
        rec.fields().iter().map(|(k, _)| *k).collect::<Vec<_>>(),
        vec!["total", "word"],
        "field order is report order, which is what both renderings keep"
    );
    // One record, one line, whichever plane is active.
    assert_eq!(rec.render(OutputFormat::Text), rec.text());
    assert_eq!(rec.render(OutputFormat::Json), rec.json());
    assert_eq!(OutputFormat::default(), OutputFormat::Text);
    assert_eq!(OutputFormat::Json.as_str(), "json");
}

#[test]
fn a_saturated_estimate_renders_as_an_unsigned_number_not_a_negative_one() {
    // The plan's aggregates saturate at `u64::MAX`. Rendering that through
    // an `i64` would print `-1`: a byte total with the wrong sign, on both
    // planes. The value is unsigned, so it must print as one.
    let rec = Record::keyed("PLAN").put("est_write_bytes", u64::MAX);
    assert_eq!(
        rec.text(),
        format!("PLAN est_write_bytes={}", u64::MAX),
        "a saturated estimate is the largest unsigned number, not -1"
    );
    assert_eq!(
        rec.json(),
        format!(r#"{{"event":"plan","est_write_bytes":{}}}"#, u64::MAX)
    );
}

#[test]
fn progress_record_counts_done_as_the_three_outcomes() {
    let s = Summary {
        files_total: 10,
        files_skipped: AtomicU64::new(3),
        files_repaired: AtomicU64::new(2),
        files_failed: AtomicU64::new(1),
        ..Summary::default()
    };
    s.download.settle(123);
    let rec = progress_record(&s);
    assert_eq!(
        rec.text(),
        "PROGRESS done=6 total=10 skipped=3 repaired=2 failed=1 download_bytes=123"
    );
    // `done` and `total` are two numbers on both planes. They are separate
    // fields rather than one pre-joined `6/10` string, because a pre-joined
    // string cannot be a JSON number.
    let v: serde_json::Value = serde_json::from_str(&rec.json()).unwrap();
    assert_eq!(v["event"], "progress");
    assert_eq!(v["done"], 6);
    assert_eq!(v["total"], 10);
}

#[test]
fn summary_line_keyvalue_and_json_agree() {
    let s = Summary {
        files_total: 4,
        files_skipped: AtomicU64::new(1),
        files_repaired: AtomicU64::new(2),
        files_failed: AtomicU64::new(1),
        deleted_extra_bytes: AtomicU64::new(20),
        flags: SummaryFlags::default(),
        ..Summary::default()
    };
    s.download.settle(10);
    let rec = summary_record(&s, 3);
    assert_eq!(
        rec.text(),
        "SUMMARY total=4 skipped=1 repaired=2 failed=1 download_bytes=10 deleted_extra_bytes=20 exit=3"
    );
    let json = rec.json();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(v["event"], "summary");
    assert_eq!(v["total"], 4);
    assert_eq!(v["skipped"], 1);
    assert_eq!(v["repaired"], 2);
    assert_eq!(v["failed"], 1);
    assert_eq!(v["download_bytes"], 10);
    assert_eq!(v["deleted_extra_bytes"], 20);
    assert_eq!(v["exit"], 3);
    // An unflagged run gains no members at all — that is the whole
    // contract of the conditional keys.
    assert_eq!(v.as_object().unwrap().len(), 8, "{json}");
}

#[test]
fn conditional_keys_appear_only_when_their_flag_is_active() {
    let mut s = Summary {
        files_total: 8,
        ..Summary::default()
    };
    s.flags.scoped = true;
    let rec = summary_record(&s, 0);
    let line = rec.text();
    for k in ["scoped=1", "config_bumped=0", "plan_files=8"] {
        assert!(line.contains(k), "{k} missing from {line}");
    }
    assert!(!line.contains("limited"), "{line}");
    assert!(!line.contains("plan_download_bytes"), "{line}");
    assert!(!line.contains("write_bytes"), "{line}");
    assert!(!line.contains("stopped"), "{line}");
    // Fixed fields are untouched; only a tail is appended.
    assert!(line.starts_with("SUMMARY total=8 skipped=0 repaired=0 failed=0 download_bytes=0 deleted_extra_bytes=0 exit=0 "));

    // The same run as JSON: same keys, right types.
    let v: serde_json::Value = serde_json::from_str(&rec.json()).unwrap();
    assert_eq!(v["scoped"], 1);
    assert_eq!(v["config_bumped"], 0);
    assert_eq!(v["plan_files"], 8);
    assert_eq!(v.as_object().unwrap().len(), 11);
}

#[test]
fn every_conditional_key_has_a_known_name_and_typed_rendering() {
    let mut s = Summary {
        files_total: 3,
        ..Summary::default()
    };
    s.flags = SummaryFlags {
        limited: true,
        scoped: true,
        plan_download_bytes: Some(4242),
        download_bytes_spent: Some(4300),
        write_bytes: Some(77),
        write_bytes_spent: Some(96),
        stopped: Some("--max-write-bytes"),
        config_bumped_zero: false,
        local_manifest: Some("unavailable"),
        repair_mode: Some("inplace"),
        files_inplace: Some(3),
        files_temp: Some(1),
        kept_bytes: Some(900),
        ram_bytes: Some(4096),
        damaged_chunks: Some(7),
    };
    let rec = summary_record(&s, 5);
    assert_eq!(
        rec.text(),
        "SUMMARY total=3 skipped=0 repaired=0 failed=0 download_bytes=0 deleted_extra_bytes=0 \
             exit=5 limited=1 scoped=1 config_bumped=0 plan_files=3 plan_download_bytes=4242 \
             download_bytes_spent=4300 write_bytes=77 write_bytes_spent=96 \
             repair_mode=inplace inplace=3 temp=1 kept_bytes=900 ram_bytes=4096 \
             damaged_chunks=7 stopped=--max-write-bytes local_manifest=unavailable"
    );
    // `stopped` and `local_manifest` are words, not numbers: the JSON plane
    // must say so.
    let v: serde_json::Value = serde_json::from_str(&rec.json()).unwrap();
    assert_eq!(v["stopped"], "--max-write-bytes");
    assert_eq!(v["write_bytes"], 77);
    assert_eq!(v["write_bytes_spent"], 96);
    assert_eq!(v["download_bytes_spent"], 4300);
    assert_eq!(v["plan_download_bytes"], 4242);
    assert_eq!(v["local_manifest"], "unavailable");
    assert_eq!(v["repair_mode"], "inplace");
    assert_eq!(v["inplace"], 3);
    assert_eq!(v["temp"], 1);
    assert_eq!(v["kept_bytes"], 900);
    assert_eq!(v["ram_bytes"], 4096);
    assert_eq!(v["damaged_chunks"], 7);
    // Key order is fixed, so two runs' lines diff cleanly.
    let keys: Vec<&str> = conditional_extras(&s).iter().map(Extra::key).collect();
    assert_eq!(
        keys,
        vec![
            "limited",
            "scoped",
            "config_bumped",
            "plan_files",
            "plan_download_bytes",
            "download_bytes_spent",
            "write_bytes",
            "write_bytes_spent",
            "repair_mode",
            "inplace",
            "temp",
            "kept_bytes",
            "ram_bytes",
            "damaged_chunks",
            "stopped",
            "local_manifest"
        ]
    );
}

/// `local_manifest` is about the run, not a verdict on it, so it must not
/// appear when no local version was ever known — and must not drag
/// `limited`/`scoped`/`plan_files` along with it.
#[test]
fn local_manifest_stands_alone() {
    for value in ["ok", "unavailable"] {
        let mut s = Summary {
            files_total: 2,
            ..Summary::default()
        };
        s.flags.local_manifest = Some(value);
        let rec = summary_record(&s, 0);
        let line = rec.text();
        assert!(line.contains(&format!("local_manifest={value}")), "{line}");
        assert!(!line.contains("limited"), "{line}");
        assert!(!line.contains("scoped"), "{line}");
        assert!(!line.contains("plan_files"), "{line}");
        assert!(!line.contains("config_bumped"), "{line}");
        let v: serde_json::Value = serde_json::from_str(&rec.json()).unwrap();
        assert_eq!(v["local_manifest"], value);
    }
    // Absent when there was no local version to look up.
    let s = Summary {
        files_total: 2,
        ..Summary::default()
    };
    let rec = summary_record(&s, 0);
    assert!(!rec.text().contains("local_manifest"));
    let v: serde_json::Value = serde_json::from_str(&rec.json()).unwrap();
    assert!(v.get("local_manifest").is_none(), "{rec:?}");
}

#[test]
fn snapshot_copies_every_counter() {
    let s = Summary {
        files_total: 7,
        files_skipped: AtomicU64::new(1),
        ..Summary::default()
    };
    s.download.settle(9);
    let snap = s.snapshot();
    assert_eq!(snap.files_total, 7);
    assert_eq!(snap.files_skipped.load(Ordering::Relaxed), 1);
    assert_eq!(snap.download.settled(), 9);
    assert_eq!(snap.flags, SummaryFlags::default());
    // Mutating the source afterwards must not touch the snapshot.
    s.files_skipped.store(5, Ordering::Relaxed);
    assert_eq!(snap.files_skipped.load(Ordering::Relaxed), 1);
}

fn cfg_full() -> GameConfig {
    GameConfig {
        exe_file_name: "YuanShen.exe".into(),
        audio_pkg_scan_dir: "scan.txt".into(),
        audio_pkg_res_dir: "res".into(),
        audio_pkg_cache_dir: "cache".into(),
        default_download_mode: "DOWNLOAD_MODE_CHUNK".into(),
        res_category_dir: "rc.txt".into(),
        blacklist_dir: "bl.txt".into(),
        enable_resource_blacklist: true,
        game: None,
    }
}

#[test]
fn report_record_reports_versions_and_api_fields() {
    let pkg = GameBranchPackage {
        package_id: "pkg1".into(),
        branch: "main".into(),
        password: String::new(),
        tag: "5.1.0".into(),
        diff_tags: vec!["5.0.0".into(), "5.1.0".into()],
    };
    let build = ChunkBuild {
        build_id: "build42".into(),
        tag: "5.1.0".into(),
        manifests: Vec::new(),
    };
    let audio: HashSet<String> = ["ja-jp".into(), "en-us".into()].into_iter().collect();
    let rec = report_record(
        &Some("4.0.0".to_string()),
        "5.1.0",
        "hk4e_global",
        &cfg_full(),
        &pkg,
        &build,
        &audio,
    );
    let line = rec.text();
    assert!(
        line.starts_with("REPORT local_version=4.0.0 latest_version=5.1.0 biz=hk4e_global"),
        "{line}"
    );
    assert!(line.contains("exe=YuanShen.exe"), "{line}");
    assert!(line.contains("download_mode=DOWNLOAD_MODE_CHUNK"), "{line}");
    assert!(line.contains("branch=main"), "{line}");
    assert!(line.contains("package_id=pkg1"), "{line}");
    assert!(line.contains("build_id=build42"), "{line}");
    assert!(line.contains("audio_langs=en-us,ja-jp"), "{line}");
    assert!(line.contains("diff_tags=5.0.0,5.1.0"), "{line}");
}

#[test]
fn report_record_says_none_for_an_absent_local_version() {
    let cfg = GameConfig {
        exe_file_name: "GenshinImpact.exe".into(),
        ..cfg_full()
    };
    let pkg = GameBranchPackage::default();
    let build = ChunkBuild {
        build_id: "b1".into(),
        tag: "3.0.0".into(),
        manifests: Vec::new(),
    };
    let audio: HashSet<String> = HashSet::new();
    let rec = report_record(&None, "3.0.0", "hk4e_cn", &cfg, &pkg, &build, &audio);
    let v: serde_json::Value = serde_json::from_str(&rec.json()).unwrap();
    assert_eq!(v["event"], "report");
    assert_eq!(v["local_version"], "none");
    assert_eq!(v["latest_version"], "3.0.0");
    assert_eq!(v["biz"], "hk4e_cn");
    assert_eq!(v["exe"], "GenshinImpact.exe");
    assert_eq!(v["audio_langs"], "");
}
