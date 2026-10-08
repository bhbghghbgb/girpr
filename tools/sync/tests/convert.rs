//! End-to-end coverage for the one-shot sled -> redb converter.
//!
//! Builds a legacy sled DB with the old JSON/hex layout directly, converts
//! it, and reads the result back through the public redb cache API.

mod common;

use common::{TempRoot, log, parse_ndjson, rw};
use girsync::cache::{load_all_records, open_db};
use sha2::Digest;
use std::collections::HashMap;

fn old_rec(kind: &str, size: u64, hashes: HashMap<String, String>) -> serde_json::Value {
    serde_json::json!({
        "kind": kind,
        "size": size,
        "mtime_ns": 1_700_000_000_000_000_000_i64,
        "hashes": hashes,
    })
}

/// A legacy sled DB, holding exactly what the case below needs.
///
/// **Both file rows carry every algorithm the case requests**, and that is
/// load-bearing rather than incidental. The coverage rule says a record side must
/// be able to supply every requested algorithm for every undecided pair, so a row
/// holding only `md5` makes the run fail — which is the *correct* answer, not a
/// converter bug. A fixture spread one algorithm per row would be uncoverable by
/// accident rather than on purpose.
///
/// The `blake3` row is the other point of the case: an algorithm this crate has
/// never heard of must survive conversion untouched and must not affect a diff that
/// never asks for it.
fn build_sled_db(dir: &std::path::Path) {
    let db_path = dir.join("legacy-cache");
    let db = sled::open(&db_path).unwrap();
    db.insert(
        "\0meta",
        serde_json::to_vec(&serde_json::json!({
            "version": 1,
            "case_sensitive": true,
        }))
        .unwrap(),
    )
    .unwrap();
    db.insert(
        "a.txt",
        serde_json::to_vec(&old_rec(
            "file",
            5,
            [
                ("md5".to_string(), format!("{:x}", md5::compute(b"hello"))),
                (
                    "sha256".to_string(),
                    format!("{:x}", sha2::Sha256::digest(b"hello")),
                ),
            ]
            .into_iter()
            .collect(),
        ))
        .unwrap(),
    )
    .unwrap();
    // sha256 of empty string (well-known) + a dir row + an unknown algorithm.
    db.insert(
        "sub/b.bin",
        serde_json::to_vec(&old_rec(
            "file",
            0,
            [
                ("md5".to_string(), format!("{:x}", md5::compute(b""))),
                (
                    "sha256".to_string(),
                    "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".to_string(),
                ),
                ("blake3".to_string(), "ab".repeat(32)),
            ]
            .into_iter()
            .collect(),
        ))
        .unwrap(),
    )
    .unwrap();
    db.insert(
        "sub",
        serde_json::to_vec(&old_rec("dir", 0, HashMap::new())).unwrap(),
    )
    .unwrap();
    db.flush().unwrap();
    drop(db);
}

#[test]
fn convert_sled_dir_to_redb_file() {
    let t = TempRoot::new("convert");
    let work = t.mkdirs("w");
    build_sled_db(&work);
    let src = work.join("legacy-cache");
    let dst = work.join("girpr-cache");

    let c = girsync::convert::sled_to_redb(&src, &dst, false).unwrap();
    assert_eq!((c.files, c.dirs), (2, 1));
    assert!(dst.is_file(), "destination is a redb file");
    assert!(src.is_dir(), "source sled dir is left untouched");

    let recs = {
        let cache = open_db(&dst, true, rw()).unwrap();
        load_all_records(&cache).unwrap()
        // cache handle drops here, releasing the file lock before compare
    };
    assert_eq!(recs.len(), 3);

    let a = &recs["a.txt"];
    assert_eq!(a.kind, "file");
    assert_eq!(a.size, 5);
    assert_eq!(a.hashes["md5"], md5::compute(b"hello").0.to_vec());

    let b = &recs["sub/b.bin"];
    assert_eq!(b.hashes["sha256"].len(), 32);
    assert_eq!(
        b.hashes["blake3"],
        vec![0xabu8; 32],
        "unknown algorithms pass through as raw bytes"
    );

    assert_eq!(recs["sub"].kind, "dir");

    // Converted caches are immediately usable: comparing the record against
    // a folder with the same content is equal.
    let live = t.mkdirs("live");
    common::wfile(&live, "a.txt", b"hello");
    common::wfile(&live, "sub/b.bin", b"");
    std::fs::create_dir_all(live.join("sub")).unwrap();
    // Pin mtimes to the converted rows' stamps so size+mtime+hash all agree
    // (hashes alone would also agree, but mtime is part of equality).
    let ts = std::time::UNIX_EPOCH + std::time::Duration::from_nanos(1_700_000_000_000_000_000);
    for rel in ["a.txt", "sub/b.bin"] {
        std::fs::OpenOptions::new()
            .write(true)
            .open(live.join(rel))
            .unwrap()
            .set_modified(ts)
            .unwrap();
    }
    let mut o = common::compare(dst.clone(), live.clone());
    o.trust.no_trust_src = true;
    o.trust.no_trust_dst = true;
    o.common.algos = vec!["md5".to_string(), "sha256".to_string()];
    let _ = &log();
    // record-vs-folder with extra blake3 rows: blake3 is not requested, so
    // the diff only consults md5/sha256 and must be equal.
    assert_eq!(girsync::cmd_compare(o, &common::log()).unwrap(), 0);
}

#[test]
fn convert_refuses_to_overwrite_without_force() {
    let t = TempRoot::new("convert_force");
    let work = t.mkdirs("w");
    build_sled_db(&work);
    let src = work.join("legacy-cache");
    let dst = work.join("girpr-cache");

    girsync::convert::sled_to_redb(&src, &dst, false).unwrap();
    assert!(girsync::convert::sled_to_redb(&src, &dst, false).is_err());
    // ...but succeeds with --force semantics.
    let c = girsync::convert::sled_to_redb(&src, &dst, true).unwrap();
    assert_eq!((c.files, c.dirs), (2, 1));
}

#[test]
fn convert_rejects_missing_source() {
    let t = TempRoot::new("convert_missing");
    let work = t.mkdirs("w");
    assert!(
        girsync::convert::sled_to_redb(&work.join("nope"), &work.join("girpr-cache"), false)
            .is_err()
    );
}

/// The converter reports through the same writer as `girsync`, so `--output json`
/// means the same thing in both binaries and a caller parsing one can parse the
/// other. Asserted through the binary, because that is where the flag's parsing
/// and its reporting live.
#[test]
fn the_converter_reports_json_records_on_stdout() {
    let t = TempRoot::new("convert_json");
    let work = t.mkdirs("w");
    build_sled_db(&work);
    let src = work.join("legacy-cache");
    let dst = work.join("girpr-cache");

    let run = |extra: &[&str]| -> (i32, String, String) {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_sled2redb"))
            .arg(&src)
            .arg(&dst)
            .args(extra)
            .output()
            .expect("spawn sled2redb");
        (
            out.status.code().expect("sled2redb exits with a code"),
            String::from_utf8(out.stdout).expect("utf-8"),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    };
    // 5 = md5 + sha256 on each of the two files, plus blake3 on one of them. The
    // count tracks the fixture: `build_sled_db` gives every row both requested
    // algorithms so the converted record can answer a `--hash md5 --hash sha256`
    // audit, and `blake3` is the extra one that must survive untouched.
    let want = serde_json::json!({"event": "converted", "src": src.display().to_string(),
                                 "dst": dst.display().to_string(), "files": 2, "dirs": 1,
                                 "hashes": 5});

    let (code, stdout, stderr) = run(&["--output", "json"]);
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(parse_ndjson(stdout.as_bytes()), vec![want.clone()]);

    // The same record, text-rendered, and text is the default.
    let (code, stdout, stderr) = run(&["--force", "--output", "text"]);
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(
        stdout,
        format!(
            "converted src={} dst={} files=2 dirs=1 hashes=5\n",
            src.display(),
            dst.display()
        )
    );
    let (code, stdout, stderr) = run(&["--force"]);
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(
        stdout.starts_with("converted src="),
        "text is the default: {stdout}"
    );
}
