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
//! describe a file in the bundle. So is a `.` component and a component ending
//! in `.` or a space: Windows folds `a/b.` and `a/b ` onto `a/b`, so three
//! distinct entries would name one file and the last write would win. A
//! reserved DOS device name (`CON`, `NUL`, `COM1`…) is refused for the same
//! portability reason: Windows opens the device, not a file, whatever the
//! extension. The caller decides how to report the refusal: a flag is a
//! [`crate::cli::UsageError`], a document is an [`crate::http::ApiError::shape`].
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
    relative_path_rule(value).map(|(_, problem)| problem)
}

/// [`relative_path_problem`] with the rule's stable identifier.
///
/// The prose is for a human reading a failure; the identifier is for
/// `diagnostics/rejections.json`, where a consumer wants to count refusals by
/// rule without matching message text that may be reworded.
pub fn relative_path_rule(value: &str) -> Option<(&'static str, String)> {
    path_rule(value, false)
}

/// Why `value` cannot be a **logical** bundle path, or `None` when it can.
///
/// A logical path is one that appears in a document: a manifest entry's `file`,
/// a patchmap's `path`, the index's `referenced_by[].path`. It is the path the
/// game itself uses, so it is allowed to contain a Windows device name — the
/// bundle writes that component under an escaped on-disk name and records the
/// mapping in `diagnostics/path_map.json` (see [`escape_component`]).
///
/// Everything else the write rule refuses is still refused: a logical path that
/// escapes the bundle root, uses a backslash, or folds onto another name is not
/// a path the game could be using.
pub fn logical_path_problem(value: &str) -> Option<String> {
    logical_path_rule(value).map(|(_, problem)| problem)
}

/// [`logical_path_problem`] with the rule's stable identifier.
pub fn logical_path_rule(value: &str) -> Option<(&'static str, String)> {
    path_rule(value, true)
}

fn path_rule(value: &str, allow_device: bool) -> Option<(&'static str, String)> {
    if value.is_empty() {
        return Some(("path.empty", format!("{value:?} is an empty path")));
    }
    if value.chars().any(char::is_control) {
        return Some((
            "path.control",
            format!("{value:?} contains a control character; a bundle path is printable text"),
        ));
    }
    if value.contains('\\') {
        return Some((
            "path.backslash",
            format!("{value:?} uses backslashes; bundle paths are /-separated"),
        ));
    }
    if value.starts_with('/') {
        return Some((
            "path.absolute",
            format!("{value:?} is absolute; paths are relative to the bundle root"),
        ));
    }
    if value.contains(':') {
        return Some((
            "path.drive",
            format!(
                "{value:?} looks like a drive-qualified path; paths are relative to the \
                 bundle root"
            ),
        ));
    }
    if value.split('/').any(|c| c == "..") {
        return Some(("path.parent", format!("{value:?} escapes the bundle root")));
    }
    if value.split('/').any(|c| c == ".") {
        return Some((
            "path.dot-component",
            format!(
                "{value:?} has a `.` component; a bundle path names something in the \
                 bundle, and `.` names the directory it is in"
            ),
        ));
    }
    if value
        .split('/')
        .any(|c| c.ends_with('.') || c.ends_with(' '))
    {
        return Some((
            "path.windows-fold",
            format!(
                "{value:?} has a component ending in `.` or a space; Windows folds such \
                 a name onto the one without it, so two paths could name one file"
            ),
        ));
    }
    if !allow_device && value.split('/').any(is_windows_device_name) {
        return Some((
            "path.device",
            format!(
                "{value:?} names a Windows device; such a component opens the device \
                 instead of a file, so the bundle would not hold the bytes it lists"
            ),
        ));
    }
    if value.split('/').any(|c| c.is_empty()) {
        return Some((
            "path.empty-component",
            format!("{value:?} has an empty component (a doubled or trailing separator)"),
        ));
    }
    None
}

/// Whether one path component is a reserved DOS device name.
///
/// Windows resolves these names whatever the extension, so `CON.txt` opens the
/// console and a write to it creates no file — the bundle would list bytes it
/// does not hold. The list is Microsoft's (`CON`, `PRN`, `AUX`, `NUL`, `COM1`–
/// `COM9`, `LPT1`–`LPT9`, and the `$` console aliases). `COM0`/`LPT0` are not
/// reserved, so only 1–9 count. A leading dot makes a different stem, and
/// `.con` is an ordinary (if odd) file name.
///
/// Shared with the tag rule ([`crate::config`]): a tag is the component
/// `manifests/<tag>`, so a device name there is the same failure one directory
/// up.
pub(crate) fn is_windows_device_name(component: &str) -> bool {
    let stem = component.split('.').next().unwrap_or(component);
    let stem = stem.to_ascii_uppercase();
    matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || (stem.len() == 4
        && (stem.starts_with("COM") || stem.starts_with("LPT"))
        && matches!(stem.as_bytes()[3], b'1'..=b'9'))
}

/// Escape one path component so it cannot name a Windows device.
///
/// **The scheme.** A component whose stem is a reserved device name is written
/// with a leading `%`: `CON` becomes `%CON`, and `CON.txt` becomes `%CON.txt`.
/// Every other `%` is doubled, so a literal component `%CON` becomes `%%CON`.
/// The two forms are distinct, which is the whole point: without doubling the
/// `%`, the device name `CON` and the ordinary name `%CON` would both write to
/// `%CON` and the last write would win.
///
/// The encoding changes nothing about the documents: `entries[].file`,
/// `patchmaps/*.json`'s `path` and the index's references keep the game's own
/// spelling. It changes only the file on disk, and
/// [`crate::pathmap::PathMapDocument`] records each logical→on-disk pair.
///
/// **Host-independent.** The rule runs wherever the capture runs, not only on
/// Windows: a bundle captured on Linux and read on Windows must already be safe.
pub fn escape_component(component: &str) -> std::borrow::Cow<'_, str> {
    if is_windows_device_name(component) {
        return std::borrow::Cow::Owned(format!("%{component}"));
    }
    if component.contains('%') {
        return std::borrow::Cow::Owned(component.replace('%', "%%"));
    }
    std::borrow::Cow::Borrowed(component)
}

/// Escape every component of a `/`-separated bundle path.
pub fn escape_path(path: &str) -> String {
    path.split('/')
        .map(|c| escape_component(c).into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// Reverse [`escape_component`].
///
/// A leading single `%` marks an escaped device name; anywhere else `%%` stands
/// for one literal `%`. The forms [`escape_component`] produces are always
/// unambiguous, so this is the exact inverse on them.
pub fn unescape_component(component: &str) -> std::borrow::Cow<'_, str> {
    if let Some(rest) = component.strip_prefix('%')
        && !rest.starts_with('%')
    {
        return std::borrow::Cow::Owned(rest.to_string());
    }
    if component.contains("%%") {
        return std::borrow::Cow::Owned(component.replace("%%", "%"));
    }
    std::borrow::Cow::Borrowed(component)
}

/// Reverse [`escape_path`].
pub fn unescape_path(path: &str) -> String {
    path.split('/')
        .map(|c| unescape_component(c).into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// Why `value` cannot be a manifest field name, or `None` when it can.
///
/// A `matching_field` is `game` or an audio language code, and it is
/// interpolated into `manifests/<tag>/<field>.manifest…`. The path rule already
/// refuses separators of the Windows flavour, traversal, absolute paths and the
/// aliases Windows folds; this adds the one shape the path rule deliberately
/// allows but a field cannot have: `/`. A field names one file-name component,
/// not a subdirectory.
pub fn field_component_problem(value: &str) -> Option<String> {
    field_component_rule(value).map(|(_, problem)| problem)
}

/// [`field_component_problem`] with the rule's stable identifier.
pub fn field_component_rule(value: &str) -> Option<(&'static str, String)> {
    if let Some(problem) = relative_path_rule(value) {
        return Some(problem);
    }
    if value.contains('/') {
        return Some((
            "field.multi-component",
            format!("{value:?} contains a `/`; a manifest field is one file-name component"),
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
    chunk_id_rule(value).map(|(_, problem)| problem)
}

/// [`chunk_id_problem`] with the rule's stable identifier.
pub fn chunk_id_rule(value: &str) -> Option<(&'static str, String)> {
    if value.is_empty() {
        return Some(("chunk-id.empty", format!("{value:?} is an empty chunk id")));
    }
    let Some((prefix, suffix)) = value.split_once('_') else {
        return Some(("chunk-id.shape", not_a_chunk_id(value)));
    };
    let hex = |s: &str, len: usize| s.len() == len && s.bytes().all(|b| b.is_ascii_hexdigit());
    if hex(prefix, 16) && hex(suffix, 32) {
        None
    } else {
        Some(("chunk-id.shape", not_a_chunk_id(value)))
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
    fn a_manifest_field_is_one_file_name_component() {
        for good in ["game", "en-us", "zh-cn", "ko-kr"] {
            assert_eq!(field_component_problem(good), None, "{good:?}");
        }
        for bad in ["a/b", "/abs", "a\\b", "../x", "..", ""] {
            assert!(
                field_component_problem(bad).is_some(),
                "{bad:?} must be refused as a field"
            );
        }
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
            ("./x", "`.` component"),
            ("a/./b", "`.` component"),
            ("a/b.", "folds"),
            ("a/b ", "folds"),
            ("a./b", "folds"),
            ("a /b", "folds"),
            ("a//b", "empty component"),
            ("a/", "empty component"),
            ("CON", "device"),
            ("a/aux.txt", "device"),
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

    /// Windows strips a trailing `.` or space from every component, so `a/b`,
    /// `a/b.` and `a/b ` are three manifest entries that create one file — and
    /// the last write wins. On a host that does not fold them, the capture would
    /// instead produce three files, so a bundle captured here would not
    /// materialise the same way elsewhere. The rule is host-independent: refuse
    /// the shape on every platform, as the tag validator already does for `7.1.0.`.
    #[test]
    fn path_components_windows_folds_onto_one_file_are_refused() {
        for bad in [
            "a/b.", "a/b ", "a/b...", "a/b   ", "a./b", "a /b", "a/./b", "./a", "a/.",
        ] {
            assert!(
                relative_path_problem(bad).is_some(),
                "{bad:?} names a file Windows spells without the dot or space"
            );
        }
        // The rule is about *trailing* dots and spaces; a dot inside a name is
        // ordinary (`archive.tar.zst`, `7.1.0`) and must stay legal.
        for good in ["a/b.c", "a/7.1.0", "a/.hidden", "a/b c", "a/b c/d"] {
            assert_eq!(relative_path_problem(good), None, "{good:?}");
        }
    }

    /// Windows resolves a reserved DOS device name whatever the extension, so
    /// `game/NUL` or `game/aux.txt` opens a device rather than creating a file.
    /// The write can "succeed" while the bundle holds nothing to hash, which is
    /// the same silent-corruption class as a folded name.
    #[test]
    fn a_windows_device_name_is_not_a_file_name() {
        for bad in [
            "CON",
            "con",
            "a/NUL",
            "a/aux.txt",
            "a/Prn.log",
            "a/COM1",
            "a/com9.bin",
            "a/LPT3",
            "a/CONIN$",
            "a/conout$.txt",
        ] {
            let problem = relative_path_problem(bad)
                .unwrap_or_else(|| panic!("{bad:?} names a device, not a file"));
            assert!(problem.contains("device"), "{bad:?}: {problem}");
        }
        // Names that merely start like one are ordinary files.
        for good in ["CONSOLE", "a/conifer", "a/COM0", "a/LPT10", "a/.con"] {
            assert_eq!(relative_path_problem(good), None, "{good:?}");
        }
    }

    /// A logical path is the game's own spelling, so a device component is
    /// accepted there and escaped on disk. Everything a write must still refuse
    /// stays refused.
    #[test]
    fn a_logical_path_may_name_a_device_but_still_may_not_leave_the_bundle() {
        assert_eq!(logical_path_problem("a/CON"), None);
        assert_eq!(logical_path_problem("a/aux.txt"), None);
        assert_eq!(logical_path_problem("a/%CON"), None);
        for bad in ["../x", "a\\b", "/abs", "C:/x", "a/./b", "a/b.", "a//b", ""] {
            assert!(
                logical_path_problem(bad).is_some(),
                "{bad:?} must still be refused as a logical path"
            );
        }
    }

    /// The scheme has to be an exact inverse on everything it produces, because
    /// two logical paths that collide on disk are the corruption escaping exists
    /// to prevent.
    #[test]
    fn escaping_a_path_round_trips_and_separates_devices_from_literal_percents() {
        for (logical, on_disk) in [
            ("a/b", "a/b"),
            ("a/CON", "a/%CON"),
            ("a/CON.txt", "a/%CON.txt"),
            ("a/nul", "a/%nul"),
            ("a/%CON", "a/%%CON"),
            ("a/100%done.bin", "a/100%%done.bin"),
            ("a/%%", "a/%%%%"),
            ("CON", "%CON"),
        ] {
            assert_eq!(escape_path(logical), on_disk, "{logical:?}");
            assert_eq!(unescape_path(on_disk), logical, "{on_disk:?}");
            // The escaped form is safe to write.
            assert_eq!(
                relative_path_problem(on_disk),
                None,
                "{on_disk:?} is not a safe write path"
            );
        }
        // A device and a literal percent do not collide.
        assert_ne!(escape_path("CON"), escape_path("%CON"));
    }

    #[test]
    fn an_ordinary_component_is_borrowed_not_copied() {
        assert!(matches!(
            escape_component("GenshinImpact_Data"),
            std::borrow::Cow::Borrowed(_)
        ));
        assert!(matches!(
            escape_component("CON"),
            std::borrow::Cow::Owned(_)
        ));
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
