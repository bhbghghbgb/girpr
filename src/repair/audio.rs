use anyhow::Result;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::util::normalize_rel;

/// Effective audio langs from the on-disk audio-scan file
/// (`Chinese|English(US)|Japanese|Korean` → `zh-cn|en-us|ja-jp|ko-kr`).
pub(crate) fn read_current_audio(game_dir: &Path, scan_dir: &str) -> HashSet<String> {
    let mut set = HashSet::new();
    if scan_dir.is_empty() {
        return set;
    }
    let Ok(text) = std::fs::read_to_string(game_dir.join(normalize_rel(scan_dir))) else {
        return set;
    };
    for line in text.lines() {
        match line.trim() {
            "Chinese" => {
                set.insert("zh-cn".to_string());
            }
            s if s.eq_ignore_ascii_case("English(US)") || s.eq_ignore_ascii_case("English") => {
                set.insert("en-us".to_string());
            }
            "Japanese" => {
                set.insert("ja-jp".to_string());
            }
            "Korean" => {
                set.insert("ko-kr".to_string());
            }
            _ => {}
        }
    }
    set
}

/// Write the effective selection back to the audio-scan file (explicit `--audio`
/// only; the default "keep current" path leaves the file alone).
pub(crate) fn write_audio_scan(
    game_dir: &Path,
    scan_dir: &str,
    audio: &HashSet<String>,
) -> Result<()> {
    let mut lines: Vec<&str> = Vec::new();
    if audio.contains("zh-cn") {
        lines.push("Chinese");
    }
    if audio.contains("en-us") {
        lines.push("English(US)");
    }
    if audio.contains("ja-jp") {
        lines.push("Japanese");
    }
    if audio.contains("ko-kr") {
        lines.push("Korean");
    }
    let p = game_dir.join(normalize_rel(scan_dir));
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(p, lines.join("\n"))?;
    Ok(())
}

/// Starward-parity audio cache→res move (`GameInstallService.cs:427-444,643-660`):
/// prevents stranded duplicate audio when the game config names distinct dirs.
pub(crate) fn move_audio_cache(game_dir: &Path, cache_dir: &str, res_dir: &str) {
    let cache = game_dir.join(normalize_rel(cache_dir));
    let res = game_dir.join(normalize_rel(res_dir));
    if !cache.is_dir() {
        return;
    }
    let files: Vec<PathBuf> = walkdir::WalkDir::new(&cache)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(|e| e.path().to_path_buf())
        .collect();
    for src in files {
        if let Ok(rel) = src.strip_prefix(&cache) {
            let target = res.join(rel);
            if std::fs::create_dir_all(target.parent().unwrap_or(&res)).is_ok()
                && std::fs::rename(&src, &target).is_err()
            {
                let _ = std::fs::copy(&src, &target);
                let _ = std::fs::remove_file(&src);
            }
        }
    }
    tracing::info!("moved audio cache -> res");
}
