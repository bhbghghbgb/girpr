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

use anyhow::{Context, Result};
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
/// scan file; an omitted flag keeps the detected scan-file set. An **absent**
/// scan file (or an unconfigured scan dir) defaults to `en-us` so automation
/// gets a launchable game; an **unreadable** one is an error, never a silent
/// default. An empty but readable file is a real selection (game-only).
///
/// `explicit` is `Some(langs)` when `--audio` was passed; the caller is then
/// responsible for writing the selection back (subject to read-only gating).
pub fn resolve_effective_audio(
    game_dir: &Path,
    scan_dir: &str,
    explicit: Option<&HashSet<String>>,
) -> Result<HashSet<String>> {
    match explicit {
        Some(langs) => {
            if langs.is_empty() {
                tracing::info!("explicit game-only audio selection (--audio none)");
            } else {
                tracing::info!("explicit audio langs: {langs:?}");
            }
            Ok(langs.clone())
        }
        None => match read_scan_file(game_dir, scan_dir)? {
            Some(langs) => {
                tracing::info!("keeping current audio langs: {langs:?}");
                Ok(langs)
            }
            None => {
                // No file is the documented "not configured" case. An empty
                // file is *not* this case: it is the game-only selection
                // `--audio none` writes, and substituting `en-us` would undo
                // the user's choice and re-plan that language's manifests.
                let langs = HashSet::from(["en-us".to_string()]);
                tracing::info!("no audio scan file; defaulting to {langs:?}");
                Ok(langs)
            }
        },
    }
}

/// Read the game's audio scan file into canonical codes.
///
/// The three outcomes the caller must be able to tell apart:
/// - `Ok(None)` — no scan dir configured, or the file is not there. The
///   caller applies the documented `en-us` default.
/// - `Ok(Some(set))` — the file was read. An empty set is a real selection
///   (game-only), not "unreadable".
/// - `Err` — the file exists but could not be read, or it contains text but
///   none of it names a language. Treating either as absence would silently
///   replace the user's selection with `en-us`; treating the second as
///   game-only would drop the selected language from the purge keep set.
pub fn read_scan_file(game_dir: &Path, scan_dir: &str) -> Result<Option<HashSet<String>>> {
    if scan_dir.is_empty() {
        return Ok(None);
    }
    let p = crate::util::contained_join(game_dir, scan_dir).context("audio scan file path")?;
    let text = match std::fs::read_to_string(&p) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(e).with_context(|| format!("read audio scan file {}", p.display()));
        }
    };
    let (set, unrecognized) = parse_scan_text(&text);
    if set.is_empty() && !unrecognized.is_empty() {
        // Non-empty text with no recognized language is a malformed selection,
        // not `--audio none`. The empty set would make `select_manifests` drop
        // every audio manifest and hide the selected language from the purge
        // keep set, so `--purge-after` would delete its packages.
        return Err(anyhow::anyhow!(
            "audio scan file {} contains no recognized language (first unrecognized line: {:?}); \
             expected one of Chinese, English(US), Japanese, Korean, or an empty file for game-only",
            p.display(),
            unrecognized[0]
        ));
    }
    for line in unrecognized {
        tracing::warn!(
            path = %p.display(),
            line = %line,
            "unrecognized line in the audio scan file; ignored"
        );
    }
    Ok(Some(set))
}

/// Parse the scan file's text into canonical codes plus the non-blank lines
/// that named none of the four display names (in file order).
///
/// Blank lines carry no selection and are not reported: an empty file is the
/// game-only choice `--audio none` writes. A single leading UTF-8 BOM is
/// stripped first, because `char::is_whitespace` does not treat U+FEFF as
/// whitespace and it would hide the first display name.
fn parse_scan_text(text: &str) -> (HashSet<String>, Vec<&str>) {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut set = HashSet::new();
    let mut unrecognized: Vec<&str> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let code = match line {
            "Chinese" => Some("zh-cn"),
            s if s.eq_ignore_ascii_case("English(US)") || s.eq_ignore_ascii_case("English") => {
                Some("en-us")
            }
            "Japanese" => Some("ja-jp"),
            "Korean" => Some("ko-kr"),
            _ => None,
        };
        match code {
            Some(c) => {
                set.insert(c.to_string());
            }
            None => unrecognized.push(line),
        }
    }
    (set, unrecognized)
}

/// Write the selection back as display names. An empty set writes an empty file,
/// which is how the game is told to launch game-only.
///
/// The write is atomic (temp in the same directory, then rename): a crash or a
/// full disk mid-write must not leave a truncated file, because a truncated
/// file reads as an empty selection and would silently turn a configured
/// language set into game-only.
pub fn write_scan_file(game_dir: &Path, scan_dir: &str, langs: &HashSet<String>) -> Result<()> {
    let lines: Vec<&str> = DISPLAY
        .iter()
        .filter(|(code, _)| langs.contains(*code))
        .map(|(_, name)| *name)
        .collect();
    let p = crate::util::contained_join(game_dir, scan_dir)?;
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create audio scan dir {}", parent.display()))?;
    }
    let tmp = p.with_extension("girpr_tmp");
    let published = std::fs::write(&tmp, lines.join("\n"))
        .with_context(|| format!("write audio scan tmp {}", tmp.display()))
        .and_then(|()| {
            std::fs::rename(&tmp, &p)
                .with_context(|| format!("publish audio scan file {}", p.display()))
        });
    if let Err(e) = published {
        if let Err(cleanup) = std::fs::remove_file(&tmp)
            && cleanup.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(
                path = %tmp.display(),
                error = %cleanup,
                "could not remove the audio scan temp file"
            );
        }
        return Err(e);
    }
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
        assert_eq!(
            read_scan_file(&d, "Audio/scan.txt").unwrap(),
            Some(all),
            "a successful read is distinct from absence"
        );
    }

    #[test]
    fn game_only_writes_an_empty_scan_file_and_reads_back_empty() {
        let d = dir("gameonly");
        write_scan_file(&d, "scan.txt", &HashSet::new()).unwrap();
        let text = std::fs::read_to_string(d.join("scan.txt")).unwrap();
        assert!(text.is_empty(), "{text}");
        assert_eq!(
            read_scan_file(&d, "scan.txt").unwrap(),
            Some(HashSet::new()),
            "empty is the game-only selection, not absence"
        );
    }

    #[test]
    fn english_aliases_and_unconfigured_dir_are_tolerated() {
        let d = dir("aliases");
        std::fs::write(d.join("scan.txt"), "English\nChinese\n\nnonsense\n").unwrap();
        let set = read_scan_file(&d, "scan.txt").unwrap().unwrap();
        assert_eq!(set.len(), 2);
        assert!(set.contains("en-us") && set.contains("zh-cn"));
        assert_eq!(
            read_scan_file(&d, "").unwrap(),
            None,
            "no scan dir configured is absence"
        );
        assert_eq!(
            read_scan_file(&d, "missing.txt").unwrap(),
            None,
            "a missing file is absence"
        );
    }

    /// R-2: `--audio none` writes a deliberately empty file. The next unflagged
    /// run must keep game-only, not substitute `en-us` and re-plan that
    /// language's manifests.
    #[test]
    fn an_empty_file_written_by_audio_none_stays_game_only() {
        let d = dir("none_then_unflagged");
        write_scan_file(&d, "scan.txt", &HashSet::new()).unwrap();
        let resolved = resolve_effective_audio(&d, "scan.txt", None).unwrap();
        assert!(
            resolved.is_empty(),
            "the user's game-only choice must survive an unflagged run: {resolved:?}"
        );
        // Absence still takes the documented automation default.
        let absent = resolve_effective_audio(&d, "missing.txt", None).unwrap();
        assert_eq!(absent, HashSet::from(["en-us".to_string()]));
    }

    /// R-2: an unreadable scan file is an error, not a silent `en-us`. A
    /// directory at the scan path is the portable way to make the read fail
    /// with something other than `NotFound`.
    #[test]
    fn an_unreadable_scan_file_is_an_error_not_the_default() {
        let d = dir("unreadable");
        std::fs::create_dir_all(d.join("scan.txt")).unwrap();
        let err = read_scan_file(&d, "scan.txt").expect_err("a directory cannot be read as a file");
        let chain = format!("{err:#}");
        assert!(
            chain.contains("scan.txt"),
            "the read error must name the file: {chain}"
        );
        let err = resolve_effective_audio(&d, "scan.txt", None)
            .expect_err("an unreadable file must not default to en-us");
        assert!(format!("{err:#}").contains("scan.txt"), "{err:#}");
    }

    /// R-8: a non-blank file that names no known language is not the game-only
    /// selection. Reading it as an empty set drops the real language from the
    /// purge keep set; name the file and the offending line instead.
    #[test]
    fn an_all_unrecognized_file_is_an_error_that_names_the_line() {
        let d = dir("all_unknown");
        std::fs::write(d.join("scan.txt"), "English (US)\n").unwrap();
        let err = read_scan_file(&d, "scan.txt").expect_err("no line was recognized");
        let chain = format!("{err:#}");
        assert!(
            chain.contains("scan.txt"),
            "the error must name the file: {chain}"
        );
        assert!(
            chain.contains("English (US)"),
            "the error must quote the first unrecognized line: {chain}"
        );
        assert!(
            resolve_effective_audio(&d, "scan.txt", None).is_err(),
            "a malformed file must never resolve to game-only"
        );
    }

    /// R-8: a UTF-8 BOM is not whitespace, so the first display name used to
    /// match nothing and a one-language file read as game-only.
    #[test]
    fn a_utf8_bom_does_not_hide_the_first_language() {
        let d = dir("bom");
        std::fs::write(d.join("scan.txt"), "\u{feff}Chinese\n").unwrap();
        let set = read_scan_file(&d, "scan.txt").unwrap().unwrap();
        assert_eq!(
            set,
            HashSet::from(["zh-cn".to_string()]),
            "the BOM is stripped before the line is parsed"
        );
    }

    /// The tolerant case R-8 keeps: one unknown line beside a known one is
    /// skipped, and `read_scan_file` warns for each unknown line. The warning
    /// itself is not captured here: tracing caches a callsite's interest
    /// globally, so another test hitting the same `warn!` without a subscriber
    /// first would make the capture order-dependent.
    #[test]
    fn an_unrecognized_line_beside_a_known_one_is_skipped() {
        let (set, unrecognized) = parse_scan_text("Japanese\nKlingon\n");
        assert_eq!(set, HashSet::from(["ja-jp".to_string()]));
        assert_eq!(unrecognized, vec!["Klingon"]);
    }

    /// The scan file is written through a temp in the same directory and then
    /// renamed, so a failed write cannot leave a truncated selection behind.
    #[test]
    fn the_scan_file_is_published_atomically() {
        let d = dir("atomic");
        write_scan_file(&d, "scan.txt", &HashSet::new()).unwrap();
        write_scan_file(&d, "scan.txt", &HashSet::from(["ja-jp".to_string()])).unwrap();
        assert_eq!(
            std::fs::read_to_string(d.join("scan.txt")).unwrap(),
            "Japanese"
        );
        assert!(
            !d.join("scan.girpr_tmp").exists(),
            "the temp is renamed away, not left beside the file"
        );
    }

    /// The scan dir is server data and the write creates its parent. An escape
    /// must not create or write anything outside the game dir; the read path
    /// refuses it instead of calling it "absent".
    #[test]
    fn a_scan_path_that_escapes_the_game_dir_is_not_read_or_written() {
        let d = dir("escape");
        let set: HashSet<String> = ["en-us".to_string()].into_iter().collect();
        assert!(write_scan_file(&d, "../outside.txt", &set).is_err());
        assert!(!d.parent().unwrap().join("outside.txt").exists());
        assert!(
            read_scan_file(&d, "../outside.txt").is_err(),
            "an escaping path is an error, not an absent file"
        );
    }
}
