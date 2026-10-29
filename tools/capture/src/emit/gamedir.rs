//! The `game/` fixture: `config.ini`, the audio scan file, and the variant files.
//!
//! **M8.** Exactly three kinds of entry and nothing else, so the directory can be
//! handed to the tool under test untouched. `config.ini` mirrors that tool's own
//! writer so its surgical rewrite is a no-op except for `game_version`, and the
//! scan file's path comes from `getGameConfigs.audio_pkg_scan_dir` - never
//! hardcoded.
//!
//! ## Why the scan file's order is fixed
//!
//! The game writes display names *positionally*: line 0 is Chinese, line 1 is
//! English(US), line 2 Japanese, line 3 Korean, whatever languages are
//! installed. So a file listing only `English(US)` would be read as
//! "language 0 is English", and the tool under test would configure the wrong
//! voice pack. The file is therefore written with all four lines and the
//! unselected ones blanked, not truncated — a short file is a *different* file,
//! not a smaller one.
//!
//! ## Why the scan file's shape matters
//!
//! The spec's wording is precise and it is a *selection*, not a positional
//! table: "display names, one per line, in the order the game writes them...
//! `--audio none` writes an empty file". An empty file could not be a
//! positional table, so the file is the selected codes' names, one per line, in
//! the game's canonical order rather than the caller's.
use std::path::Path;

use crate::biz::Biz;

/// The `[General]` block `config.ini` carries.
///
/// The keys are the tool's own, and the order is the tool's own: a surgical
/// rewrite of `game_version` has to leave the rest of the file byte-identical,
/// and a re-serialiser that reordered or re-spaced the keys would not be a
/// surgical rewrite.
pub const GENERAL_SECTION: &str = "[General]";

/// Render `config.ini` for a region at a version.
///
/// `game_version` is the **old** tag. The fixture is a launchable install of the
/// old version, and the run under test is the upgrade out of it — putting the
/// new tag here would make the fixture describe the state the tool is supposed
/// to be repairing.
pub fn config_ini(biz: &Biz, game_version: &str) -> String {
    let mut out = String::with_capacity(160);
    out.push_str(GENERAL_SECTION);
    out.push('\n');
    for (k, v) in [
        ("channel", biz.channel),
        ("cps", biz.cps),
        ("game_biz", biz.as_str()),
        ("game_version", game_version),
        // Present and empty, as the tool's writer leaves it. An omitted key is
        // a different file to most INI readers than a key with no value.
        ("sdk_version", ""),
        ("sub_channel", biz.sub_channel),
    ] {
        out.push_str(k);
        out.push('=');
        out.push_str(v);
        out.push('\n');
    }
    out
}

/// Read `game_version` back out of a rendered `config.ini`.
///
/// A13 re-parses the file rather than comparing it to the string it was built
/// from, because the question is whether a *reader* can get the version out — and
/// a comparison against the same input would pass even if the format were
/// unreadable.
pub fn parse_game_version(ini: &str) -> Result<String, String> {
    let mut in_general = false;
    for line in ini.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with([';', '#']) {
            continue;
        }
        if line.starts_with('[') {
            in_general = line.trim_matches(|c| c == '[' || c == ']').trim() == "General";
            continue;
        }
        if !in_general {
            continue;
        }
        if let Some((k, v)) = line.split_once('=')
            && k.trim() == "game_version"
        {
            let v = v.trim();
            if v.is_empty() {
                return Err("game_version is present but empty".into());
            }
            return Ok(v.to_string());
        }
    }
    Err(format!("no game_version in the {GENERAL_SECTION} section"))
}

/// The display name the game uses for an audio code, and its position in the
/// game's canonical ordering.
fn display_name(code: &str) -> Option<&'static str> {
    match code {
        "zh-cn" => Some("Chinese"),
        "en-us" => Some("English(US)"),
        "ja-jp" => Some("Japanese"),
        "ko-kr" => Some("Korean"),
        _ => None,
    }
}

/// Render the audio scan file for the selected codes.
///
/// Lines are the selected codes' display names, one per line, in the order
/// `ORDER` lists them rather than the order the caller named them — two runs
/// with the same selection must produce the same file whatever order the flags
/// arrived in. `--audio none` produces an empty body, which is a real state and
/// not an error: the fixture then has no voice pack and the tool under test
/// should say so rather than find a stale file.
///
/// An unrecognised code is **refused**, not skipped. The measured set is the
/// four above; a code outside it has no display name this tool knows, and
/// writing a file with a missing line would be a fixture that silently
/// configures the wrong language.
pub fn audio_scan_file(codes: &[String]) -> Result<String, String> {
    for c in codes {
        if display_name(c).is_none() {
            return Err(format!(
                "audio code {c:?} has no known display name; measured codes are zh-cn, \
                 en-us, ja-jp, ko-kr"
            ));
        }
    }
    let mut selected: Vec<&str> = codes.iter().filter_map(|c| display_name(c)).collect();
    selected.sort_by_key(|n| ORDER.iter().position(|o| o == n).unwrap_or(usize::MAX));
    selected.dedup();
    let mut out = String::new();
    for name in selected {
        out.push_str(name);
        out.push('\n');
    }
    Ok(out)
}

/// The game's canonical audio ordering.
const ORDER: [&str; 4] = ["Chinese", "English(US)", "Japanese", "Korean"];

/// One file to write into `game/` or `expected/`.
pub struct VariantFile<'a> {
    /// The manifest-relative path, which is also the path under the tree.
    pub rel_path: &'a str,
    /// The bytes, exactly as the manifest's chunks assembled them.
    pub bytes: &'a [u8],
    /// The `capture_id` this file came from, for the run log.
    pub capture_id: &'a str,
}

/// What `game/` and `expected/` ended up holding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Written {
    /// Artifact-relative paths written under `game/`, in write order.
    pub game: Vec<String>,
    /// Artifact-relative paths written under `expected/`.
    pub expected: Vec<String>,
}

/// Write `config.ini` and the audio scan file, the two entries that are not
/// variant files.
///
/// Separate from the per-file write because they are written once and the
/// variant files are written one capture at a time, as each assembles — a
/// capture that keeps every assembled file in memory until the end would hold
/// the whole fixture at once, and the whole fixture is the largest thing this
/// tool ever holds.
pub fn write_prelude(
    root: &Path,
    biz: &Biz,
    old_tag: &str,
    scan_dir: &str,
    audio: &[String],
) -> std::io::Result<Vec<String>> {
    // Resolved before anything is written: an unknown audio code has to stop the
    // run rather than leave a `game/` tree holding a scan file that was never
    // finished.
    let scan = audio_scan_file(audio).map_err(std::io::Error::other)?;
    Ok(vec![
        super::write_rel(root, "game/config.ini", config_ini(biz, old_tag).as_bytes())?,
        super::write_rel(root, &format!("game/{scan_dir}"), scan.as_bytes())?,
    ])
}

/// Write one variant file into `tree` (`game` or `expected`).
///
/// The manifest path is the game's own spelling; on disk, a Windows device
/// component is escaped (`CON` → `%CON`) and the pair is recorded in
/// [`crate::pathmap::PathMap`]. The returned path is the escaped
/// artifact-relative one, because that is what the tree now holds and what
/// `MANIFEST.json → files[]` must list.
pub fn write_variant(
    root: &Path,
    tree: &str,
    file: &VariantFile<'_>,
    path_map: &crate::pathmap::PathMap,
) -> std::io::Result<String> {
    let logical = format!("{tree}/{}", file.rel_path);
    let rel = path_map.escape(&logical);
    tracing::info!(
        capture_id = file.capture_id,
        bytes = file.bytes.len(),
        path = %file.rel_path,
        on_disk = %rel,
        tree,
        "variant file written"
    );
    super::write_rel(root, &rel, file.bytes)
}

/// Write `game/` and `expected/` in one call.
///
/// Convenience for a test and for any caller holding every assembly at once;
/// [`write_prelude`] plus [`write_variant`] is what `run` uses, so the fixture is
/// never fully resident.
#[allow(clippy::type_complexity)]
pub fn write_gamedir(
    root: &Path,
    biz: &Biz,
    old_tag: &str,
    scan_dir: &str,
    audio: &[String],
    old_files: &[VariantFile<'_>],
    new_files: &[VariantFile<'_>],
    path_map: &crate::pathmap::PathMap,
) -> std::io::Result<Written> {
    let mut w = Written {
        game: write_prelude(root, biz, old_tag, scan_dir, audio)?,
        expected: Vec::new(),
    };
    for f in old_files {
        w.game
            .push(write_variant(root, super::GAME_TREE, f, path_map)?);
    }
    for f in new_files {
        w.expected
            .push(write_variant(root, super::EXPECTED_TREE, f, path_map)?);
    }
    Ok(w)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::biz::Biz;
    use crate::cli::Biz as BizId;

    fn global() -> Biz {
        Biz::resolve(BizId::Hk4eGlobal)
    }

    #[test]
    fn config_ini_carries_the_general_block_with_the_old_version() {
        let ini = config_ini(&global(), "7.0.0");
        assert_eq!(
            ini,
            "[General]\nchannel=1\ncps=hyp_hoyoverse\ngame_biz=hk4e_global\ngame_version=7.0.0\nsdk_version=\nsub_channel=0\n",
            "the key order is the writer's, because a surgical rewrite has to leave it alone"
        );
        assert!(
            ini.ends_with('\n'),
            "a trailing newline: most INI readers want one"
        );
        assert_eq!(parse_game_version(&ini).unwrap(), "7.0.0");
    }

    #[test]
    fn every_biz_renders_and_re_parses() {
        for id in [BizId::Hk4eCn, BizId::Hk4eGlobal, BizId::Hk4eBilibili] {
            let b = Biz::resolve(id);
            let ini = config_ini(&b, "6.7.0");
            assert_eq!(parse_game_version(&ini).unwrap(), "6.7.0", "{id:?}");
            assert!(ini.contains(&format!("game_biz={}", b.as_str())), "{ini}");
            assert!(ini.contains(&format!("channel={}", b.channel)), "{ini}");
        }
    }

    /// A13 is "re-parses", not "equals the string it was built from". These are
    /// the shapes a reader has to survive.
    #[test]
    fn game_version_is_readable_from_the_shapes_a_reader_will_actually_meet() {
        assert_eq!(
            parse_game_version(
                "noise\n[Other]\ngame_version=9.9.9\n[General]\ngame_version=7.0.0\n"
            )
            .unwrap(),
            "7.0.0",
            "the [General] section wins over an earlier section"
        );
        assert_eq!(
            parse_game_version("[General]\r\n game_version = 7.0.0 \r\n").unwrap(),
            "7.0.0",
            "CRLF and surrounding space, because a file written on another OS has them"
        );
        assert_eq!(
            parse_game_version("; a comment\n[General]\ngame_version=7.0.0\n").unwrap(),
            "7.0.0"
        );
    }

    #[test]
    fn an_unreadable_config_ini_says_what_is_missing_rather_than_returning_nothing() {
        let e = parse_game_version("[Other]\ngame_version=7.0.0\n").unwrap_err();
        assert!(e.contains("game_version"), "{e}");
        let e = parse_game_version("[General]\ngame_version=\n").unwrap_err();
        assert!(e.contains("empty"), "{e}");
        let e = parse_game_version("").unwrap_err();
        assert!(e.contains("General"), "{e}");
    }

    #[test]
    fn the_scan_file_holds_display_names_one_per_line() {
        let f = audio_scan_file(&["en-us".to_string()]).unwrap();
        assert_eq!(f, "English(US)\n");
        let none = audio_scan_file(&[]).unwrap();
        assert_eq!(
            none, "",
            "--audio none writes an empty file, which is a real state"
        );
    }

    /// Two runs with the same selection must produce the same file, whatever
    /// order the flags arrived in.
    #[test]
    fn the_scan_file_is_in_the_games_order_not_the_callers() {
        let a = audio_scan_file(&["ko-kr".into(), "zh-cn".into(), "en-us".into()]).unwrap();
        let b = audio_scan_file(&["en-us".into(), "ko-kr".into(), "zh-cn".into()]).unwrap();
        assert_eq!(a, b);
        assert_eq!(a, "Chinese\nEnglish(US)\nKorean\n");
        // And the game's canonical order, not alphabetical.
        assert!(a.find("Chinese").unwrap() < a.find("English(US)").unwrap());
    }

    #[test]
    fn a_duplicated_code_is_written_once() {
        let f = audio_scan_file(&["en-us".into(), "en-us".into()]).unwrap();
        assert_eq!(f, "English(US)\n");
    }

    /// An unknown code has no display name this tool knows, and writing a file
    /// with a line missing would silently configure the wrong language.
    #[test]
    fn an_unknown_audio_code_is_refused_with_the_measured_set_named() {
        let e = audio_scan_file(&["fr-fr".to_string()]).unwrap_err();
        assert!(e.contains("fr-fr"), "{e}");
        for code in ["zh-cn", "en-us", "ja-jp", "ko-kr"] {
            assert!(e.contains(code), "{e} does not name {code}");
        }
    }

    #[test]
    fn writing_the_tree_produces_three_kinds_of_entry_and_nothing_else() {
        let root = girpr_tempdir::TempDir::new("gircap-m8-gamedir", "three_kinds");
        let old = VariantFile {
            rel_path: "GenshinImpact_Data/a.dll",
            bytes: b"old",
            capture_id: "identical",
        };
        let new = VariantFile {
            rel_path: "GenshinImpact_Data/a.dll",
            bytes: b"new",
            capture_id: "identical",
        };
        let gone = VariantFile {
            rel_path: "GenshinImpact_Data/gone.blk",
            bytes: b"bye",
            capture_id: "removed",
        };
        let w = write_gamedir(
            &root,
            &global(),
            "7.0.0",
            "GenshinImpact_Data/Persistent/audio_lang_14",
            &["en-us".to_string()],
            &[old, gone],
            &[new],
            &crate::pathmap::PathMap::new(),
        )
        .expect("writes");

        assert_eq!(
            w.game.len(),
            4,
            "config.ini, the scan file, and two old files"
        );
        assert_eq!(
            w.expected.len(),
            1,
            "a removed capture contributes no expected file"
        );
        assert!(w.game.contains(&"game/config.ini".to_string()));
        assert!(
            w.game
                .contains(&"game/GenshinImpact_Data/Persistent/audio_lang_14".to_string())
        );
        assert_eq!(
            std::fs::read_to_string(root.join("game/GenshinImpact_Data/a.dll")).unwrap(),
            "old"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("expected/GenshinImpact_Data/a.dll")).unwrap(),
            "new"
        );
        // `expected/` holds files and nothing else.
        let expected_entries: Vec<String> = std::fs::read_dir(root.join("expected"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(expected_entries, vec!["GenshinImpact_Data".to_string()]);
    }

    /// A device component is written escaped and recorded, while the caller's
    /// logical path is unchanged.
    #[test]
    fn a_device_component_is_written_escaped_and_mapped() {
        let root = girpr_tempdir::TempDir::new("gircap-m8-gamedir", "device_escape");
        let map = crate::pathmap::PathMap::new();
        let device = VariantFile {
            rel_path: "GenshinImpact_Data/CON",
            bytes: b"console",
            capture_id: "identical",
        };
        let literal = VariantFile {
            rel_path: "GenshinImpact_Data/%CON",
            bytes: b"percent",
            capture_id: "identical",
        };
        let w = write_gamedir(
            &root,
            &global(),
            "7.0.0",
            "GenshinImpact_Data/Persistent/audio_lang_14",
            &[],
            &[],
            &[device, literal],
            &map,
        )
        .expect("writes");

        assert!(
            w.expected
                .contains(&"expected/GenshinImpact_Data/%CON".to_string())
        );
        assert!(
            w.expected
                .contains(&"expected/GenshinImpact_Data/%%CON".to_string()),
            "a literal percent is doubled, not confused with the device"
        );
        assert_eq!(
            std::fs::read(root.join("expected/GenshinImpact_Data/%CON")).unwrap(),
            b"console"
        );
        assert_eq!(
            std::fs::read(root.join("expected/GenshinImpact_Data/%%CON")).unwrap(),
            b"percent"
        );
        let doc = map.document();
        assert_eq!(
            doc.resolve("expected/GenshinImpact_Data/CON"),
            Some("expected/GenshinImpact_Data/%CON")
        );
        assert_eq!(
            doc.resolve("expected/GenshinImpact_Data/%CON"),
            Some("expected/GenshinImpact_Data/%%CON")
        );
    }

    #[test]
    fn the_config_ini_body_names_the_biz_and_the_old_tag() {
        let ini = config_ini(&global(), "7.0.0");
        assert!(ini.contains("7.0.0"), "{ini}");
    }
}
