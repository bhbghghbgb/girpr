//! Byte-level fixture for the recorded request/response JSON.
//!
//! The artifact bundle pins the JSON gircap writes under `api/`. Rewriting the
//! HTTP pipeline must not change one byte of it. Two fields are per-run facts
//! and are scrubbed before the comparison, exactly as the bundle differ does:
//! `ts` (a millisecond clock stamp) and `elapsed_ms` (a measured duration).
//! Everything else — request and response header maps, `content-type`, body
//! lengths, `stored_as`, `seq`, `status` — is compared byte for byte against
//! the golden strings in [`recorded_json_golden`].

mod common;
mod recorded_json_golden;

use std::path::{Path, PathBuf};

use common::{Mock, World, dispatch_with};
use girpr_tempdir::TempDir;

use recorded_json_golden::{ERROR_RESPONSES_JSON, ERROR_SHAPES_JSON, REQUESTS_JSON};

/// Replace each `"ts":"…"` and `"elapsed_ms":<digits>` value with a fixed
/// spelling, leaving every other byte of the document untouched.
fn scrub_volatile(input: &str) -> String {
    fn replace_field(input: &str, key: &str, replacement: &str) -> String {
        let needle = format!("\"{key}\":");
        let mut out = String::with_capacity(input.len());
        let mut rest = input;
        while let Some(at) = rest.find(&needle) {
            out.push_str(&rest[..at + needle.len()]);
            let after = &rest[at + needle.len()..];
            if let Some(body) = after.strip_prefix('"') {
                // A JSON string value. The timestamp contains no escapes.
                let end = body.find('"').expect("a terminated JSON string");
                out.push_str(replacement);
                rest = &body[end + 1..];
            } else {
                // A number value: consume the digits.
                let end = after
                    .find(|c: char| !c.is_ascii_digit())
                    .unwrap_or(after.len());
                out.push_str(replacement);
                rest = &after[end..];
            }
        }
        out.push_str(rest);
        out
    }
    let s = replace_field(input, "ts", "\"<TS>\"");
    replace_field(&s, "elapsed_ms", "0")
}

fn find_bundle(out: &Path) -> PathBuf {
    fn walk(dir: &Path, found: &mut Vec<PathBuf>) {
        if let Ok(rd) = std::fs::read_dir(dir) {
            for entry in rd.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, found);
                } else {
                    found.push(path);
                }
            }
        }
    }
    let mut files = Vec::new();
    walk(out, &mut files);
    files
        .into_iter()
        .find(|p| p.file_name().is_some_and(|n| n == "MANIFEST.json"))
        .map(|p| p.parent().expect("bundle has a parent").to_path_buf())
        .expect("a bundle under the output root")
}

fn capture_recorded_json() -> Vec<u8> {
    let scratch = TempDir::new("gircap-recorded-json", "fixture");
    let out = scratch.join("artifacts");
    let mock = Mock::in_memory(World::all_shapes());
    let args: Vec<String> = vec![
        "capture".into(),
        "--out".into(),
        out.to_string_lossy().into_owned(),
        "--hyp-base".into(),
        mock.hyp_base(),
        "--sophon-base".into(),
        mock.sophon_base(),
        // Concurrency must not change which bytes are recorded or their order.
        "--jobs".into(),
        "4".into(),
    ];
    let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
    let dispatch = dispatch_with(&borrowed, Some(mock.transport()));
    assert_eq!(dispatch.code, 0, "capture failed: {:?}", dispatch.detail);

    let bundle = find_bundle(&out);
    let mut all = Vec::new();
    for rel in [
        "api/requests.ndjson",
        "api/error_responses.json",
        "api/error_shapes.json",
    ] {
        let raw = std::fs::read(bundle.join(rel)).expect(rel);
        let text = std::str::from_utf8(&raw).expect("recorded JSON is UTF-8");
        all.extend_from_slice(scrub_volatile(text).as_bytes());
        all.push(b'\n');
    }
    all
}

/// The pinned recorded JSON, in the same order as [`capture_recorded_json`].
fn golden() -> Vec<u8> {
    let mut all = Vec::new();
    for doc in [REQUESTS_JSON, ERROR_RESPONSES_JSON, ERROR_SHAPES_JSON] {
        all.extend_from_slice(doc.as_bytes());
        all.push(b'\n');
    }
    all
}

#[test]
fn recorded_request_and_response_json_is_byte_identical() {
    assert_eq!(
        capture_recorded_json(),
        golden(),
        "recorded request/response JSON drifted from the artifact-bundle fixture"
    );
}
