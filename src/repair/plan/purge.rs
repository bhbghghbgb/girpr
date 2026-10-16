//! Collapse-style files-cleanup (v1 scope) — docs/02 steps 5 and 7.
//!
//! There is exactly **one** cleanup: `--purge-before` and `--purge-after` call
//! [`plan_purge`] then [`apply_purge`]; only the timing differs. It is a
//! set-difference of the files on disk against the live manifest, the same idea
//! as Collapse's `GetUnusedFileInfoList`.
//!
//! The walk and the delete are **separate calls** (docs/05 M0), so the whole
//! decision — which files, which directories, how many bytes — exists before
//! anything is deleted. That is what makes "how much would this purge delete?"
//! answerable (`--max-delete-bytes`, docs/04 B4) and printable (`--print-plan`,
//! docs/04 C1). Nothing else about the cleanup changed.
//!
//! The keep rules are a pure function ([`classify_purge_path`]) so they can be
//! unit-tested exhaustively without touching a filesystem.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::Result;
use walkdir::WalkDir;

use super::RepairPlan;
use crate::repair::scope::PathScope;

/// One file the files-cleanup has decided to delete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PurgeCandidate {
    /// `/`-separated rel path, i.e. exactly what `classify_purge_path` saw.
    pub rel: String,
    /// Absolute-or-game-relative path to remove.
    pub path: PathBuf,
    pub bytes: u64,
}

/// The complete, unexecuted files-cleanup decision (docs/05 M0).
///
/// Built by `plan_purge`, consumed by `apply_purge`. Everything the run will
/// delete is here before the first byte is deleted.
#[derive(Debug, Clone, Default)]
pub struct PurgePlan {
    /// Delete candidates, sorted by path so a rerun's log is comparable.
    pub files: Vec<PurgeCandidate>,
    /// Directories the purge empties, deepest first; `game_dir` excluded.
    pub dirs: Vec<PathBuf>,
    /// `files` summed — the number a delete-bytes ceiling is checked against.
    pub total_bytes: u64,
    /// On-disk files kept **only** because the path scope hides them. Zero on
    /// an unflagged run; the A1 guard-3 warn is driven off this.
    pub hidden: u64,
}

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
/// NEVER appear in chunk manifests.
///
/// Pure with respect to the walk's contents: it reads the filesystem and
/// decides, but deletes nothing.
pub fn plan_purge(
    game_dir: &Path,
    plan: &RepairPlan,
    server_keep: &HashSet<String>,
    scope: &PathScope,
) -> PurgePlan {
    let expected = plan.expected_paths();
    let mut files: Vec<PurgeCandidate> = Vec::new();
    let mut all_dirs: Vec<PathBuf> = Vec::new();
    // Ancestors of every `Keep`-classified file: anything else is left empty by
    // the purge and eligible for the emptied-dir sweep.
    let mut kept_dirs: HashSet<PathBuf> = HashSet::new();
    let mut hidden = 0u64;

    for e in WalkDir::new(game_dir)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if e.file_type().is_dir() {
            all_dirs.push(e.path().to_path_buf());
            continue;
        }
        if !e.file_type().is_file() {
            continue;
        }
        let path = e.path();
        let rel = rel_path(path, game_dir);
        let outside = !scope.is_visible(&rel);
        match classify_purge_path(&rel, &expected, server_keep, scope) {
            PurgeVerdict::Keep => mark_kept_ancestors(path, game_dir, &mut kept_dirs),
            PurgeVerdict::Purge => {
                debug_assert!(!outside, "a hidden path must classify Keep");
                let bytes = e.metadata().map(|m| m.len()).unwrap_or(0);
                files.push(PurgeCandidate {
                    rel,
                    path: path.to_path_buf(),
                    bytes,
                });
            }
        }
        if outside {
            hidden += 1;
        }
    }

    // Deterministic delete order, so a rerun's log is comparable.
    files.sort_by(|a, b| a.path.cmp(&b.path));
    let mut dirs: Vec<PathBuf> = all_dirs
        .into_iter()
        .filter(|d| *d != game_dir && !kept_dirs.contains(d))
        .collect();
    dirs.sort_by_key(|a| std::cmp::Reverse(a.components().count()));
    let total_bytes = files.iter().map(|c| c.bytes).sum();
    // docs/04 A1 guard 3: the scope and a purge flag agreeing is correct, but
    // it should be visible. `plan_purge` runs once per purge pass, so a run
    // with both `--purge-before` and `--purge-after` says this twice — one line
    // per decision, which is what actually happened.
    if hidden > 0 {
        tracing::warn!(
            "{hidden} on-disk paths are outside the active --include/--exclude scope and will not be deleted"
        );
    }
    PurgePlan {
        files,
        dirs,
        total_bytes,
        hidden,
    }
}

/// Carry "this directory still has something to keep" up from a kept file to
/// `game_dir`, so the emptied-dir sweep never sweeps a dir whose only contents
/// were kept. `game_dir` itself is excluded (it is never removed).
fn mark_kept_ancestors(file: &Path, game_dir: &Path, out: &mut HashSet<PathBuf>) {
    let Some(mut dir) = file.parent() else {
        return;
    };
    while dir != game_dir {
        if !dir.starts_with(game_dir) {
            return;
        }
        out.insert(dir.to_path_buf());
        let Some(parent) = dir.parent() else {
            return;
        };
        dir = parent;
    }
}

/// Execute a decided [`PurgePlan`]. Returns the number of bytes actually
/// deleted, which is always `0` in dry-run (the candidates are only logged,
/// and the sweep is skipped entirely).
pub fn apply_purge(plan: &PurgePlan, dry_run: bool) -> Result<u64> {
    let mut deleted_bytes = 0u64;
    let mut would_be = 0u64;
    for c in &plan.files {
        if dry_run {
            would_be += c.bytes;
            tracing::info!("would purge extra {} ({} bytes)", c.path.display(), c.bytes);
        } else if std::fs::remove_file(&c.path).is_ok() {
            deleted_bytes += c.bytes;
            tracing::info!("purged extra {} ({} bytes)", c.path.display(), c.bytes);
        }
    }
    if dry_run {
        tracing::info!(
            "purge dry-run: {would_be} bytes would be freed (deleted_extra_bytes stays 0 by design)"
        );
    } else {
        remove_emptied_dirs(&plan.dirs);
    }
    Ok(deleted_bytes)
}

/// Remove the directories the purge emptied, deepest first. The set is already
/// derived from the walk ([`PurgePlan::dirs`]), so this no longer re-walks; only
/// the `remove_dir` calls remain, which still fail harmlessly if something
/// appeared since the plan was built. `game_dir` is never in the set.
fn remove_emptied_dirs(dirs: &[PathBuf]) {
    for d in dirs {
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
/// [`super::build_plan`] and the purge walk).
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
/// 0. **Path scope** (docs/04 §2): a path outside `--include`/`--exclude`
///    does not exist, so it is kept — *before* the manifest check, not after.
///    Order is load-bearing: `plan.expected_paths()` is the only
///    manifest-derived input to the keep set, so a scope applied anywhere but
///    here would leave the cleanup with a *narrower* keep set than an
///    unfiltered run, and the fixture `gircap` hands us would be eaten.
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
    scope: &PathScope,
) -> PurgeVerdict {
    // 0. Out of scope → does not exist.
    if !scope.is_visible(rel) {
        return PurgeVerdict::Keep;
    }
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
        plan_of_all(&[rel])
    }

    fn plan_of_all(rels: &[&str]) -> RepairPlan {
        RepairPlan {
            latest: "5.0".into(),
            files: rels
                .iter()
                .map(|rel| PlannedFile {
                    rel: (*rel).into(),
                    size: 4,
                    md5: String::new(),
                    chunks: vec![],
                })
                .collect(),
            url_prefix_by_file: HashMap::new(),
        }
    }

    fn purge_now(dir: &Path, plan: &RepairPlan, dry_run: bool) -> u64 {
        apply_purge(
            &plan_purge(dir, plan, &keep_set(), &PathScope::default()),
            dry_run,
        )
        .unwrap()
    }

    /// Compile a scope for the classifier tests.
    fn scope(include: &[&str], exclude: &[&str]) -> PathScope {
        let inc: Vec<String> = include.iter().map(|s| s.to_string()).collect();
        let exc: Vec<String> = exclude.iter().map(|s| s.to_string()).collect();
        PathScope::new(&inc, &exc).unwrap()
    }

    #[test]
    fn purge_keeps_manifest_and_config_only() {
        let (expected, keep) = purge_fixture();
        let open = PathScope::default();
        for rel in ["game.dat", "config.ini"] {
            assert_eq!(
                classify_purge_path(rel, &expected, &keep, &open),
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
                classify_purge_path(rel, &expected, &keep, &open),
                PurgeVerdict::Purge,
                "{rel} must be purged"
            );
        }
    }

    #[test]
    fn purge_keeps_audio_version_files_case_insensitively() {
        let (expected, keep) = purge_fixture();
        let open = PathScope::default();
        // Collapse GenshinInstall.cs:232-241 parity (audio_lang_14 + per-lang pkg_version).
        for rel in [
            "audio_lang_14",
            "Audio_English_pkg_version",
            "audio_chinese_pkg_version",
            "AUDIO_JAPANESE_PKG_VERSION",
        ] {
            assert_eq!(
                classify_purge_path(rel, &expected, &keep, &open),
                PurgeVerdict::Keep,
                "{rel} must be kept"
            );
        }
    }

    #[test]
    fn purge_purges_temps_like_any_other_orphan() {
        let (expected, keep) = purge_fixture();
        let open = PathScope::default();
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
                classify_purge_path(rel, &expected, &keep, &open),
                PurgeVerdict::Purge,
                "{rel} must be purged"
            );
        }
    }

    /// The load-bearing case of docs/04 §2: a path the scope hides must be
    /// `Keep` **whatever** the other rules would say — a manifest path, a plain
    /// orphan, a temp, and an audio bookkeeping file alike.
    #[test]
    fn a_hidden_path_is_kept_whatever_the_other_rules_say() {
        let (expected, keep) = purge_fixture();
        let scoped = scope(&["game/foo.dat"], &[]);
        for rel in [
            "game.dat",      // rule 1 would keep it anyway
            "login.dat",     // rule "everything else purges"
            "a.dat_tmp",     // temps purge like any orphan
            "audio_lang_14", // rule 3 would keep it anyway
            "ScreenShot/s.png",
            "YuanShen.exe",
        ] {
            assert_eq!(
                classify_purge_path(rel, &expected, &keep, &scoped),
                PurgeVerdict::Keep,
                "{rel} is outside the scope and must not be touched"
            );
        }
        // ...and the one admitted path is classified exactly as unfiltered.
        assert_eq!(
            classify_purge_path("game/foo.dat", &expected, &keep, &scoped),
            PurgeVerdict::Purge
        );
    }

    #[test]
    fn an_exclude_only_scope_also_protects_orphans() {
        let (expected, keep) = purge_fixture();
        let scoped = scope(&[], &["ScreenShot/*"]);
        assert_eq!(
            classify_purge_path("ScreenShot/s.png", &expected, &keep, &scoped),
            PurgeVerdict::Keep
        );
        assert_eq!(
            classify_purge_path("login.dat", &expected, &keep, &scoped),
            PurgeVerdict::Purge
        );
    }

    #[test]
    fn purge_dry_run_deletes_nothing_but_reports() {
        let dir = std::env::temp_dir().join("girpr_test_purge_dryrun");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("game.dat"), b"keep").unwrap();
        std::fs::write(dir.join("stray.dat"), b"drop").unwrap();
        let bytes = purge_now(&dir, &plan_of("game.dat"), true);
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
        let bytes = purge_now(&dir, &plan_of("game.dat"), false);
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
        purge_now(&dir, &plan, false);
        assert!(!dir.join("emptied").exists(), "emptied dir tree removed");
        assert!(dir.exists(), "game_dir itself survives");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn purge_keeps_dirs_that_still_hold_a_manifest_file() {
        let dir = std::env::temp_dir().join("girpr_test_purge_kept_dirs");
        std::fs::create_dir_all(dir.join("deep/nested")).unwrap();
        std::fs::write(dir.join("deep/nested/game.dat"), b"keep").unwrap();
        std::fs::write(dir.join("orphan.dat"), b"x").unwrap();
        let plan = plan_of("deep/nested/game.dat");
        let pp = plan_purge(&dir, &plan, &keep_set(), &PathScope::default());
        // `deep` has a kept descendant, so neither it nor `deep/nested` may be
        // swept, even though every sibling of the kept file is being purged.
        assert!(
            !pp.dirs.iter().any(|d| d.ends_with("deep")),
            "dir with a kept file must not be swept: {:?}",
            pp.dirs
        );
        purge_now(&dir, &plan, false);
        assert!(dir.join("deep").exists(), "kept dir survives");
        assert!(dir.join("deep/nested/game.dat").exists());
        assert!(!dir.join("orphan.dat").exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn plan_purge_totals_bytes_and_deletes_nothing() {
        let dir = std::env::temp_dir().join("girpr_test_purge_plan");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("game.dat"), b"keep").unwrap();
        std::fs::write(dir.join("a.dat"), b"12345").unwrap();
        std::fs::write(dir.join("b.dat"), b"123").unwrap();
        let pp = plan_purge(
            &dir,
            &plan_of("game.dat"),
            &keep_set(),
            &PathScope::default(),
        );
        let rels: Vec<&str> = pp.files.iter().map(|c| c.rel.as_str()).collect();
        assert_eq!(rels, vec!["a.dat", "b.dat"], "sorted by path");
        assert_eq!(pp.total_bytes, 8);
        assert_eq!(pp.hidden, 0, "an open scope hides nothing");
        // Planning alone mutates nothing (M0's whole point).
        assert!(dir.join("a.dat").exists());
        assert!(dir.join("b.dat").exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The case a scope has to get right for it not to be destructive: a purge
    /// under a scope must leave the whole fixture on disk, *and* leave its
    /// directories alone — a hidden path must count as "kept", so the
    /// emptied-dir sweep never sweeps the tree it lives in.
    #[test]
    fn a_purge_under_a_scope_does_not_delete_or_sweep_anything_outside_it() {
        let dir = std::env::temp_dir().join("girpr_test_purge_scoped");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("game")).unwrap();
        std::fs::create_dir_all(dir.join("adopted")).unwrap();
        // Two manifest paths, one admitted and one hidden, plus two hidden
        // orphans. Under `--include game/foo.dat` every one of them survives.
        std::fs::write(dir.join("game/foo.dat"), b"f").unwrap();
        std::fs::write(dir.join("game/bar.dat"), b"b").unwrap();
        std::fs::write(dir.join("adopted/stray.dat"), b"y").unwrap();
        std::fs::write(dir.join("login.dat"), b"z").unwrap();

        let scoped = scope(&["game/foo.dat"], &[]);
        let pp = plan_purge(
            &dir,
            &plan_of_all(&["game/foo.dat", "game/bar.dat"]),
            &keep_set(),
            &scoped,
        );
        assert_eq!(pp.total_bytes, 0, "nothing is a delete candidate");
        assert_eq!(pp.hidden, 3, "bar.dat + stray.dat + login.dat hidden");
        apply_purge(&pp, false).unwrap();

        assert_eq!(std::fs::read(dir.join("game/foo.dat")).unwrap(), b"f");
        assert_eq!(
            std::fs::read(dir.join("game/bar.dat")).unwrap(),
            b"b",
            "an out-of-scope *manifest* path is still kept"
        );
        assert!(
            dir.join("adopted").is_dir(),
            "a dir whose only file was hidden must not be swept"
        );
        assert_eq!(std::fs::read(dir.join("adopted/stray.dat")).unwrap(), b"y");
        assert_eq!(std::fs::read(dir.join("login.dat")).unwrap(), b"z");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_scope_that_admits_an_orphan_still_deletes_it() {
        let dir = std::env::temp_dir().join("girpr_test_purge_scope_purge");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("game.dat"), b"keep").unwrap();
        std::fs::write(dir.join("stray.dat"), b"drop").unwrap();
        // `--include "*.dat"` admits the orphan; it is still not in the
        // manifest, so it purges.
        let scoped = scope(&["*.dat"], &[]);
        let pp = plan_purge(&dir, &plan_of("game.dat"), &keep_set(), &scoped);
        assert_eq!(pp.total_bytes, 4);
        assert_eq!(pp.hidden, 0);
        assert_eq!(apply_purge(&pp, false).unwrap(), 4);
        assert!(!dir.join("stray.dat").exists());
        assert!(dir.join("game.dat").exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn dry_run_leaves_dirs_alone() {
        let dir = std::env::temp_dir().join("girpr_test_purge_dirs_dry");
        std::fs::create_dir_all(dir.join("empty_dir")).unwrap();
        std::fs::write(dir.join("orphan.dat"), b"x").unwrap();
        let plan = plan_of("game.dat");
        purge_now(&dir, &plan, true);
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
