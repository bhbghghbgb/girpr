//! End-to-end cases for the cache <-> NDJSON conversion.
//!
//! Every case builds a real redb cache, moves bytes through the public
//! library functions, and reads a real cache back. The text form is asserted
//! as independent lines so a change that made it depend on a prior line would
//! fail here, which is the property "incrementally parseable" means.

use std::io::Cursor;
use std::path::{Path, PathBuf};

use clap::Parser;
use girpr_tempdir::TempDir;
use girsync_cache::{CacheDb, CacheOpen, FileRec, hash::DigestSet};
use girsync_cache_json::{
    Args, Command, convert::JSON_TEMP_PREFIX, export_cache, import_cache, ndjson,
};

/// A cache with both row shapes and all three digest slots, plus a foreign
/// name and a known name with an unexpected length. Those last two are the
/// rows a naive `HashMap<String, String>` text form would lose.
fn fixture_cache(dir: &TempDir) -> PathBuf {
    let path = dir.join("girpr-cache");
    let cache = girsync_cache::open_db(
        &path,
        true,
        CacheOpen::ReadWrite {
            ignore_cache: false,
            backup_first: false,
            retain: None,
        },
    )
    .unwrap();

    let mut file = FileRec {
        kind: "file".into(),
        size: 42,
        mtime_ns: 123_456_789,
        hashes: DigestSet::default(),
    };
    file.hashes.insert_named("md5", &[1u8; 16]);
    file.hashes.insert_named("sha256", &[2u8; 32]);
    file.hashes.insert_named("xxhash64", &[3u8; 8]);
    cache.put("a.txt", &file).unwrap();

    let mut odd = FileRec {
        kind: "file".into(),
        size: 7,
        mtime_ns: 9,
        hashes: DigestSet::default(),
    };
    odd.hashes.insert_named("blake3", &[7u8; 32]);
    // A known name with the wrong length lands in the foreign arm, exactly
    // where the wire decoder puts it. Its text spelling must survive too.
    odd.hashes.insert_named("md5", &[9u8; 5]);
    cache.put("sub/odd.bin", &odd).unwrap();
    cache.put("sub", &FileRec::dir()).unwrap();
    drop(cache);
    path
}

fn lines(bytes: &[u8]) -> Vec<ndjson::Line> {
    String::from_utf8(bytes.to_vec())
        .unwrap()
        .lines()
        .map(serde_json::from_str)
        .map(Result::unwrap)
        .collect()
}

#[test]
fn export_then_import_preserves_every_row_and_the_meta_mode() {
    let dir = TempDir::new("girsync_cache_json", "roundtrip");
    let src = fixture_cache(&dir);
    let mut out = Vec::new();
    let export_stats = export_cache(&src, &mut out).unwrap();
    let parsed = lines(&out);

    assert_eq!(parsed.len(), 4, "one meta line plus three rows");
    match &parsed[0] {
        ndjson::Line::Meta(m) => {
            assert_eq!(m.version, girsync_cache::CACHE_VERSION);
            assert!(m.case_sensitive);
        }
        other => panic!("meta must come first, got {other:?}"),
    }
    for line in &parsed[1..] {
        assert!(
            matches!(line, ndjson::Line::Entry(_)),
            "every other line is a row"
        );
    }

    let dst = dir.join("imported-cache");
    let stats = import_cache(Cursor::new(out), &dst, false).unwrap();
    assert_eq!(export_stats.entries, 3);
    assert_eq!(stats.entries, 3);
    assert_eq!(stats.files, 2);
    assert_eq!(stats.dirs, 1);

    let before = CacheDb::open_record(&src).unwrap();
    let after = CacheDb::open_record(&dst).unwrap();
    assert_eq!(
        before.meta().unwrap().case_sensitive,
        after.meta().unwrap().case_sensitive
    );
    assert_eq!(before.load_all().unwrap(), after.load_all().unwrap());

    let odd = after.get("sub/odd.bin").unwrap().unwrap();
    assert_eq!(odd.hashes.get_named("blake3"), Some(&[7u8; 32][..]));
    assert_eq!(odd.hashes.get_named("md5"), Some(&[9u8; 5][..]));
}

#[test]
fn an_empty_cache_exports_as_one_line_and_imports_back_empty() {
    let dir = TempDir::new("girsync_cache_json", "empty");
    let src = dir.join("girpr-cache");
    let cache = girsync_cache::open_db(
        &src,
        false,
        CacheOpen::ReadWrite {
            ignore_cache: false,
            backup_first: false,
            retain: None,
        },
    )
    .unwrap();
    drop(cache);

    let mut out = Vec::new();
    export_cache(&src, &mut out).unwrap();
    let parsed = lines(&out);
    assert_eq!(parsed.len(), 1);
    assert!(matches!(&parsed[0], ndjson::Line::Meta(m) if !m.case_sensitive));

    let dst = dir.join("imported-cache");
    let stats = import_cache(Cursor::new(out), &dst, false).unwrap();
    assert_eq!(stats.entries, 0);
    let after = CacheDb::open_record(&dst).unwrap();
    assert!(!after.meta().unwrap().case_sensitive);
    assert!(after.load_all().unwrap().is_empty());
}

/// Imports that must be refused, with the fragment their error has to name.
/// A refused import leaves no destination and no temporary behind.
#[test]
fn import_refuses_bad_streams_without_touching_the_destination() {
    let dir = TempDir::new("girsync_cache_json", "bad_streams");
    let dst = dir.join("imported-cache");
    let meta = r#"{"type":"meta","version":4,"case_sensitive":true}"#;
    let entry = |path: &str| {
        format!(
            r#"{{"type":"entry","path":"{path}","kind":"file","size":1,"mtime_ns":2,"hashes":[]}}"#
        )
    };

    let cases: Vec<(&str, String, &str)> = vec![
        (
            "foreign version",
            format!("{}\n", meta.replace("\"version\":4", "\"version\":99")),
            "unsupported cache version 99",
        ),
        (
            "entry before meta",
            format!("{}\n", entry("a.txt")),
            "meta line must come first",
        ),
        (
            "second meta",
            format!("{meta}\n{meta}\n"),
            "second meta line",
        ),
        (
            "duplicate path",
            format!("{meta}\n{}\n{}\n", entry("a.txt"), entry("a.txt")),
            "repeats path",
        ),
        (
            "malformed line",
            format!("{meta}\n{}\n{{oops}}\n", entry("a.txt")),
            "line 3",
        ),
        (
            "bad hex",
            format!(
                "{meta}\n{{\"type\":\"entry\",\"path\":\"a.txt\",\"kind\":\"file\",\"size\":1,\"mtime_ns\":2,\"hashes\":[[\"md5\",\"zz\"]]}}\n"
            ),
            "line 2",
        ),
        (
            "unknown kind",
            format!(
                "{meta}\n{{\"type\":\"entry\",\"path\":\"a.txt\",\"kind\":\"link\",\"size\":1,\"mtime_ns\":2,\"hashes\":[]}}\n"
            ),
            "invalid record kind",
        ),
        ("blank line", format!("{meta}\n\n"), "line 2"),
        (
            "blank line between entries",
            format!("{meta}\n{}\n\n{}\n", entry("a.txt"), entry("b.txt")),
            "line 3",
        ),
        ("whitespace-only line", format!("{meta}\n   \n"), "line 2"),
        (
            "whitespace-only final line",
            format!("{meta}\n  "),
            "line 2",
        ),
        ("CRLF blank line", format!("{meta}\r\n\r\n"), "line 2"),
        ("empty stream", String::new(), "expected a meta line"),
    ];

    for (name, text, want) in cases {
        let err = import_cache(Cursor::new(text), &dst, false)
            .err()
            .unwrap_or_else(|| panic!("{name}: import should have failed"));
        assert!(
            format!("{err:#}").contains(want),
            "{name}: error {err:#} does not name {want:?}"
        );
        assert!(!dst.exists(), "{name}: no destination may be left behind");
    }
    let leftovers: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(JSON_TEMP_PREFIX))
        .collect();
    assert!(leftovers.is_empty(), "no import temporary was left behind");
}

#[test]
fn import_accepts_whitespace_separated_objects() {
    let dir = TempDir::new("girsync_cache_json", "whitespace_separated");
    let dst = dir.join("imported-cache");
    let meta = r#"{"type":"meta","version":4,"case_sensitive":true}"#;
    let entry = |path: &str| {
        format!(
            r#"{{"type":"entry","path":"{path}","kind":"file","size":1,"mtime_ns":2,"hashes":[]}}"#
        )
    };
    let stream = format!("{} {} {}", meta, entry("a.txt"), entry("b.txt"));
    let stats = import_cache(Cursor::new(stream), &dst, false).unwrap();
    assert_eq!(stats.entries, 2);
    let after = CacheDb::open_record(&dst).unwrap();
    assert!(after.get("a.txt").unwrap().is_some());
    assert!(after.get("b.txt").unwrap().is_some());
}

#[test]
fn import_needs_force_to_replace_a_cache_and_then_replaces_it_atomically() {
    let dir = TempDir::new("girsync_cache_json", "force");
    let dst = dir.join("girpr-cache");
    let old = girsync_cache::open_db(
        &dst,
        true,
        CacheOpen::ReadWrite {
            ignore_cache: false,
            backup_first: false,
            retain: None,
        },
    )
    .unwrap();
    old.put("old.txt", &FileRec::dir()).unwrap();
    drop(old);

    let src_dir = TempDir::new("girsync_cache_json", "force_src");
    let mut out = Vec::new();
    export_cache(&fixture_cache(&src_dir), &mut out).unwrap();
    let err = import_cache(Cursor::new(out.clone()), &dst, false)
        .expect_err("an existing destination needs --force");
    assert!(format!("{err:#}").contains("pass --force"));

    let kept = CacheDb::open_record(&dst).unwrap();
    assert!(kept.get("old.txt").unwrap().is_some());
    drop(kept);

    import_cache(Cursor::new(out), &dst, true).unwrap();
    let after = CacheDb::open_record(&dst).unwrap();
    assert!(after.get("old.txt").unwrap().is_none());
    assert!(after.get("a.txt").unwrap().is_some());
    assert_eq!(after.load_all().unwrap().len(), 3);
}

#[test]
fn import_refuses_a_directory_destination() {
    let dir = TempDir::new("girsync_cache_json", "dir_destination");
    let dst = dir.join("not-a-file");
    std::fs::create_dir(&dst).unwrap();
    let stream = format!(
        "{{\"type\":\"meta\",\"version\":{},\"case_sensitive\":true}}\n",
        girsync_cache::CACHE_VERSION
    );
    let err = import_cache(Cursor::new(stream), &dst, true)
        .expect_err("a directory is not a valid cache destination");
    assert!(format!("{err:#}").contains("is not a file"));
    assert!(dst.is_dir(), "the refused destination was left alone");
}

#[test]
fn the_cli_accepts_both_directions_and_the_dash_shorthand() {
    let args = Args::try_parse_from(["cache-json", "to-json", "cache.redb"]).unwrap();
    assert!(matches!(
        args.command,
        Command::ToJson { ref ndjson, .. } if ndjson == Path::new("-")
    ));

    let args =
        Args::try_parse_from(["cache-json", "from-json", "-", "cache.redb", "--force"]).unwrap();
    assert!(matches!(
        args.command,
        Command::FromJson {
            force: true,
            ref ndjson,
            ..
        } if ndjson == Path::new("-")
    ));
}
