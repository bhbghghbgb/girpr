//! The band-1 gate: **two captures of the same fixture must agree**, once the
//! fields that were never deterministic are normalised away.
//!
//! `cargo nextest run` proves a refactor did not break the *code*. It does not
//! prove it did not change what a bundle **says** — and that is the risk that
//! matters here, because a bundle is a fixture other work depends on. A
//! refactor that silently dropped a field or rewrote `MANIFEST.json` would pass
//! 450 tests and still be wrong. This file is the check that cannot see that
//! difference, and it is what makes the band-1 and band-2 work routine instead
//! of scary.
//!
//! ## Why it is "identical after normalisation", not "identical"
//!
//! Two captures of one input are **never** byte-identical. Measured, not
//! assumed — this harness exists partly because the list of what moves had to be
//! found by running it:
//!
//! | what | where | why it moves |
//! |---|---|---|
//! | the bundle directory's own name | `<old>__<new>/<capture_id>/` | `capture_id` is the UTC second the run started |
//! | `captured <ts>` | `README.md` | the same moment, second precision |
//! | `ts` | `api/requests.ndjson` | one RFC 3339 stamp per request |
//! | `elapsed_ms` | `api/requests.ndjson` | a measured duration |
//! | `argv`'s `--out` | `MANIFEST.json` | records the command line, which names *this run's* scratch directory |
//!
//! ## The cascade, and why three documents are exempted
//!
//! `api/requests.ndjson` is not byte-reproducible, and `checksums.json` covers
//! it (A16). So its `bytes`/`md5`/`sha256` differ — and because three documents
//! *record* that file's size or digest, they differ too:
//!
//! ```text
//!   api/requests.ndjson            (the root cause)
//!     -> MANIFEST.json   files[] row for it, and `bytes_on_disk`
//!     -> README.md       the "N B on disk" total
//!     -> verify/checksums.json   rows for the two above, plus `total_bytes`
//! ```
//!
//! `DERIVED_PATHS` names exactly those three rows. For them this harness compares
//! **structure with the known-dependent numbers blanked**, not bytes — which is
//! what keeps the other ~55 files, including every blob and every assembled
//! `game/` file, under a byte-exact comparison.
//!
//! `verify/selfcheck.json` used to be the fourth, blanking A16's "N files, M B".
//! It no longer states a byte total at all — the row lives inside one of the
//! counted documents, so any total written there describes an earlier, smaller
//! tree — and with nothing volatile left in it the whole document is compared
//! byte-for-byte.
//!
//! Two properties keep the remaining exemption honest:
//!
//! * every blank must **match something**, or the harness fails — so a refactor
//!   that renames a field cannot silently turn "compare 3 documents with blanks"
//!   into "compare 3 documents with nothing to compare";
//! * `verify/` is not exempt from *being right* — the existing suites assert its
//!   contents, and a real bundle is checked with `gircap verify` by hand.
//!
//! ## Why `--jobs 1`
//!
//! With `--jobs > 1` the **order** of chunk rows in `api/requests.ndjson` varies
//! run to run: `seq` is reserved when a request is issued, so which chunk gets
//! `seq` 7 depends on which of four workers wakes first. That is real, and it is
//! not what this gate is for. Pinning `--jobs 1` makes issue order deterministic
//! and leaves the comparison to measure the thing it was built to measure.
//! (Worth knowing for the concurrency work: `--jobs > 1` does **not** preserve
//! the `seq` values of a run, only the set of rows.)
//!
//! ## Strictness
//!
//! Normalisation rewrites **values in place, on the raw bytes**. It does not
//! round-trip through `serde_json`, which would sort the keys and hide exactly
//! the drift this gate exists to catch. A reformatted `MANIFEST.json` still
//! fails. `.gz` payloads are compared decompressed, because `hash.rs` already
//! states that flate2 output is not promised byte-stable and `checksums.json`
//! records the decompressed digest for that reason.
//!
//! ## No network
//!
//! Two captures of `World::all_shapes()` against the loopback mock, in a
//! scratch directory removed on drop. See `tests/common/mod.rs`: the mock is the
//! only listener and binds `127.0.0.1:0`, so there is no route off the host.

mod common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use common::{Mock, World};
use girpr_tempdir::TempDir;

/// The binary under test, beside this test's own executable.
fn bin() -> PathBuf {
    let mut p = std::env::current_exe().expect("the test binary has a path");
    p.pop(); // deps/
    if p.ends_with("deps") {
        p.pop();
    }
    p.join(format!("gircap{}", std::env::consts::EXE_SUFFIX))
}

/// A scratch root that is removed when the test ends, whatever happens.
type Scratch = TempDir;

fn run(args: &[&str]) -> (i32, String, String) {
    let out = Command::new(bin())
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("running {args:?}: {e}"));
    (
        out.status.code().expect("the process was not killed"),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// Capture `World::all_shapes()` into `<scratch>/<slot>/`.
fn capture_into(mock: &Mock, scratch: &Scratch, slot: &str) -> PathBuf {
    let out = scratch.join(slot);
    let args = [
        "capture",
        "--out",
        out.to_str().expect("scratch path is utf-8"),
        "--hyp-base",
        &mock.hyp_base(),
        "--sophon-base",
        &mock.sophon_base(),
        // See the module doc: concurrency permutes the chunk rows in the request
        // log, which is orthogonal to what this gate measures.
        "--jobs",
        "1",
    ];
    let (code, stdout, stderr) = run(&args);
    assert_eq!(code, 0, "capture into {slot} failed:\n{stdout}\n{stderr}");
    bundle_in(&out)
}

fn bundle_in(out: &Path) -> PathBuf {
    let mut found = Vec::new();
    walk(out, &mut found);
    found
        .into_iter()
        .find(|p| p.is_dir() && p.join("MANIFEST.json").is_file())
        .unwrap_or_else(|| panic!("no bundle under {}", out.display()))
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        out.push(p.clone());
        if p.is_dir() {
            walk(&p, out);
        }
    }
}

// ------------------------------------------------------------- normalisation

/// What one run's bundle knows about itself, and must therefore stop asserting.
struct Volatile<'a> {
    /// The bundle directory's name: a UTC stamp, unique per run.
    capture_id: &'a str,
    /// The `--out` root this run was given, which `argv` records verbatim.
    out_root: &'a str,
}

/// The rows whose `bytes`/`md5`/`sha256` are a digest of a non-reproducible
/// file, and are therefore blanked rather than compared.
///
/// See the module doc for the cascade. Adding a path here is a claim that its
/// digest cannot be reproduced; removing one is a claim that it can.
const DERIVED_PATHS: [&str; 3] = ["api/requests.ndjson", "MANIFEST.json", "README.md"];

/// Rewrite the fields that differ between two runs of one input.
///
/// **In place, on the raw bytes**, so key order, indentation and every other
/// byte are still compared — and byte-wise throughout, because a chunk blob is
/// arbitrary bytes and a chunk that happened to contain a non-ASCII byte must
/// survive the harness unchanged.
fn normalize(body: &[u8], v: &Volatile<'_>) -> Vec<u8> {
    let escaped = v.out_root.replace('\\', "\\\\");
    let out = replace_all(body, v.out_root.as_bytes(), b"<OUT>");
    let out = replace_all(&out, escaped.as_bytes(), b"<OUT>");
    let out = replace_all(&out, v.capture_id.as_bytes(), b"<CAPTURE_ID>");
    let out = scrub_timestamps(&out);
    scrub_elapsed(&out)
}

/// Replace every occurrence of `find` with `to`.
fn replace_all(hay: &[u8], find: &[u8], to: &[u8]) -> Vec<u8> {
    if find.is_empty() {
        return hay.to_vec();
    }
    let mut out = Vec::with_capacity(hay.len());
    let mut i = 0;
    while i < hay.len() {
        if hay[i..].starts_with(find) {
            out.extend_from_slice(to);
            i += find.len();
        } else {
            out.push(hay[i]);
            i += 1;
        }
    }
    out
}

/// Replace every RFC 3339 instant with `<TS>`, at both precisions the bundle
/// uses: millisecond in `requests.ndjson`, second in `README.md`.
fn scrub_timestamps(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match ts_len(b, i) {
            Some(len) => {
                out.extend_from_slice(b"<TS>");
                i += len;
            }
            None => {
                out.push(b[i]);
                i += 1;
            }
        }
    }
    out
}

/// The length of an RFC 3339 instant at `i`, if one starts there.
///
/// `2026-10-16T18:03:39Z` or `2026-10-16T18:03:39.720Z`.
///
/// Both precisions are needed and neither is optional: the request log stamps
/// milliseconds, `README.md` and `MANIFEST.json` stamp seconds. Getting only
/// one right makes this harness fail at random — once a pair of captures
/// happens to straddle a second boundary — which is the worst possible failure
/// mode for a gate.
fn ts_len(b: &[u8], i: usize) -> Option<usize> {
    const DATE_TIME: usize = 19; // 2026-10-16T18:03:39
    let t = b.get(i..i + DATE_TIME)?;
    let d = |k: usize| t[k].is_ascii_digit();
    let shaped = d(0)
        && d(1)
        && d(2)
        && d(3)
        && t[4] == b'-'
        && d(5)
        && d(6)
        && t[7] == b'-'
        && d(8)
        && d(9)
        && t[10] == b'T'
        && d(11)
        && d(12)
        && t[13] == b':'
        && d(14)
        && d(15)
        && t[16] == b':'
        && d(17)
        && d(18);
    if !shaped {
        return None;
    }
    let rest = b.get(i + DATE_TIME..)?;
    // Longest match first: the fractional form starts with the same `.` that
    // would otherwise be read as "not an instant".
    if let Some([b'.', a, b2, c, b'Z']) = rest.get(..5)
        && a.is_ascii_digit()
        && b2.is_ascii_digit()
        && c.is_ascii_digit()
    {
        return Some(DATE_TIME + 5);
    }
    (rest.first() == Some(&b'Z')).then_some(DATE_TIME + 1)
}

/// Replace `"elapsed_ms":<digits>` with `"elapsed_ms":<MS>`.
fn scrub_elapsed(b: &[u8]) -> Vec<u8> {
    const KEY: &[u8] = b"\"elapsed_ms\":";
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i..].starts_with(KEY) {
            out.extend_from_slice(KEY);
            out.extend_from_slice(b"<MS>");
            i += KEY.len();
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    out
}

/// Blank a per-file `bytes`/`md5`/`sha256` triple inside a pretty-printed
/// `files[]` / `entries[]` row, wherever it appears in `doc`.
///
/// Row-based and structure-light on purpose: it reads the document's own
/// indentation and leaves everything else exactly as written, so the comparison
/// that follows is still over the bundle's real bytes.
///
/// Returns the number of rows blanked, so the caller can insist it was more than
/// zero — an exemption that matches nothing is a silent hole in the gate.
fn blank_derived_rows(doc: &str, paths: &[&str]) -> String {
    let mut out = String::with_capacity(doc.len());
    let mut in_row = false;
    let mut blanked = 0usize;
    for line in doc.split_inclusive('\n') {
        let trimmed = line.trim();
        if trimmed.starts_with("\"path\"") {
            in_row = paths.iter().any(|p| trimmed.contains(&format!("\"{p}\"")));
            if in_row {
                blanked += 1;
            }
        }
        let derived_field = in_row
            && (trimmed.starts_with("\"bytes\"")
                || trimmed.starts_with("\"md5\"")
                || trimmed.starts_with("\"sha256\""));
        if derived_field {
            out.push_str(&blank_value(line));
        } else {
            out.push_str(line);
        }
        if in_row && trimmed.starts_with('}') {
            in_row = false;
        }
    }
    assert!(
        blanked > 0,
        "no `path` row matched {paths:?}; the exemption in this harness no longer \
         lines up with the bundle's documents and would be comparing nothing"
    );
    out
}

/// Replace the value on the right of a `"key": value` line with `<blank>`.
fn blank_value(line: &str) -> String {
    match line.find(':') {
        Some(at) => format!("{}: <blank>\n", &line[..at]),
        None => line.to_string(),
    }
}

/// Blank one named numeric field's value, keeping the line otherwise intact.
///
/// `expected` is the minimum number of matches: an exemption that matches
/// nothing is a silent hole in the gate, not a harmless no-op.
fn blank_number(doc: &str, key: &str, expected: usize) -> String {
    let mut out = String::with_capacity(doc.len());
    let mut hits = 0;
    for line in doc.split_inclusive('\n') {
        if line.trim_start().starts_with(&format!("\"{key}\"")) {
            out.push_str(&blank_value(line));
            hits += 1;
        } else {
            out.push_str(line);
        }
    }
    assert!(
        hits >= expected,
        "expected {expected} `{key}` line(s) to blank, blanked {hits}; the document's \
         shape has moved and this exemption would be comparing nothing"
    );
    out
}

/// Blank a byte total written inside prose: `<N> B on disk`, `N files, <M> B,`.
fn blank_prose_total(doc: &str, marker: &str, expected: usize) -> String {
    let mut out = String::with_capacity(doc.len());
    let mut hits = 0;
    for line in doc.split_inclusive('\n') {
        match marker_split(line, marker) {
            Some((before, after)) => {
                out.push_str(before);
                out.push_str("<TOTALS>");
                out.push_str(after);
                hits += 1;
            }
            None => out.push_str(line),
        }
    }
    assert!(
        hits >= expected,
        "expected {expected} `{marker}` total(s) to blank, blanked {hits}; the \
         document's shape has moved and this exemption would be comparing nothing"
    );
    out
}

/// Split `line` at the number immediately preceding `marker`, keeping the
/// surrounding text. Returns `None` when the marker is absent.
fn marker_split<'a>(line: &'a str, marker: &str) -> Option<(&'a str, &'a str)> {
    let at = line.find(marker)?;
    let head = &line[..at];
    let digits = head.len() - head.trim_end_matches(|c: char| c.is_ascii_digit()).len();
    if digits == 0 {
        return None;
    }
    Some((&head[..head.len() - digits], &line[at..]))
}

// ----------------------------------------------------------------- comparing

/// Every file under `bundle`, keyed by bundle-relative path, normalised.
fn tree(bundle: &Path, v: &Volatile<'_>) -> BTreeMap<String, Vec<u8>> {
    let mut entries = Vec::new();
    walk(bundle, &mut entries);
    let mut out = BTreeMap::new();
    for p in entries {
        if !p.is_file() {
            continue;
        }
        let rel = p
            .strip_prefix(bundle)
            .expect("walked from the bundle root")
            .to_string_lossy()
            .replace('\\', "/");
        let raw = std::fs::read(&p).expect("bundle file is readable");
        let body = if rel.ends_with(".gz") {
            // Compared decompressed: `hash.rs` does not promise stable flate2
            // output, and `checksums.json` records the decompressed digest anyway.
            gircap::hash::gunzip(&raw).expect("bundle .gz decompresses")
        } else {
            raw
        };
        out.insert(rel.clone(), normalize_doc(&rel, &body, v));
    }
    out
}

/// [`normalize`], plus the cascade exemptions, applied only to the documents
/// that actually carry them.
///
/// Every exemption is scoped to one named document and must match something, so
/// a refactor that renamed a field fails the harness loudly instead of quietly
/// turning an exemption into "compare nothing".
///
/// `verify/selfcheck.json` appears nowhere above, on purpose. It used to blank
/// A16's "N files, M B", and that blank was hiding a number rather than removing
/// one: A16's byte total was the total of the *previous* pass, because the row
/// lives inside one of the counted documents. A16 no longer states a byte total,
/// so nothing in that document is volatile and it is compared byte-for-byte.
fn normalize_doc(rel: &str, body: &[u8], v: &Volatile<'_>) -> Vec<u8> {
    let n = normalize(body, v);
    let s = String::from_utf8_lossy(&n).into_owned();
    let out = match rel {
        // The requests.ndjson row, plus the `files[]` sum it feeds.
        "MANIFEST.json" => {
            let m = blank_derived_rows(&s, &["api/requests.ndjson"]);
            blank_number(&m, "bytes_on_disk", 1)
        }
        // "N B downloaded, M B on disk".
        "README.md" => blank_prose_total(&s, " B on disk", 1),
        // The non-reproducible rows, plus the sum over all of them.
        "verify/checksums.json" => {
            let c = blank_derived_rows(&s, &DERIVED_PATHS);
            blank_number(&c, "total_bytes", 1)
        }
        _ => return n,
    };
    out.into_bytes()
}

/// A readable description of the differences between two normalised files.
fn differences(a: &[u8], b: &[u8]) -> String {
    let n = a.len().min(b.len());
    let mut out = Vec::new();
    let mut i = 0;
    while i < n {
        if a[i] != b[i] {
            let lo = i.saturating_sub(30).min(n);
            let hi = (i + 50).min(n);
            out.push(format!(
                "byte {i} ({} vs {} B)\n  A: …{}…\n  B: …{}…",
                a.len(),
                b.len(),
                String::from_utf8_lossy(&a[lo..hi]),
                String::from_utf8_lossy(&b[lo..hi]),
            ));
            i = hi;
        } else {
            i += 1;
        }
    }
    if a.len() != b.len() {
        out.push(format!("length {} vs {}", a.len(), b.len()));
    }
    if out.is_empty() {
        out.push("identical".into());
    }
    out.join("\n")
}

// ------------------------------------------------------------------- the test

/// **The gate.** Two captures of one fixture, normalised, are identical.
///
/// When this fails the message names every differing file and every differing
/// byte range in each — a refactor that changed what a bundle says is otherwise
/// very hard to see, because `cargo nextest run` is still green.
#[test]
fn two_captures_of_the_same_fixture_agree() {
    let mock = Mock::start(World::all_shapes());
    let scratch = TempDir::new("gircap-diff", "pair");

    let a = capture_into(&mock, &scratch, "a");
    let b = capture_into(&mock, &scratch, "b");

    let id = |p: &Path| p.file_name().unwrap().to_string_lossy().into_owned();
    let (a_root, b_root) = (scratch.join("a"), scratch.join("b"));
    let ta = tree(
        &a,
        &Volatile {
            capture_id: &id(&a),
            out_root: a_root.to_str().unwrap(),
        },
    );
    let tb = tree(
        &b,
        &Volatile {
            capture_id: &id(&b),
            out_root: b_root.to_str().unwrap(),
        },
    );

    // The set of files first: a file that appeared or vanished is the loudest
    // possible regression, and reads better as a set difference than as a diff.
    let only_a: Vec<&String> = ta.keys().filter(|k| !tb.contains_key(*k)).collect();
    let only_b: Vec<&String> = tb.keys().filter(|k| !ta.contains_key(*k)).collect();
    assert!(
        only_a.is_empty() && only_b.is_empty(),
        "the two bundles disagree about which files exist\n  only in A: {only_a:?}\n  only in B: {only_b:?}"
    );

    let mut diffs = Vec::new();
    for (rel, a) in &ta {
        let b = &tb[rel];
        if a != b {
            diffs.push(format!("{rel}\n  {}", differences(a, b)));
        }
    }
    assert!(
        diffs.is_empty(),
        "{} of {} files differ between two captures of one fixture:\n\n{}",
        diffs.len(),
        ta.len(),
        diffs.join("\n\n")
    );

    // Guard the gate itself, twice over.
    //
    // `capture_id` is second-resolution, so two mock captures can land on the
    // same id; what must differ is the raw request log, because `ts` is
    // millisecond-resolution and `elapsed_ms` is measured. If it did not,
    // "identical after normalisation" would be vacuously true.
    let raw_a = std::fs::read(a.join("api/requests.ndjson")).expect("requests.ndjson");
    let raw_b = std::fs::read(b.join("api/requests.ndjson")).expect("requests.ndjson");
    assert_ne!(
        raw_a, raw_b,
        "two captures must differ before normalisation, or this test proves nothing"
    );
    assert!(
        ta.len() > 20,
        "expected a bundle of real shape, got {} files",
        ta.len()
    );
}

// ------------------------------------------------------- tests of the gate

// A harness that is subtly wrong is worse than no harness: it either fails at
// random, or it passes a real regression. The tests below pin the normalisation
// down so that neither can happen quietly.

/// The per-run facts a normalisation needs.
fn volatile() -> Volatile<'static> {
    Volatile {
        capture_id: "20261016T180534Z",
        out_root: r"C:\Users\x\gircap\a",
    }
}

fn norm(s: &str) -> String {
    String::from_utf8(normalize(s.as_bytes(), &volatile())).expect("ascii in, ascii out")
}

#[test]
fn both_timestamp_precisions_are_scrubbed() {
    // The request log stamps milliseconds, the README and MANIFEST stamp
    // seconds. Scrubbing only one makes the gate fail whenever two captures
    // straddle a second boundary, which is the failure mode that would get this
    // harness switched off.
    assert_eq!(
        norm(r#"{"ts":"2026-10-16T18:05:34.482Z","at":"2026-10-16T18:05:34Z"}"#),
        r#"{"ts":"<TS>","at":"<TS>"}"#
    );
    // And a pair one second apart must normalise identically — that is the case
    // that actually bites.
    assert_eq!(
        norm("at 2026-10-16T18:05:34Z"),
        norm("at 2026-10-16T18:05:35Z")
    );
    assert_eq!(
        norm("at 2026-10-16T18:05:34.482Z"),
        norm("at 2026-10-16T18:05:35.001Z")
    );
}

#[test]
fn something_that_merely_looks_like_a_timestamp_is_left_alone() {
    for keep in [
        "2026-10-16",                // a date
        "2026-10-16T18:05",          // no seconds
        "2026-10-16 18:05:34Z",      // no `T`
        "2026-10-16T18:05:34.48Z",   // two fractional digits
        "2026-10-16T18:05:34.4822Z", // four
    ] {
        assert_eq!(norm(keep), keep, "must not be scrubbed: {keep}");
    }

    // Shape only, deliberately: the scrubber asks "does this token *look* like
    // an instant the bundle writes", not "is it a real date". Validating month
    // 13 would put calendar rules in a test harness, and the thing it would
    // protect against — a game asset whose bytes happen to spell a timestamp
    // with 13 in it — is not worth the code. `binary_bytes_survive_
    // normalisation_unchanged` covers the case that actually matters.
    assert_eq!(norm("2026-13-45T99:99:99.999Z"), "<TS>");
}

#[test]
fn the_digest_and_duration_fields_are_scrubbed() {
    assert_eq!(
        norm(r#"{"seq":8,"elapsed_ms":224,"n":7}"#),
        r#"{"seq":8,"elapsed_ms":<MS>,"n":7}"#
    );
    // Variable width on purpose: `elapsed_ms` is 1 digit as often as 4.
    assert_eq!(norm(r#"{"elapsed_ms":8}"#), r#"{"elapsed_ms":<MS>}"#);
}

#[test]
fn the_out_root_is_scrubbed_in_both_spellings() {
    // `argv` is JSON, so the path arrives with its separators escaped.
    let json = r#""C:\\Users\\x\\gircap\\a""#;
    assert_eq!(norm(json), r#""<OUT>""#);
    let plain = r"C:\Users\x\gircap\a\artifacts";
    assert_eq!(norm(plain), "<OUT>\\artifacts");
    assert_eq!(
        norm("20261016T180534Z"),
        "<CAPTURE_ID>",
        "the bundle directory's own name is the capture id"
    );
}

#[test]
fn binary_bytes_survive_normalisation_unchanged() {
    // A chunk blob is arbitrary bytes. If any of them were reinterpreted as
    // text the harness would report a difference that is not one.
    let blob: Vec<u8> = vec![0x00, 0xFF, 0x80, 0xC3, 0x28, 0xB5, 0x2F, 0xFD, 0x1B, 0x8B];
    assert_eq!(normalize(&blob, &volatile()), blob);
    let texty: Vec<u8> = b"\xC3\xA9\xE2\x82\xAC binary \x00\xFF".to_vec();
    assert_eq!(normalize(&texty, &volatile()), texty);
}

#[test]
fn an_exemption_that_stops_matching_is_a_failure_not_a_silent_hole() {
    // The shape moved: the row is gone. That has to be loud, or `blank_derived_rows`
    // would quietly stop comparing anything and the gate would become theatre.
    let doc = r#"{
  "entries": [
    {
      "path": "README.md",
      "bytes": 10,
      "md5": "aa",
      "sha256": "bb"
    }
  ]
}"#;
    let r = std::panic::catch_unwind(|| blank_derived_rows(doc, &["api/requests.ndjson"]));
    assert!(
        r.is_err(),
        "a blank that matches nothing must fail the harness"
    );
}

/// The other half: an exemption that *does* match must blank exactly the three
/// derived fields and nothing else, or it is too broad.
#[test]
fn a_matched_exemption_blanks_only_the_derived_fields() {
    let doc = r#"{
  "entries": [
    {
      "path": "README.md",
      "bytes": 10,
      "md5": "aa",
      "sha256": "bb",
      "encoding": "identity"
    },
    {
      "path": "api/requests.ndjson",
      "bytes": 20,
      "md5": "cc",
      "sha256": "dd",
      "encoding": "identity"
    }
  ]
}"#;
    let out = blank_derived_rows(doc, &["api/requests.ndjson"]);
    assert!(
        out.contains(r#""path": "README.md""#),
        "untouched row:\n{out}"
    );
    assert!(out.contains(r#""bytes": 10"#), "untouched row:\n{out}");
    assert!(
        out.contains(r#""encoding": "identity""#),
        "encoding must survive:\n{out}"
    );
    assert!(out.contains(r#""bytes": <blank>"#), "derived bytes:\n{out}");
    assert!(out.contains(r#""md5": <blank>"#), "derived md5:\n{out}");
    assert!(
        out.contains(r#""sha256": <blank>"#),
        "derived sha256:\n{out}"
    );
    assert!(
        !out.contains(r#""bytes": 20"#),
        "derived size leaked:\n{out}"
    );
}
