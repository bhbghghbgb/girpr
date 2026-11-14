//! End-to-end coverage for the one-shot sled -> redb converter.
//!
//! Builds a legacy sled DB with the old JSON/hex layout directly, converts
//! it, and reads the result back through the public redb cache API.

mod common;

use common::{log, parse_ndjson, rw, temp_root};
use girsync::cache::{load_all_records, open_db};
use girsync::convert::CONVERT_TEMP_PREFIX;
use md5::Md5;
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

/// Run the converter binary and hand back its exit code, stdout and stderr.
///
/// The binary is spawned rather than the library called because the arguments
/// and the exit code live in the binary: `girsync::convert::sled_to_redb` has
/// no opinion about how the run was spelled.
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
                ("md5".to_string(), format!("{:x}", Md5::digest(b"hello"))),
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
                ("md5".to_string(), format!("{:x}", Md5::digest(b""))),
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

/// The temporary is written into the folder the destination cache lives in,
/// so its name has to be inside the tool's own namespace.
///
/// A name outside it would leave the tool's residue looking like ordinary
/// content: the next run over that folder would copy the temporary out of a
/// source folder, or delete it as an extra in a destination one. The scan
/// reserves every `girpr-cache*` component, so a leftover the tool could not
/// remove is tool state rather than something the next run mirrors.
#[test]
fn the_conversion_temporary_is_named_inside_the_tools_namespace() {
    let name = format!("{CONVERT_TEMP_PREFIX}1234-20240101T000000.000000Z.redb");
    assert!(
        girsync::util::is_cache_rel(&name),
        "`{name}` is the tool's own state, not content"
    );
}

#[test]
fn convert_sled_dir_to_redb_file() {
    let t = temp_root("convert");
    let work = t.subdir("w");
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
    let _ = &log();
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

    girsync::convert::sled_to_redb(&src, &dst, false).unwrap();
    assert!(girsync::convert::sled_to_redb(&src, &dst, false).is_err());
    // ...but succeeds with --force semantics.
    let c = girsync::convert::sled_to_redb(&src, &dst, true).unwrap();
    assert_eq!((c.files, c.dirs), (2, 1));
}

#[test]
fn convert_rejects_missing_source() {
    let t = temp_root("convert_missing");
    let work = t.subdir("w");
    assert!(
        girsync::convert::sled_to_redb(&work.join("nope"), &work.join("girpr-cache"), false)
            .is_err()
    );
}

/// A conversion that fails leaves **no destination at all**, and nothing else.
///
/// Two things are pinned, and they are different questions. The destination
/// is absent, because there was no `--force` and therefore no previous cache
/// to preserve — a failed conversion must not leave a file that opens as a
/// cache, because the next run would trust it. And the directory holds
/// nothing but the source: the conversion writes into a temporary sibling
/// beside the destination, and that temporary is the tool's own residue. It
/// must not outlive the failure, because a file in the user's tree that
/// looks like a cache is a file the user has to clean up by hand.
#[test]
fn a_failed_conversion_leaves_no_destination_and_no_temporary() {
    let t = temp_root("convert_fail");
    let work = t.subdir("w");
    build_sled_db_with_bad_row(&work);
    let src = work.join("legacy-cache-bad");
    let dst = work.join("girpr-cache");

    let err = girsync::convert::sled_to_redb(&src, &dst, false).unwrap_err();
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
}

/// Under `--force`, a conversion that fails leaves the previous cache exactly
/// as it was.
///
/// This is the half that decides whether `--force` is safe to use: the
/// previous cache is the only copy of the hashes the old build had, so
/// destroying it before the new one is known to be good trades a cache for
/// nothing.
///
/// The old cache is compared byte for byte and then opened, because "the file
/// is still there" and "the file is still a cache" are different answers, and
/// only the second one is worth anything to the next run.
#[test]
fn a_failed_forced_conversion_leaves_the_previous_cache_intact() {
    let t = temp_root("convert_force_fail");
    let work = t.subdir("w");
    build_sled_db(&work);
    build_sled_db_with_bad_row(&work);
    let good = work.join("legacy-cache");
    let bad = work.join("legacy-cache-bad");
    let dst = work.join("girpr-cache");

    girsync::convert::sled_to_redb(&good, &dst, false).unwrap();
    let before = std::fs::read(&dst).unwrap();

    let err = girsync::convert::sled_to_redb(&bad, &dst, true).unwrap_err();
    assert!(
        format!("{:#}", err).contains("bad.txt"),
        "the failure names the row it could not decode: {:#}",
        err
    );
    assert_eq!(
        std::fs::read(&dst).unwrap(),
        before,
        "the previous cache is byte-identical"
    );

    // ...and it is still a cache, not just a file of the right size.
    let recs = {
        let cache = open_db(&dst, true, rw()).unwrap();
        load_all_records(&cache).unwrap()
        // cache handle drops here, releasing the file lock
    };
    assert_eq!(recs.len(), 3, "the previous cache still holds its rows");
    assert_eq!(
        entry_names(&work),
        vec![
            "girpr-cache".to_string(),
            "legacy-cache".to_string(),
            "legacy-cache-bad".to_string(),
        ],
        "the failed conversion left nothing behind"
    );
}

/// A successful `--force` conversion **replaces** the old destination.
///
/// The source gains a row between the two conversions, so the second result
/// is observably different from the first: a destination that was left
/// alone would still hold three rows rather than four.
#[test]
fn a_forced_conversion_replaces_the_destination_and_leaves_no_temporary() {
    let t = temp_root("convert_force_ok");
    let work = t.subdir("w");
    build_sled_db(&work);
    let src = work.join("legacy-cache");
    let dst = work.join("girpr-cache");

    girsync::convert::sled_to_redb(&src, &dst, false).unwrap();
    {
        let db = sled::open(&src).unwrap();
        db.insert(
            "later.txt",
            serde_json::to_vec(&old_rec("file", 7, HashMap::new())).unwrap(),
        )
        .unwrap();
        db.flush().unwrap();
    }

    let c = girsync::convert::sled_to_redb(&src, &dst, true).unwrap();
    assert_eq!((c.files, c.dirs), (3, 1));
    assert!(dst.is_file(), "the destination is a redb file");

    let recs = {
        let cache = open_db(&dst, true, rw()).unwrap();
        load_all_records(&cache).unwrap()
        // cache handle drops here, releasing the file lock
    };
    assert_eq!(recs.len(), 4, "the new cache replaced the old one");
    assert!(
        recs.contains_key("later.txt"),
        "the row the first conversion did not have is there"
    );
    assert_eq!(
        entry_names(&work),
        vec!["girpr-cache".to_string(), "legacy-cache".to_string()],
        "the conversion left nothing behind"
    );
}

/// A conversion that succeeds creates the destination and nothing else.
///
/// The temporary the conversion writes into is gone, which is the half of
/// the atomicity that is easy to forget: a leftover beside the destination
/// is a file in the user's tree that nothing will ever clean up.
#[test]
fn a_successful_conversion_creates_the_destination_and_no_temporary() {
    let t = temp_root("convert_ok");
    let work = t.subdir("w");
    build_sled_db(&work);
    let src = work.join("legacy-cache");
    let dst = work.join("girpr-cache");

    let c = girsync::convert::sled_to_redb(&src, &dst, false).unwrap();
    assert_eq!((c.files, c.dirs), (2, 1));
    assert!(dst.is_file(), "the destination is a redb file");
    assert_eq!(
        entry_names(&work),
        vec!["girpr-cache".to_string(), "legacy-cache".to_string()],
        "the conversion left nothing behind"
    );
}

/// The converter reports through the same writer as `girsync`, so `--output json`
/// means the same thing in both binaries and a caller parsing one can parse the
/// other. Asserted through the binary, because that is where the flag's parsing
/// and its reporting live.
#[test]
fn the_converter_reports_json_records_on_stdout() {
    let t = temp_root("convert_json");
    let work = t.subdir("w");
    build_sled_db(&work);
    let src = work.join("legacy-cache");
    let dst = work.join("girpr-cache");

    let run = |extra: &[&str]| -> (i32, String, String) {
        let mut args: Vec<&std::ffi::OsStr> = vec![src.as_os_str(), dst.as_os_str()];
        args.extend(extra.iter().map(std::ffi::OsStr::new));
        run_converter(args)
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

/// The flags are accepted on either side of the two paths, and `--output=json`
/// is the same as `--output json`.
///
/// The parser is `clap`'s, so its spelling is the spelling and no narrower set
/// of forms sits behind it. The forms are pinned here because a run that
/// refused one of them would fail on the spelling rather than on the
/// conversion — and the spelling is what a script writes.
///
/// Every form prints the same record, and the two spellings of `--output`
/// print it byte for byte: they are one run and not two.
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
    // side of the paths, and `--force` on either side of them.
    //
    // Each form gets a destination of its own, so no form has to force over
    // the cache another one left, and every record is about its own run.
    let forms: &[(&[&str], &[&str])] = &[
        (&[], &["--output", "json"]),
        (&["--output", "json"], &[]),
        (&[], &["--output=json"]),
        (&["--output=json"], &[]),
        (&["--force", "--output", "json"], &[]),
        (&["--output", "json"], &["--force"]),
    ];

    for (i, (before, after)) in forms.iter().enumerate() {
        let dst = work.join(format!("girpr-cache-{i}"));
        let (code, stdout, stderr) = run_converter(args_of(before, after, &dst));
        assert_eq!(
            code, 0,
            "the form {before:?} before the paths and {after:?} after them: stderr: {stderr}"
        );
        assert_eq!(
            parse_ndjson(stdout.as_bytes()),
            vec![serde_json::json!({
                "event": "converted",
                "src": src.display().to_string(),
                "dst": dst.display().to_string(),
                "files": 2,
                "dirs": 1,
                "hashes": 5,
            })],
            "the form {before:?} before the paths and {after:?} after them: \
             the record is the one every spelling prints"
        );
    }

    // The two spellings of `--output` are the same run: the same destination,
    // the same record, byte for byte.
    let dst = work.join("girpr-cache-spellings");
    let (code, spaced, stderr) = run_converter(args_of(&["--output", "json"], &[], &dst));
    assert_eq!(code, 0, "stderr: {stderr}");
    let (code, equals, stderr) = run_converter(args_of(&["--force", "--output=json"], &[], &dst));
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(spaced, equals, "the two spellings are the same run");
}

/// A usage error is a failure like any other, and fails with the converter's
/// own code.
///
/// `clap` exits `2` for a parse error, and the main binary keeps that. This
/// binary does not: it has one exit code for every way it can fail, so a
/// script that ran the converter needs no second code to remember.
#[test]
fn a_usage_error_exits_three() {
    let (code, _stdout, stderr) = run_converter(["--not-a-flag"]);
    assert_eq!(code, 3, "stderr: {stderr}");
    assert!(
        stderr.contains("--not-a-flag"),
        "the error names the flag it did not know: {stderr}"
    );
}

/// Both positionals are required: the sled directory is read, and the redb
/// file is what is written.
#[test]
fn a_missing_positional_exits_three() {
    let (code, _stdout, stderr) = run_converter(["only-one"]);
    assert_eq!(code, 3, "stderr: {stderr}");
    assert!(
        stderr.contains("REDB_PATH"),
        "the error names the argument it did not get: {stderr}"
    );
}

/// `--output` still takes only the two formats it always took, and says which
/// when the value is not one of them.
///
/// The paths are placeholders: the arguments are rejected before anything is
/// read or written, so no fixture is needed for a case that never converts.
#[test]
fn an_unknown_output_format_exits_three_and_names_the_accepted_values() {
    let (code, _stdout, stderr) = run_converter(["sled", "cache.redb", "--output", "yaml"]);
    assert_eq!(code, 3, "stderr: {stderr}");
    assert!(stderr.contains("text"), "the error names text: {stderr}");
    assert!(stderr.contains("json"), "the error names json: {stderr}");
}

/// `--help` is an answer, not a failure: it exits 0 and prints the flags.
#[test]
fn the_help_exits_zero_and_documents_the_flags() {
    let (code, stdout, stderr) = run_converter(["--help"]);
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(
        stdout.contains("--force"),
        "the help names --force: {stdout}"
    );
    assert!(
        stdout.contains("--output"),
        "the help names --output: {stdout}"
    );
    // The order of the two positionals is part of the contract: the sled
    // directory is read, the redb file is written.
    let sled = stdout
        .find("SLED_DIR")
        .expect("the help names the sled dir");
    let redb = stdout
        .find("REDB_PATH")
        .expect("the help names the redb file");
    assert!(sled < redb, "the sled dir comes first: {stdout}");
}

/// The help output, pinned whole.
///
/// `clap` reads a doc comment on the argument struct as the command's
/// `long_about`, and `--help` prints it **verbatim** — the `**` markers and
/// the `#[command(version)]` attribute in it included, because `clap` does not
/// render markdown. A note written for a source reader would then tell a user
/// about an attribute they cannot type, so the struct carries no doc comment
/// and this case is what holds it there.
///
/// The pin is the whole text rather than a few names in it, so anything that
/// reaches the help shows up as a difference: a paragraph of prose, a flag, a
/// changed wording. The names the case above looks for would survive all of
/// them.
///
/// Two things are normalized before the comparison, and neither is what the
/// pin is about: the program name, which carries the host's executable suffix
/// (`sled2redb.exe` on Windows, `sled2redb` elsewhere), and the trailing
/// spaces `clap` leaves on the blank line inside the possible values.
#[test]
fn the_help_output_is_pinned() {
    let (code, stdout, stderr) = run_converter(["--help"]);
    assert_eq!(code, 0, "stderr: {stderr}");
    let text = stdout
        .replace("sled2redb.exe", "sled2redb")
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(
        text, EXPECTED_HELP,
        "the help output is pinned: a doc comment on the argument struct would \
         print here"
    );
}

/// The help output of the converter, as `--help` prints it.
///
/// The program name is the host's spelling without the executable suffix, and
/// the trailing spaces are gone: the case above normalizes both before it
/// compares.
const EXPECTED_HELP: &str = "\
Convert a legacy sled cache directory into a redb cache file

Usage: sled2redb [OPTIONS] <SLED_DIR> <REDB_PATH>

Arguments:
  <SLED_DIR>
          Legacy sled cache directory

  <REDB_PATH>
          Destination redb file

Options:
      --force
          Replace an existing destination

      --output <OUTPUT>
          How the conversion record is rendered

          Possible values:
          - text: One human-readable line per record: `MISSING a.txt`, `COPY a.txt`, `SUMMARY renamed=2 mkdir=1 ...`. The default, and the format the README's examples are written in
          - json: One JSON object per line (NDJSON), same records, same order: `{\"event\":\"missing\",\"path\":\"a.txt\"}`

          [default: text]

  -h, --help
          Print help (see a summary with '-h')";
