//! Collapse-style files-cleanup (v1 scope) — docs/02 steps 5 and 7.
//!
//! There is exactly **one** cleanup: `--purge-before` and `--purge-after` call
//! [`purge_extra`]; only the timing differs. It is a set-difference of the files
//! on disk against the live manifest, the same idea as Collapse's
//! `GetUnusedFileInfoList`.
//!
//! The keep rules are a pure function ([`classify_purge_path`]) so they can be
//! unit-tested exhaustively without touching a filesystem.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::Result;
use walkdir::WalkDir;

use super::plan::RepairPlan;

/// NOTE(collapse-parity): set-difference of on-disk files vs the live manifest.
/// See https://github.com/CollapseLauncher/Collapse/blob/dc47259171794596331dffcf90db85a6ac0415ac/CollapseLauncher/Classes/InstallManagement/Genshin/GenshinInstall.cs#L177-L263
///
/// V1 GAP (deliberate, documented): Collapse's expected set is the union of
/// the Sophon primary manifests + dispatcher persistent manifests
/// (`res_versions_external`, `data_versions`) + plugin/SDK/WPF zip entries,
/// minus `EliminateUnnecessaryAssetIndex` (unselected audio) and `ctable*`.
/// v1 only uses `{latest Sophon manifest paths}` — no dispatcher, no
/// SDK/WPF/plugin zips (game launches without them; see README v1 limits).
/// Do not "fix" the missing union without reading the v1-limits section.
///
/// Temps (`*_tmp`, `*.hdiff`, `chunk/`, `ldiff/`, `staging/`, legacy `*.diff` /
/// `*deletefiles*`) are NOT skipped — they are not in the live manifest, so they
/// purge here like any other orphan, into the single `deleted_extra_bytes`
/// counter.
///
/// `server_keep` comes from [`keep_set`]: files that live inside `game_dir` but
/// NEVER appear in chunk manifests. Returns the number of bytes actually
/// deleted, which is always `0` in dry-run.
pub fn purge_extra(
    game_dir: &Path,
    plan: &RepairPlan,
    server_keep: &HashSet<String>,
    dry_run: bool,
) -> Result<u64> {
    let expected = plan.expected_paths();
    let mut candidates: Vec<(PathBuf, u64)> = Vec::new();
    for e in WalkDir::new(game_dir)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if !e.file_type().is_file() {
            continue;
        }
        let rel = rel_path(e.path(), game_dir);
        if classify_purge_path(&rel, &expected, server_keep) == PurgeVerdict::Purge {
            let sz = e.metadata().map(|m| m.len()).unwrap_or(0);
            candidates.push((e.path().to_path_buf(), sz));
        }
    }
    // Deterministic delete order, so a rerun's log is comparable.
    candidates.sort_by(|a, b| a.0.cmp(&b.0));

    let mut deleted_bytes = 0u64;
    let mut would_be = 0u64;
    for (p, sz) in candidates {
        if dry_run {
            would_be += sz;
            tracing::info!("would purge extra {} ({} bytes)", p.display(), sz);
        } else if std::fs::remove_file(&p).is_ok() {
            deleted_bytes += sz;
            tracing::info!("purged extra {} ({} bytes)", p.display(), sz);
        }
    }
    if dry_run {
        tracing::info!(
            "purge dry-run: {would_be} bytes would be freed (deleted_extra_bytes stays 0 by design)"
        );
    } else {
        remove_emptied_dirs(game_dir);
    }
    Ok(deleted_bytes)
}

/// Remove directories left empty by the purge, deepest first. Only ever removes
/// empty dirs, and never `game_dir` itself.
fn remove_emptied_dirs(game_dir: &Path) {
    let mut dirs: Vec<PathBuf> = WalkDir::new(game_dir)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_dir())
        .map(|e| e.path().to_path_buf())
        .collect();
    dirs.sort_by_key(|a| std::cmp::Reverse(a.components().count()));
    for d in dirs {
        if d == game_dir {
            continue;
        }
        let _ = std::fs::remove_dir(d);
    }
}

/// `/`-separated rel path of `p` under `game_dir`.
fn rel_path(p: &Path, game_dir: &Path) -> String {
    p.strip_prefix(game_dir)
        .unwrap_or(p)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Metadata files the files-cleanup must keep.
///
/// v1: `config.ini` ONLY. It lives inside `game_dir` but NEVER appears in
/// Sophon chunk manifests, and the tool rewrites it after patching — purging
/// it would break the version bookkeeping. Everything else (exe, blacklist /
/// res-category / audio-scan bookkeeping, `ScreenShot/`, logs, temps) is
/// intentionally NOT kept: Collapse's `GenshinInstall.GetUnusedFileInfoList`
/// protects only `FilesCleanupIgnoreList` (empty for Genshin) plus the
/// `Audio_*_pkg_version` regexes, and this tool maximizes free space — if the
/// user wants anything else kept, that needs an explicit `--keep` flag
/// (not implemented in v1).
///
/// NOTE: the game flags unknown executables inside the game folder, so the
/// tool binary itself must never live there; the exe needs no keep rule.
///
/// `audio_lang_*` / `Audio_*_pkg_version` are matched by filename pattern in
/// [`classify_purge_path`], not here, because their location varies by channel.
///
/// All entries are `/`-separated rel paths (same normalization as
/// [`super::plan::build_plan`] and the purge walk).
pub fn keep_set() -> HashSet<String> {
    HashSet::from(["config.ini".to_string()])
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PurgeVerdict {
    Keep,
    Purge,
}

/// Decide the fate of one `/`-separated rel path for the files-cleanup.
/// Pure function (no I/O) so it can be unit-tested exhaustively.
///
/// Rule order matters (first match wins); each rule names its provenance:
/// 1. Manifest membership (Collapse expected set).
/// 2. `config.ini` ([`keep_set`] — the only metadata keep).
/// 3. Collapse audio pkg-version parity (`audio_lang_*` + case-insensitive
///    `Audio_*_pkg_version`, `GenshinInstall.cs:232-241`). v1 matches broadly
///    by pattern rather than per-`audio_lang_14`-line exact regexes.
///
///    Everything else purges — including temps (`*_tmp`, `*.hdiff`, `chunk/`,
///    `ldiff/`, `staging/`, `*.diff`, `*deletefiles*`), `ScreenShot/`, logs,
///    exe and server bookkeeping files.
pub fn classify_purge_path(
    rel: &str,
    expected: &HashSet<String>,
    server_keep: &HashSet<String>,
) -> PurgeVerdict {
    // 1. In the live manifest → keep.
    if expected.contains(rel) {
        return PurgeVerdict::Keep;
    }
    // 2. config.ini → keep.
    if server_keep.contains(rel) {
        return PurgeVerdict::Keep;
    }
    // 3. Collapse audio pkg-version parity → keep.
    if is_audio_version_file(file_name(rel)) {
        return PurgeVerdict::Keep;
    }
    PurgeVerdict::Purge
}

/// Collapse `GenshinInstall.cs:232-241` parity: `audio_lang_14` (any
/// `audio_lang_*`) plus `Audio_<lang>_pkg_version` (matched case-insensitively
/// — on disk it is `Audio_English_pkg_version`, i.e. capital A).
fn is_audio_version_file(file_name: &str) -> bool {
    let lower = file_name.to_ascii_lowercase();
    lower.starts_with("audio_lang_")
        || (lower.starts_with("audio_") && lower.ends_with("_pkg_version"))
}

fn file_name(rel: &str) -> &str {
    rel.rsplit('/').next().unwrap_or(rel)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repair::plan::PlannedFile;
    use std::collections::HashMap;

    fn purge_fixture() -> (HashSet<String>, HashSet<String>) {
        let expected: HashSet<String> = ["game.dat".to_string()].into_iter().collect();
        let server_keep = keep_set();
        (expected, server_keep)
    }

    fn plan_of(rel: &str) -> RepairPlan {
        RepairPlan {
            latest: "5.0".into(),
            files: vec![PlannedFile {
                rel: rel.into(),
                size: 4,
                md5: String::new(),
                chunks: vec![],
            }],
            url_prefix_by_file: HashMap::new(),
        }
    }

    #[test]
    fn purge_keeps_manifest_and_config_only() {
        let (expected, keep) = purge_fixture();
        for rel in ["game.dat", "config.ini"] {
            assert_eq!(
                classify_purge_path(rel, &expected, &keep),
                PurgeVerdict::Keep,
                "{rel} must be kept"
            );
        }
        // v1: exe, server bookkeeping, user data all purge (maximize free
        // space; explicit --keep not implemented).
        for rel in [
            "YuanShen.exe",
            "blacklist.txt",
            "res_category.txt",
            "audio_scan.txt",
            "ScreenShot/shot.png",
            "log/output.txt",
            "login.dat",
        ] {
            assert_eq!(
                classify_purge_path(rel, &expected, &keep),
                PurgeVerdict::Purge,
                "{rel} must be purged"
            );
        }
    }

    #[test]
    fn purge_keeps_audio_version_files_case_insensitive() {
        let (expected, keep) = purge_fixture();
        // Collapse GenshinInstall.cs:232-241 parity (audio_lang_14 + per-lang pkg_version).
        for rel in [
            "audio_lang_14",
            "Audio_English_pkg_version",
            "audio_chinese_pkg_version",
            "AUDIO_JAPANESE_PKG_VERSION",
        ] {
            assert_eq!(
                classify_purge_path(rel, &expected, &keep),
                PurgeVerdict::Keep,
                "{rel} must be kept"
            );
        }
    }

    #[test]
    fn purge_purges_temps_like_any_other_orphan() {
        let (expected, keep) = purge_fixture();
        // Single files-cleanup: temps are not skipped, they purge into the
        // same counter. --purge-before and --purge-after delete the same set.
        for rel in [
            "a.dat_tmp",
            "config.ini.girpr_tmp",
            "x.hdiff",
            "chunk/abc123",
            "ldiff/p1",
            "staging/y",
            "old.diff",
            "patch.deletefiles.txt",
        ] {
            assert_eq!(
                classify_purge_path(rel, &expected, &keep),
                PurgeVerdict::Purge,
                "{rel} must be purged"
            );
        }
    }

    #[test]
    fn purge_dry_run_deletes_nothing_but_reports() {
        let dir = std::env::temp_dir().join("girpr_test_purge_dryrun");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("game.dat"), b"keep").unwrap();
        std::fs::write(dir.join("stray.dat"), b"drop").unwrap();
        let bytes = purge_extra(&dir, &plan_of("game.dat"), &keep_set(), true).unwrap();
        assert_eq!(bytes, 0);
        assert!(dir.join("stray.dat").exists());
        assert!(dir.join("game.dat").exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn purge_real_run_deletes_only_unexpected() {
        let dir = std::env::temp_dir().join("girpr_test_purge_real");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("game.dat"), b"keep").unwrap();
        std::fs::write(dir.join("login.dat"), b"drop").unwrap();
        let bytes = purge_extra(&dir, &plan_of("game.dat"), &keep_set(), false).unwrap();
        assert_eq!(bytes, 4);
        assert!(!dir.join("login.dat").exists());
        assert!(dir.join("game.dat").exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn purge_removes_emptied_dirs_but_keeps_game_dir() {
        let dir = std::env::temp_dir().join("girpr_test_purge_dirs");
        std::fs::create_dir_all(dir.join("emptied/nested")).unwrap();
        std::fs::write(dir.join("emptied/nested/orphan.dat"), b"x").unwrap();
        let plan = plan_of("game.dat");
        purge_extra(&dir, &plan, &keep_set(), false).unwrap();
        assert!(!dir.join("emptied").exists(), "emptied dir tree removed");
        assert!(dir.exists(), "game_dir itself survives");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn dry_run_leaves_dirs_alone() {
        let dir = std::env::temp_dir().join("girpr_test_purge_dirs_dry");
        std::fs::create_dir_all(dir.join("empty_dir")).unwrap();
        std::fs::write(dir.join("orphan.dat"), b"x").unwrap();
        let plan = plan_of("game.dat");
        purge_extra(&dir, &plan, &keep_set(), true).unwrap();
        assert!(
            dir.join("empty_dir").exists(),
            "dry-run must not sweep dirs"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn keep_set_is_config_only() {
        let keep = keep_set();
        assert!(keep.contains("config.ini"));
        assert_eq!(keep.len(), 1);
    }
}
