//! Shape rules for values that name something inside a bundle.
//!
//! **One rule, four trust boundaries.** A path that is joined into a bundle is
//! only safe if it cannot leave the directory it is joined onto: `--files` names
//! one, a manifest's `SophonChunkFile.file` names one, `chunks/index.json`'s
//! `stored_path` names where a reader will look for a blob, and
//! `getGameConfigs.audio_pkg_scan_dir` names where the fixture writes the audio
//! scan file. Those values arrive from a flag, a manifest's protobuf, a bundle
//! someone may have edited, and the API respectively — so the rule cannot live
//! in any one of those call sites.
//!
//! The rule is deliberately about *shape*, not about existence: `a/b` is legal
//! whether or not `a/b` is published. Manifest paths are `/`-separated and
//! rooted at the game directory, so a backslash, an absolute path, a drive
//! prefix, an empty component or a `..` component is a document that cannot
//! describe a file in the bundle. The caller decides how to report the refusal:
//! a flag is a [`crate::cli::UsageError`], a document is an
//! [`crate::http::ApiError::shape`].
//!
//! **A chunk id is stricter still.** It is interpolated into a CDN URL
//! ([`crate::manifest::CapturedManifest::chunk_url`]) and into
//! `chunks/blobs/<id>.zst`, and its documented shape is `<16 hex>_<32 hex>`
//! (`docs/01-artifact-schema.md` §12). That shape is also what keeps URL
//! metacharacters out of the path, not only what keeps `..` out of it.

/// Why `value` cannot be a bundle-relative path, or `None` when it can.
///
/// The message names the value and the rule, so a caller only has to add where
/// the value came from (`--files`, a manifest entry, the index).
pub fn relative_path_problem(value: &str) -> Option<String> {
    if value.is_empty() {
        return Some(format!("{value:?} is an empty path"));
    }
    if value.chars().any(char::is_control) {
        return Some(format!(
            "{value:?} contains a control character; a bundle path is printable text"
        ));
    }
    if value.contains('\\') {
        return Some(format!(
            "{value:?} uses backslashes; bundle paths are /-separated"
        ));
    }
    if value.starts_with('/') {
        return Some(format!(
            "{value:?} is absolute; paths are relative to the bundle root"
        ));
    }
    if value.contains(':') {
        return Some(format!(
            "{value:?} looks like a drive-qualified path; paths are relative to the \
             bundle root"
        ));
    }
    if value.split('/').any(|c| c == "..") {
        return Some(format!("{value:?} escapes the bundle root"));
    }
    if value.split('/').any(|c| c.is_empty()) {
        return Some(format!(
            "{value:?} has an empty component (a doubled or trailing separator)"
        ));
    }
    None
}

/// Why `value` cannot be a chunk id, or `None` when it can.
///
/// The documented shape is exactly sixteen hexadecimal digits, an underscore and
/// thirty-two hexadecimal digits. Case is not part of the identity: both cases
/// are accepted, because hex is hex and the id is opaque to this tool. What is
/// not accepted is anything else — a separator, a `.` component, a URL
/// metacharacter, or a different length — because the same string becomes both a
/// URL path segment and a file name.
pub fn chunk_id_problem(value: &str) -> Option<String> {
    if value.is_empty() {
        return Some(format!("{value:?} is an empty chunk id"));
    }
    let Some((prefix, suffix)) = value.split_once('_') else {
        return Some(not_a_chunk_id(value));
    };
    let hex = |s: &str, len: usize| s.len() == len && s.bytes().all(|b| b.is_ascii_hexdigit());
    if hex(prefix, 16) && hex(suffix, 32) {
        None
    } else {
        Some(not_a_chunk_id(value))
    }
}

fn not_a_chunk_id(value: &str) -> String {
    format!("{value:?} is not a chunk id of the documented <16 hex>_<32 hex> shape")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_manifest_path_is_legal() {
        assert_eq!(
            relative_path_problem("GenshinImpact_Data/Persistent/audio_lang_14"),
            None
        );
        assert_eq!(relative_path_problem("a"), None);
        assert_eq!(relative_path_problem("a/b/c.blk"), None);
        // Non-ASCII is a structural finding for `decode_counts`, not a path
        // escape: the rule only rejects what cannot name a file in the bundle.
        assert_eq!(relative_path_problem("a/\u{4e2d}\u{6587}"), None);
    }

    #[test]
    fn every_way_out_of_the_bundle_is_named() {
        for (value, want) in [
            ("", "empty"),
            ("../x", "escapes"),
            ("a/../../b", "escapes"),
            ("a\\..\\b", "backslashes"),
            ("a\\b", "backslashes"),
            ("/etc/passwd", "absolute"),
            ("C:/Windows", "drive-qualified"),
            ("C:evil", "drive-qualified"),
            ("a//b", "empty component"),
            ("a/", "empty component"),
            ("a\u{0}b", "control character"),
        ] {
            let problem = relative_path_problem(value).unwrap_or_else(|| {
                panic!("{value:?} must be refused");
            });
            assert!(
                problem.contains(want),
                "{value:?}: {problem:?} should mention {want:?}"
            );
        }
    }

    #[test]
    fn a_chunk_id_is_exactly_sixteen_and_thirty_two_hex_digits() {
        assert_eq!(
            chunk_id_problem("ddb31a9fd85346c4_de1cfe6927dacbc58d2d55a0b45ea0f3"),
            None
        );
        assert_eq!(
            chunk_id_problem("DDB31A9FD85346C4_DE1CFE6927DACBC58D2D55A0B45EA0F3"),
            None,
            "hex is hex: an upper-case id names the same bytes"
        );
    }

    #[test]
    fn every_way_a_chunk_id_could_leave_the_blob_directory_is_named() {
        for value in [
            "",
            "..",
            "../x",
            "a/b",
            "a\\b",
            "C:",
            "ddb31a9fd85346c4",
            "ddb31a9fd85346c4_de1cfe6927dacbc58d2d55a0b45ea0f3/../../x",
            "ddb31a9fd85346c4_de1cfe6927dacbc58d2d55a0b45ea0f",
            "ddb31a9fd85346c_de1cfe6927dacbc58d2d55a0b45ea0f3",
            "ddb31a9fd85346cg_de1cfe6927dacbc58d2d55a0b45ea0f3",
            "ddb31a9fd85346c4_de1cfe6927dacbc58d2d55a0b45ea0f3?x",
        ] {
            let problem = chunk_id_problem(value).unwrap_or_else(|| {
                panic!("{value:?} must be refused");
            });
            assert!(problem.contains("chunk id"), "{value:?}: {problem:?}");
        }
    }
}
