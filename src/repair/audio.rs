//! The audio scan file (`audio_pkg_scan_dir`) — the game's own list of which
//! audio languages it should load.
//!
//! The file holds display names (`Chinese`, `English(US)`, `Japanese`,
//! `Korean`); `girpr` speaks the Sophon `matching_field` codes
//! (`zh-cn` / `en-us` / `ja-jp` / `ko-kr`) and converts at this boundary only.
//!
//! Who decides what ends up in the file (and whether it is written at all) is
//! [`resolve_effective_audio`], which needs the server-side `GameConfig`; this
//! module owns only the file format.

use std::collections::HashSet;
use std::path::Path;

use anyhow::Result;
/// Display names, in the order the game writes them.
const DISPLAY: [(&str, &str); 4] = [
    ("zh-cn", "Chinese"),
    ("en-us", "English(US)"),
    ("ja-jp", "Japanese"),
    ("ko-kr", "Korean"),
];

/// Decide the effective audio set for this run.
///
/// Explicit `--audio` (including `none` = game-only) wins and overwrites the
/// scan file; an omitted flag keeps the detected scan-file set, defaulting to
/// `en-us` when undetectable so automation gets a launchable game.
///
/// `explicit` is `Some(langs)` when `--audio` was passed; the caller is then
/// responsible for writing the selection back (subject to read-only gating).
pub fn resolve_effective_audio(
    game_dir: &Path,
    scan_dir: &str,
    explicit: Option<&HashSet<String>>,
) -> HashSet<String> {
    match explicit {
        Some(langs) => {
            if langs.is_empty() {
                tracing::info!("explicit game-only audio selection (--audio none)");
            } else {
                tracing::info!("explicit audio langs: {langs:?}");
            }
            langs.clone()
        }
        None => {
            let mut langs = read_scan_file(game_dir, scan_dir);
            if langs.is_empty() {
                langs.insert("en-us".to_string());
            }
            tracing::info!("keeping current audio langs: {langs:?}");
            langs
        }
    }
}

/// Read the game's audio scan file into canonical codes.
///
/// Unreadable or unconfigured means an empty set; the caller decides the
/// `en-us` default.
pub fn read_scan_file(game_dir: &Path, scan_dir: &str) -> HashSet<String> {
    let mut set = HashSet::new();
    if scan_dir.is_empty() {
        return set;
    }
    let Ok(p) = crate::util::contained_join(game_dir, scan_dir) else {
        tracing::warn!(
            scan_dir,
            "audio scan path escapes the game dir; treating it as unreadable"
        );
        return set;
    };
    let Ok(text) = std::fs::read_to_string(p) else {
        return set;
    };
    for line in text.lines() {
        let code = match line.trim() {
            "Chinese" => Some("zh-cn"),
            s if s.eq_ignore_ascii_case("English(US)") || s.eq_ignore_ascii_case("English") => {
                Some("en-us")
            }
            "Japanese" => Some("ja-jp"),
            "Korean" => Some("ko-kr"),
            _ => None,
        };
        if let Some(c) = code {
            set.insert(c.to_string());
        }
    }
    set
}

/// Write the selection back as display names. An empty set writes an empty file,
/// which is how the game is told to launch game-only.
pub fn write_scan_file(game_dir: &Path, scan_dir: &str, langs: &HashSet<String>) -> Result<()> {
    let lines: Vec<&str> = DISPLAY
        .iter()
        .filter(|(code, _)| langs.contains(*code))
        .map(|(_, name)| *name)
        .collect();
    let p = crate::util::contained_join(game_dir, scan_dir)?;
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(p, lines.join("\n"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use girpr_tempdir::TempDir;

    fn dir(name: &str) -> TempDir {
        TempDir::new("girpr_test_audio", name)
    }

    #[test]
    fn scan_file_roundtrips_all_langs_in_display_order() {
        let d = dir("roundtrip");
        let all: HashSet<String> = ["ko-kr", "zh-cn", "en-us", "ja-jp"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        write_scan_file(&d, "Audio/scan.txt", &all).unwrap();
        let text = std::fs::read_to_string(d.join("Audio/scan.txt")).unwrap();
        assert_eq!(text, "Chinese\nEnglish(US)\nJapanese\nKorean");
        assert_eq!(read_scan_file(&d, "Audio/scan.txt"), all);
    }

    #[test]
    fn game_only_writes_an_empty_scan_file_and_reads_back_empty() {
        let d = dir("gameonly");
        write_scan_file(&d, "scan.txt", &HashSet::new()).unwrap();
        let text = std::fs::read_to_string(d.join("scan.txt")).unwrap();
        assert!(text.is_empty(), "{text}");
        assert!(read_scan_file(&d, "scan.txt").is_empty());
    }

    #[test]
    fn english_aliases_and_unconfigured_dir_are_tolerated() {
        let d = dir("aliases");
        std::fs::write(d.join("scan.txt"), "English\nChinese\n\nnonsense\n").unwrap();
        let set = read_scan_file(&d, "scan.txt");
        assert_eq!(set.len(), 2);
        assert!(set.contains("en-us") && set.contains("zh-cn"));
        assert!(read_scan_file(&d, "").is_empty());
        assert!(read_scan_file(&d, "missing.txt").is_empty());
    }

    /// The scan dir is server data and the write creates its parent. An escape
    /// must not create or write anything outside the game dir; the read path
    /// treats it as unreadable.
    #[test]
    fn a_scan_path_that_escapes_the_game_dir_is_not_read_or_written() {
        let d = dir("escape");
        let set: HashSet<String> = ["en-us".to_string()].into_iter().collect();
        assert!(write_scan_file(&d, "../outside.txt", &set).is_err());
        assert!(!d.parent().unwrap().join("outside.txt").exists());
        assert!(read_scan_file(&d, "../outside.txt").is_empty());
    }
}
