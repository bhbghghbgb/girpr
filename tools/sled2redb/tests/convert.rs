//! End-to-end coverage for the one-shot sled -> redb converter.
//!
//! Builds a legacy sled DB with the old JSON/hex layout directly, converts
//! it, and reads the result back through the public redb cache API.

mod common;

use clap::{Parser, ValueEnum};
use common::{parse_ndjson, rw, temp_root};
use girsync::cache::{FileRec, load_all_records, open_db};
use girsync::hash::DigestSet;
use girsync::report::OutputFormat;
use md5::Md5;
use sha2::Digest;
use sled2redb::Args;
use sled2redb::convert::{Converted, write_records};
use std::collections::HashMap;
use std::path::Path;

fn old_rec(kind: &str, size: u64, hashes: HashMap<String, String>) -> serde_json::Value {
    serde_json::json!({
        "kind": kind,
        "size": size,
        "mtime_ns": 1_700_000_000_000_000_000_i64,
        "hashes": hashes,
    })
}

/// Render a digest as lowercase hex.
fn hex(bytes: &[u8]) -> String {
    ::hex::encode(bytes)
}

/// A decoded row built directly, for cases that only exercise the redb-write
/// half. `(algorithm, bytes)` pairs go through `insert_named`, the same routing
/// rule the sled reader uses, so an unknown algorithm lands in the foreign arm
/// just as it would after a real conversion.
fn decoded_rec(kind: &str, size: u64, hashes: &[(&str, Vec<u8>)]) -> FileRec {
    let mut set = DigestSet::default();
    for (algo, bytes) in hashes {
        set.insert_named(algo, bytes);
    }
    FileRec {
        kind: kind.to_string(),
        size,
        mtime_ns: 1_700_000_000_000_000_000,
        hashes: set,
    }
}

/// Write the round-trip fixture's decoded rows through [`write_records`], with
/// an optional extra file row. This gives the same `(2, 1, 5)` counters as
/// [`build_sled_db`] (two files, one dir, five hashes — five because one file
/// carries an extra, unknown algorithm), so a case can pin counters and file
/// outcomes without paying for an fsync-bearing sled fixture.
fn write_fixture_records(dst: &Path, extra_file: bool) -> Converted {
    let mut recs: Vec<(String, FileRec)> = vec![
        (
            "a.txt".into(),
            decoded_rec(
                "file",
                5,
                &[("md5", vec![1u8; 16]), ("sha256", vec![2u8; 32])],
            ),
        ),
        (
            "sub/b.bin".into(),
            decoded_rec(
                "file",
                0,
                &[
                    ("md5", vec![3u8; 16]),
                    ("sha256", vec![4u8; 32]),
                    ("blake3", vec![0xabu8; 32]),
                ],
            ),
        ),
        ("sub".into(), FileRec::dir()),
    ];
    if extra_file {
        recs.push(("later.txt".into(), decoded_rec("file", 7, &[])));
    }
    write_records(dst, true, &recs).unwrap()
}

/// Run the converter binary and hand back its exit code, stdout and stderr.
///
/// The binary is spawned where the *process* boundary is what is under test —
/// its exit code, its `--help` output, and one flag spelled end to end. The
/// arguments and the record now live in the library
/// ([`Args`] / [`sled2redb::conversion_output`]), so everything else in this
/// suite inspects them in-process without a conversion subprocess.
fn run_converter<I>(args: I) -> (i32, String, String)
where
    I: IntoIterator,
    I::Item: AsRef<std::ffi::OsStr>,
{
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_sled2redb"))
        .args(args)
        .output()
        .expect("spawn sled2redb");
    (
        out.status.code().expect("sled2redb exits with a code"),
        String::from_utf8(out.stdout).expect("utf-8"),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// Parse an argument spelling against the converter's own [`Args`], in process.
///
/// This is the parser the binary itself uses, so it checks clap's *acceptance*
/// of a spelling — and the value a flag selects — without paying for a full
/// conversion subprocess. The help pin still holds the hand-built command
/// ([`the_converter_command`]); parsing does not need it. Conversion
/// correctness is the job of the rest of this suite and of the one end-to-end
/// run kept below.
fn parse_in_process(args: Vec<std::ffi::OsString>) -> Args {
    // `try_parse_from` takes the first item as argv[0], so prepend the program
    // name and let the caller pass only the arguments under test.
    let argv = std::iter::once(std::ffi::OsString::from("sled2redb")).chain(args);
    Args::try_parse_from(argv)
        .unwrap_or_else(|e| panic!("the spelling must be accepted by clap: {e}"))
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
    build_sled_db_at(dir, "legacy-cache");
}

/// The same fixture as [`build_sled_db`], in a database of the given name.
///
/// The name is a parameter so a case can hold a source that converts and a
/// source that does not at the same time, which is what the `--force` case
/// needs: it needs a cache to preserve *and* a source that fails.
fn build_sled_db_at(dir: &std::path::Path, name: &str) {
    let db_path = dir.join(name);
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
                ("md5".to_string(), hex(&Md5::digest(b"hello"))),
                ("sha256".to_string(), hex(&sha2::Sha256::digest(b"hello"))),
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
                ("md5".to_string(), hex(&Md5::digest(b""))),
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

/// A legacy sled DB with one row the converter cannot decode.
///
/// The row is bad the way a real legacy cache goes bad: a digest that is not
/// hex. It is a database of its own, so a case can hold a source that
/// converts and one that does not at the same time.
fn build_sled_db_with_bad_row(dir: &std::path::Path) {
    let db_path = dir.join("legacy-cache-bad");
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
        "bad.txt",
        serde_json::to_vec(&old_rec(
            "file",
            3,
            [("md5".to_string(), "zz".to_string())]
                .into_iter()
                .collect(),
        ))
        .unwrap(),
    )
    .unwrap();
    db.flush().unwrap();
    drop(db);
}

/// The names of the entries in `dir`, sorted.
///
/// The whole directory, not a filtered view of it: what a case wants to know
/// is what the tool left in the user's tree, and a filter would hide the very
/// entry it is looking for.
fn entry_names(dir: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// The round-trip test: the one case that pays for a file-backed reopen of the
/// converted cache. It asserts the whole contract — rows decoded from the sled
/// JSON/hex layout into redb records, unknown algorithms passed through, and a
/// converted cache that compares equal to a folder with the same content. Every
/// other case inspects outcomes without reopening the cache file.
#[test]
fn convert_sled_dir_to_redb_file() {
    let t = temp_root("convert");
    let work = t.subdir("w");
    build_sled_db(&work);
    let src = work.join("legacy-cache");
    let dst = work.join("girpr-cache");

    let c = sled2redb::convert::sled_to_redb(&src, &dst, false).unwrap();
    assert_eq!((c.files, c.dirs), (2, 1));
    assert!(dst.is_file(), "destination is a redb file");
    assert!(src.is_dir(), "source sled dir is left untouched");
    assert_eq!(
        entry_names(&work),
        vec!["girpr-cache".to_string(), "legacy-cache".to_string()],
        "the conversion left nothing behind"
    );

    let recs = {
        let cache = open_db(&dst, true, rw()).unwrap();
        load_all_records(&cache).unwrap()
        // cache handle drops here, releasing the file lock before compare
    };
    assert_eq!(recs.len(), 3);

    let a = &recs["a.txt"];
    assert_eq!(a.kind, "file");
    assert_eq!(a.size, 5);
    assert_eq!(
        a.hashes.get_named("md5"),
        Some(&Md5::digest(b"hello")[..]),
        "hex digests were decoded to raw bytes"
    );

    let b = &recs["sub/b.bin"];
    assert_eq!(b.hashes.get_named("sha256").map(<[u8]>::len), Some(32));
    assert_eq!(
        b.hashes.get_named("blake3"),
        Some(&[0xabu8; 32][..]),
        "unknown algorithms pass through as raw bytes"
    );

    assert_eq!(recs["sub"].kind, "dir");

    // Converted caches are immediately usable: comparing the record against
    // a folder with the same content is equal.
    let live = t.subdir("live");
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
    o.common.algos = vec![girsync::hash::HashAlg::Md5, girsync::hash::HashAlg::Sha256];
    // record-vs-folder with extra blake3 rows: blake3 is not requested, so
    // the diff only consults md5/sha256 and must be equal.
    assert_eq!(girsync::cmd_compare(o, &common::log()).unwrap(), 0);
}

#[test]
fn convert_refuses_to_overwrite_without_force() {
    let t = temp_root("convert_force");
    let work = t.subdir("w");
    build_sled_db(&work);
    let src = work.join("legacy-cache");
    let dst = work.join("girpr-cache");

    // A stale file at the destination is enough to exercise the refuse branch:
    // `sled_to_redb` checks only `dst.exists()` before declining to overwrite,
    // so no full conversion is needed to build the thing it refuses.
    std::fs::write(&dst, b"stale").unwrap();
    assert!(sled2redb::convert::sled_to_redb(&src, &dst, false).is_err());
    // ...but succeeds with --force semantics.
    let c = sled2redb::convert::sled_to_redb(&src, &dst, true).unwrap();
    assert_eq!((c.files, c.dirs), (2, 1));
}

#[test]
fn convert_rejects_missing_source() {
    let t = temp_root("convert_missing");
    let work = t.subdir("w");
    assert!(
        sled2redb::convert::sled_to_redb(&work.join("nope"), &work.join("girpr-cache"), false)
            .is_err()
    );
}

/// A conversion that fails to decode a row is harmless in both directions.
///
/// Two different questions are pinned here, against the **one** bad-row sled
/// DB both need. Without `--force`, no destination may appear and nothing may
/// be left behind: a failed conversion must not leave a file that opens as a
/// cache, nor a temporary sibling in the user's tree. With `--force`, the
/// previous cache — the only copy of the hashes the old build had — must be
/// left byte-for-byte intact; destroying it before the new one is known good
/// would trade a cache for nothing.
///
/// The previous cache is written straight through the redb-write seam with
/// the same decoded rows the good sled fixture used to convert. That needs no
/// good sled DB (and no fsync) to set up the thing that must be preserved;
/// only the bad-row DB is real, because it is the source of the failure.
#[test]
fn a_failed_conversion_leaves_no_destination_and_no_temporary() {
    let t = temp_root("convert_fail");
    let work = t.subdir("w");
    build_sled_db_with_bad_row(&work);
    let src = work.join("legacy-cache-bad");

    // Without --force: no destination, and no temporary residue.
    let dst = work.join("girpr-cache");
    let err = sled2redb::convert::sled_to_redb(&src, &dst, false).unwrap_err();
    let msg = format!("{:#}", err);
    assert!(
        msg.contains("bad.txt") && msg.contains("md5"),
        "the failure names the row it could not decode: {msg}"
    );
    assert!(!dst.exists(), "a failed conversion leaves no destination");
    assert_eq!(
        entry_names(&work),
        vec!["legacy-cache-bad".to_string()],
        "nothing but the source is left behind"
    );

    // With --force: the previous cache survives byte for byte.
    let forced = work.join("girpr-cache-prev");
    let c = write_fixture_records(&forced, false);
    assert_eq!((c.files, c.dirs, c.hashes), (2, 1, 5));
    let before = std::fs::read(&forced).unwrap();

    let err = sled2redb::convert::sled_to_redb(&src, &forced, true).unwrap_err();
    assert!(
        format!("{:#}", err).contains("bad.txt"),
        "the failure names the row it could not decode: {:#}",
        err
    );
    assert_eq!(
        std::fs::read(&forced).unwrap(),
        before,
        "the previous cache is byte-identical"
    );
    assert_eq!(
        entry_names(&work),
        vec![
            "girpr-cache-prev".to_string(),
            "legacy-cache-bad".to_string()
        ],
        "the failed conversion left nothing but the source db and the cache"
    );
}

/// A rewritten cache **replaces** the old destination and leaves no temporary.
///
/// The write half is driven directly with decoded rows: a first cache is
/// written at the destination, then a second with an extra file row is written
/// into a sibling and moved onto the destination the way `sled_to_redb` puts a
/// finished temporary in place. The second result is observably different
/// from the first — the destination would still hold three rows rather than
/// four if it were left alone — and the replacement is checked by comparing
/// bytes and by listing the directory. The exact counters come from
/// [`write_records`]; the faithful sled-row-to-redb-row mapping is asserted
/// once in [`convert_sled_dir_to_redb_file`], so no reopen is needed here.
///
/// The `sled_to_redb` `--force` path itself is still pinned end to end by
/// [`convert_refuses_to_overwrite_without_force`], so this case can use the
/// pure seam without losing that behavior.
#[test]
fn a_rewrite_replaces_the_destination_and_leaves_no_temporary() {
    let t = temp_root("convert_force_ok");
    let work = t.subdir("w");
    let dst = work.join("girpr-cache");

    let first = write_fixture_records(&dst, false);
    assert_eq!((first.files, first.dirs, first.hashes), (2, 1, 5));
    let before = std::fs::read(&dst).unwrap();

    // Write the "converted" cache into a sibling, as sled_to_redb does, then
    // move it onto the destination.
    let temp = work.join("girpr-cache-next");
    let c = write_fixture_records(&temp, true);
    assert_eq!((c.files, c.dirs), (3, 1));
    std::fs::rename(&temp, &dst).unwrap();

    assert!(dst.is_file(), "the destination is a redb file");
    assert_ne!(
        std::fs::read(&dst).unwrap(),
        before,
        "the rewrite replaced the old cache"
    );
    assert_eq!(
        entry_names(&work),
        vec!["girpr-cache".to_string()],
        "the rewrite left nothing but the cache"
    );
}

/// The converter reports through the same writer as `girsync`, so `--output json`
/// means the same thing in both binaries and a caller parsing one can parse the
/// other. The record is a pure function of the counts and the two paths, so its
/// renderings are asserted in-process against the library seam: no spawn and no
/// sled fixture are needed to pin JSON, text and the default format.
#[test]
fn the_converter_reports_json_records_on_stdout() {
    let src = Path::new("legacy-cache");
    let dst = Path::new("girpr-cache");
    // 5 = md5 + sha256 on each of the two files, plus blake3 on one of them —
    // the counts `build_sled_db` yields for this fixture, now supplied directly
    // because only the render, not the conversion, is under test here.
    let c = Converted {
        files: 2,
        dirs: 1,
        hashes: 5,
    };
    let want = serde_json::json!({"event": "converted", "src": src.display().to_string(),
                                 "dst": dst.display().to_string(), "files": 2, "dirs": 1,
                                 "hashes": 5});

    let json = sled2redb::conversion_output(&c, src, dst, OutputFormat::Json);
    assert_eq!(parse_ndjson(json.as_bytes()), vec![want]);

    // The same record, text-rendered.
    let text = sled2redb::conversion_output(&c, src, dst, OutputFormat::Text);
    assert_eq!(
        text,
        format!(
            "converted src={} dst={} files=2 dirs=1 hashes=5",
            src.display(),
            dst.display()
        )
    );

    // Text is the default: the flag omitted from a real parse selects it, and
    // the default renders exactly like an explicit `--output text`.
    let default = Args::try_parse_from(["sled2redb", "sled", "redb"])
        .expect("the two paths alone parse")
        .output;
    assert_eq!(default, OutputFormat::Text, "text is the default");
    assert_eq!(
        sled2redb::conversion_output(&c, src, dst, default),
        text,
        "the default renders exactly like --output text"
    );
}

/// The flags are accepted on either side of the two paths, and `--output=json`
/// is the same as `--output json`.
///
/// The parser is `clap`'s, so its spelling is the spelling and no narrower set
/// of forms sits behind it. The forms are pinned here because a run that
/// refused one of them would fail on the spelling rather than on the
/// conversion — and the spelling is what a script writes.
///
/// Acceptance is a parsing property, so every form is checked in-process
/// against the converter's own [`Args`]. Only one full conversion remains: it
/// proves a spelling end to end and prints the record every spelling prints.
/// The two `--output` spellings are proved byte-identical in-process, by
/// parsing each and rendering the same record through the library seam.
/// Conversion correctness is covered by the rest of the suite.
#[test]
fn the_flags_are_accepted_on_either_side_of_the_paths() {
    let t = temp_root("convert_args");
    let work = t.subdir("w");
    build_sled_db(&work);
    let src = work.join("legacy-cache");

    // The arguments of a run, in the order the form spells them: the flags
    // before the paths, the two paths, the flags after them.
    let args_of = |before: &[&str], after: &[&str], dst: &std::path::Path| {
        let mut args: Vec<std::ffi::OsString> = before
            .iter()
            .map(|a| std::ffi::OsStr::new(*a).to_os_string())
            .collect();
        args.push(src.as_os_str().to_os_string());
        args.push(dst.as_os_str().to_os_string());
        args.extend(
            after
                .iter()
                .map(|a| std::ffi::OsStr::new(*a).to_os_string()),
        );
        args
    };

    // The forms, as the flags before the paths and the flags after them: the
    // flags after the paths, the flags before them, the `=` spelling on either
    // side of the paths, and `--force` on either side of them. Each is checked
    // in-process, so a spelling that clap refused fails here on the spelling
    // rather than on a conversion — which is exactly what is under test.
    let forms: &[(&[&str], &[&str])] = &[
        (&[], &["--output", "json"]),
        (&["--output", "json"], &[]),
        (&[], &["--output=json"]),
        (&["--output=json"], &[]),
        (&["--force", "--output", "json"], &[]),
        (&["--output", "json"], &["--force"]),
    ];

    for (i, (before, after)) in forms.iter().enumerate() {
        let dst = work.join(format!("girpr-cache-parse-{i}"));
        let args = parse_in_process(args_of(before, after, &dst));
        assert_eq!(
            args.output,
            OutputFormat::Json,
            "the form {before:?} before the paths and {after:?} after them must select json"
        );
        assert_eq!(
            args.sled_dir.as_os_str(),
            src.as_os_str(),
            "the sled path is the first positional in the form {before:?} / {after:?}"
        );
        assert_eq!(
            args.redb_path.as_os_str(),
            dst.as_os_str(),
            "the redb path is the second positional in the form {before:?} / {after:?}"
        );
        assert_eq!(
            args.force,
            before.contains(&"--force") || after.contains(&"--force"),
            "the form {before:?} / {after:?} sets --force only where the form spells it"
        );
    }

    // One end-to-end conversion still proves a spelling is accepted by the real
    // binary and prints the record every spelling prints.
    let dst = work.join("girpr-cache-spellings");
    let (code, spaced, stderr) = run_converter(args_of(&["--output", "json"], &[], &dst));
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(
        parse_ndjson(spaced.as_bytes()),
        vec![serde_json::json!({
            "event": "converted",
            "src": src.display().to_string(),
            "dst": dst.display().to_string(),
            "files": 2,
            "dirs": 1,
            "hashes": 5,
        })],
        "the record is the one every spelling prints"
    );

    // The two spellings of `--output` are the same record: parse each spelling
    // against `Args` — the second one also carries `--force` — and render the
    // same record with each. Both select json and `--force` never reaches the
    // record, so the lines are byte-identical without a second conversion.
    let equals_dst = work.join("girpr-cache-equals");
    let spaced_args = parse_in_process(args_of(&["--output", "json"], &[], &equals_dst));
    let equals_args = parse_in_process(args_of(&["--force", "--output=json"], &[], &equals_dst));
    assert_eq!(spaced_args.output, OutputFormat::Json);
    assert_eq!(equals_args.output, OutputFormat::Json);
    assert!(
        !spaced_args.force && equals_args.force,
        "--force is set only where the form spells it"
    );
    let c = Converted {
        files: 2,
        dirs: 1,
        hashes: 5,
    };
    assert_eq!(
        sled2redb::conversion_output(&c, &src, &equals_dst, spaced_args.output),
        sled2redb::conversion_output(&c, &src, &equals_dst, equals_args.output),
        "the two --output spellings render the same bytes"
    );
}

/// Usage errors are failures like any other, and fail with the converter's own
/// code.
///
/// `clap` exits `2` for a parse error, and the main binary keeps that. This
/// binary does not: it has one exit code for every way it can fail, so a
/// script that ran the converter needs no second code to remember.
///
/// Every case is a row below: the arguments under test, the single exit code
/// all failures share (`3`), and the name the error must print so a script can
/// tell which argument it got wrong. Naming the bad argument is a clap parse
/// property, so three of the four rows are asserted in-process through
/// [`Args::try_parse_from`] — the same parser the binary uses.
///
/// The version row is the one shape that could pass for an *answer* rather
/// than a failure: a converter that grew a version would print one and exit
/// `0`, and nothing else in this file converts something to catch it. So that
/// row keeps the one process run in this test, pinning both the exit code the
/// binary chooses for every failure and that no version line is printed on
/// either stream.
#[test]
fn usage_errors_exit_three_and_name_the_bad_argument() {
    struct Case {
        label: &'static str,
        args: &'static [&'static str],
        names: &'static [&'static str],
        is_version: bool,
    }

    let cases: &[Case] = &[
        Case {
            label: "an unknown flag",
            args: &["--not-a-flag"],
            names: &["--not-a-flag"],
            is_version: false,
        },
        Case {
            label: "a version request",
            args: &["--version"],
            names: &["--version"],
            is_version: true,
        },
        Case {
            label: "a missing positional",
            args: &["only-one"],
            names: &["REDB_PATH"],
            is_version: false,
        },
        Case {
            label: "an unknown output format",
            args: &["sled", "cache.redb", "--output", "yaml"],
            names: &["text", "json"],
            is_version: false,
        },
    ];

    for case in cases {
        if case.is_version {
            // The one process run here: it pins the binary's exit-code wiring
            // and that an undeclared version is a failure, not an answer.
            let (code, stdout, stderr) = run_converter(case.args);
            assert_eq!(code, 3, "{}: stderr: {stderr}", case.label);
            for name in case.names {
                assert!(
                    stderr.contains(name),
                    "{}: the error names `{name}`: {stderr}",
                    case.label
                );
            }
            assert!(
                stdout.is_empty(),
                "a refused version prints no version line on stdout: {stdout}"
            );
            assert!(
                !stderr.contains(env!("CARGO_PKG_VERSION")),
                "a refused version prints no version line at all: {stderr}"
            );
        } else {
            // The other three spellings are parse properties: `Args` is the
            // binary's parser, so its error must name the argument the same way
            // the process error does.
            let argv = std::iter::once(std::ffi::OsString::from("sled2redb")).chain(
                case.args
                    .iter()
                    .map(|a| std::ffi::OsStr::new(*a).to_os_string()),
            );
            let err = match Args::try_parse_from(argv) {
                Ok(_) => panic!("{}: this spelling must be a usage error", case.label),
                Err(e) => e,
            };
            let rendered = err.to_string();
            for name in case.names {
                assert!(
                    rendered.contains(name),
                    "{}: the error names `{name}`: {rendered}",
                    case.label
                );
            }
        }
    }
}

/// The converter's own command, as the help pin holds it.
///
/// This is a **definition** and not a text, and that is the whole point of
/// it: the case below renders it with clap and compares that rendering with
/// the help the binary prints, so **both sides are laid out by the same
/// clap**. What the comparison holds is therefore the converter's own
/// command — its about, its arguments and flags, their descriptions and the
/// default it takes — and none of clap's own words.
///
/// Clap's words are on both sides at once: the usage line, the `Arguments:`
/// and `Options:` headers, the `Possible values:` line, the `[default: …]`
/// marker, the print-help wording, and the layout itself. A deliberate clap
/// bump moves all of them together, so the pin still holds and **nothing has
/// to be updated by hand**: the golden is the converter's command, and a clap
/// bump does not change the converter's command.
///
/// A change in the converter's own command moves one side only, and that is a
/// difference: a flag added, removed or renamed, a description reworded, the
/// about changed, the default changed.
///
/// The values `--output` accepts are read from the shared [`OutputFormat`]
/// enum rather than written out here, so the shared wording is not a second
/// copy of it. The enum's own descriptions are rendered on both sides, and a
/// change to them moves both sides together — as a clap bump does. The names
/// of the values are what a user types, and they are pinned by name beside
/// the comparison.
///
/// The descriptions carry no trailing full stop, because clap's derive takes
/// one off the doc comment it reads: `/// Legacy sled cache directory.` is
/// rendered as `Legacy sled cache directory`.
///
/// The bin name is the executable's own spelling, because that is what clap
/// prints in the usage line: the binary is invoked by path, and clap takes
/// the file name of `argv[0]` for it.
fn the_converter_command() -> clap::Command {
    clap::Command::new("sled2redb")
        .about("Convert a legacy sled cache directory into a redb cache file")
        .arg(
            clap::Arg::new("sled_dir")
                .value_name("SLED_DIR")
                .help("Legacy sled cache directory")
                .required(true),
        )
        .arg(
            clap::Arg::new("redb_path")
                .value_name("REDB_PATH")
                .help("Destination redb file")
                .required(true),
        )
        .arg(
            clap::Arg::new("force")
                .long("force")
                .action(clap::ArgAction::SetTrue)
                .help("Replace an existing destination"),
        )
        .arg(
            clap::Arg::new("output")
                .long("output")
                .value_name("OUTPUT")
                .value_parser(clap::builder::EnumValueParser::<OutputFormat>::new())
                .default_value("text")
                .help("How the conversion record is rendered"),
        )
        .bin_name(format!("sled2redb{}", std::env::consts::EXE_SUFFIX))
}

/// The help as a comparison sees it.
///
/// Neither difference is what the pin is about: clap leaves trailing spaces
/// on the blank line inside the possible values, and the rendered help ends
/// where the printed one ends with a newline.
fn normalize_help(help: &str) -> String {
    help.lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
}

/// The help output, pinned over the converter's own command.
///
/// `clap` reads a **multi-paragraph** doc comment on the argument struct as
/// the command's `long_about`, and `--help` prints it **verbatim** — the `**`
/// markers and the `#[command(version)]` attribute in it included, because
/// `clap` does not render markdown. A note written for a source reader would
/// then tell a user about an attribute they cannot type, so the struct carries
/// no doc comment and this case is what holds it there.
///
/// A single-paragraph doc comment is the command's short `about` instead, and
/// the explicit `about` here overrides it, so such a note reaches neither
/// `--help` nor `-h`: it prints nowhere, and there is nothing for this pin to
/// hold.
///
/// The pin is the whole command rather than a few names in it, so anything
/// that reaches the help shows up as a difference: a paragraph of prose, a
/// flag, a changed wording. The names the case above looks for would survive
/// all of them. A multi-paragraph doc comment on the argument struct reaches
/// the help as the text at the top of it, and the top of it is pinned here, so
/// a note put back on the struct shows up as a difference in the first line.
///
/// The pin is a **definition** and not a text, so clap's own words are not
/// held by it: the definition is rendered here, beside the binary's own
/// rendering, and the two are compared. A deliberate clap bump changes both
/// renderings together and the pin still holds — what a clap bump needs
/// updated by hand is the converter's own command, and a clap bump does not
/// change it.
#[test]
fn the_help_output_is_pinned() {
    let (code, stdout, stderr) = run_converter(["--help"]);
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(
        normalize_help(&stdout),
        normalize_help(&the_converter_command().render_long_help().to_string()),
        "the help output is pinned: the converter's own command changed, or a \
         multi-paragraph doc comment on the argument struct would print here"
    );

    // The values the flag accepts are what a user types, so they are pinned
    // by name: the shared enum's wording is not the converter's, but the
    // spellings it accepts are.
    let accepted: Vec<String> = OutputFormat::value_variants()
        .iter()
        .map(|variant| {
            clap::ValueEnum::to_possible_value(variant)
                .expect("every value of the enum is a possible value")
                .get_name()
                .to_string()
        })
        .collect();
    assert_eq!(
        accepted,
        ["text", "json"],
        "the values --output accepts are pinned by name"
    );
}
