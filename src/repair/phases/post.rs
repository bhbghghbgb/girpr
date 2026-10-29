//! The post-phase: Starward-handling deletes + `config.ini` bump, then the
//! optional files-cleanup (docs/02 step 7).
//!
//! Order matters and is load-bearing:
//! 1. deprecated files (they are not in the live manifest, so the purge below
//!    would delete them too — the explicit loop keeps them cleaned even without
//!    any purge flag),
//! 2. audio cache → res move (Starward parity; prevents stranded duplicates),
//! 3. `--purge-after` files-cleanup,
//! 4. `config.ini` bump last, so the on-disk version only advances once the
//!    bytes are in place.
//!
//! The whole phase is skipped in read-only modes and whenever any file failed.
//! The deprecated list itself is **not** fetched here — it arrives as an
//! argument, having been read during the plan phase.

use std::path::Path;
use std::sync::atomic::Ordering;

use anyhow::{Context, Result};
use walkdir::WalkDir;

use crate::config::{RunCtx, RunFailure};
use crate::report::Summary;
use crate::util;

use super::metadata::Meta;
use crate::repair::phases::plan::RepairPlan;
use crate::repair::purge;
use crate::repair::scope::PathScope;

/// Run the post-phase.
///
/// Failures in the bookkeeping the launcher depends on are fatal (`exit 3`):
/// by that point the bytes are already repaired, so a half-finished
/// `config.ini` write would desync the launcher from the game. The
/// best-effort housekeeping steps — deprecated deletes, the audio cache move,
/// the cleanup's directory sweep — cannot be retried into success by failing
/// the run, so a refused delete or move is logged and the phase carries on.
///
/// `deprecated` is the server's deprecated-file list, already fetched (and
/// already possibly empty, if the call failed) during the plan phase.
///
/// **This phase owns its own `--dry-run` discipline.** `repair::run` does not
/// reach it only through `if !readonly` — `--purge-only` is a second call site —
/// and a gate every caller has to remember is not a gate. Each destructive step
/// below therefore consults `ctx.dry_run` itself.
pub async fn run(
    ctx: &RunCtx,
    game_dir: &Path,
    plan: &RepairPlan,
    meta: &Meta,
    summary: &Summary,
    deprecated: &[String],
) -> Result<PostOutcome, RunFailure> {
    let dry = ctx.dry_run;
    delete_deprecated(game_dir, deprecated, summary, &ctx.path_scope, dry);
    move_audio_cache(game_dir, &meta.config, &ctx.path_scope, dry);
    if ctx.purge_after || ctx.purge_only {
        let purge_plan = purge::plan_purge(
            game_dir,
            plan,
            &purge::keep_set(),
            &meta.blacklist,
            &ctx.path_scope,
        );
        // The post-repair re-plan is the only purge pass whose candidates were
        // not computed at assert time, so `--max-delete-bytes` is checked
        // against it again here. It can only be *smaller* than the
        // up-front plan — a successful repair promotes every `*_tmp` it
        // creates, and a run that leaves one behind has already failed (`3`) or
        // stopped (`5`), both of which skip this phase. That is a property of
        // the pipeline, not a property of the ceiling, so the check is here
        // rather than assumed: deletion is the one irreversible act in the tool.
        // The same check refuses an incomplete plan (`purge.errors > 0`) when a
        // ceiling is set, because its byte total is only a lower bound.
        if let Some(hit) = ctx.limits.check_delete(&purge_plan) {
            tracing::error!("limit exceeded: {hit}");
            if dry {
                return Ok(PostOutcome::DeletedNothing);
            }
            return Ok(PostOutcome::DeleteCeiling);
        }
        let bytes = purge::apply_purge(&purge_plan, dry).map_err(RunFailure::write)?;
        summary
            .deleted_extra_bytes
            .fetch_add(bytes, Ordering::Relaxed);
    }
    // `config.ini` is bumped last so the on-disk version only
    // advances once the bytes are in place. Two situations make that premise
    // false by construction — the bytes were deliberately *not* put in place:
    //
    // - an active path scope: the run only repaired a subset of the tree,
    // - `--purge-only`: a cleanup put no bytes in place at all.
    //
    // Bumping would be exactly the launcher desync the ordering exists to
    // prevent. Skip it, say so, and let `SUMMARY` carry `config_bumped=0` so a
    // test can assert it instead of reading the file.
    if ctx.path_scope.is_active() {
        tracing::warn!(
            "path scope active: not bumping config.ini, so its game_version still describes the un-scoped parts of the install"
        );
        return Ok(PostOutcome::Ok);
    }
    if ctx.purge_only {
        tracing::warn!("--purge-only: not bumping config.ini; no bytes were repaired");
        return Ok(PostOutcome::Ok);
    }
    let (ch, sub, cps) = ctx.biz.channel_tuple();
    util::write_config_ini(
        game_dir,
        &meta.latest,
        ctx.biz.as_str(),
        (ch, sub, cps),
        false,
    )
    .context("write config.ini")
    .map_err(RunFailure::write)?;
    Ok(PostOutcome::Ok)
}

/// How the post-phase ended. The post-phase mutates the disk, so a caller that
/// reached it through a path the `readonly` gate does not cover still needs the
/// outcome spelled out rather than assumed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostOutcome {
    /// Finished, everything it was asked to do is done.
    Ok,
    /// `--max-delete-bytes` blocked the post-repair cleanup pass. Nothing in
    /// that pass was deleted; earlier passes were already applied.
    DeleteCeiling,
    /// `--dry-run`: the pass was listed and applied nothing. Distinguishable
    /// from `DeleteCeiling` so a dry run does not report a limit it never hit.
    DeletedNothing,
}

/// Delete every path the server still lists as deprecated. The list was
/// fetched during the plan phase and is advisory: an empty list (failed call)
/// simply means there is nothing to do. A path outside the active scope does
/// not exist, so it is not deleted either.
///
/// The names were normalised to the `/` form in `repair::fetch_deprecated`, at
/// the boundary where the server list enters the run, so this gate and the
/// `contained_join` sink below see one spelling.
///
/// `dry_run` lists instead of deleting, and adds nothing to the byte counter.
fn delete_deprecated(
    game_dir: &Path,
    list: &[String],
    summary: &Summary,
    scope: &PathScope,
    dry_run: bool,
) {
    for n in list {
        if !scope.is_visible(n) {
            tracing::debug!("deprecated path out of scope, kept: {n}");
            continue;
        }
        let p = match util::contained_join(game_dir, n) {
            Ok(p) => p,
            Err(e) => {
                // The list is server data and this is a **delete** sink: an
                // escape is skipped, never followed, and the skip is visible.
                tracing::warn!(
                    path = %n,
                    error = %format!("{e:#}"),
                    "deprecated path escapes the game dir; kept"
                );
                continue;
            }
        };
        // One stat, four answers. The old `if !p.is_file()` collapsed "not a
        // file" with every stat failure: an ACL or a sharing violation took
        // the same silent `continue` as a directory, so the deprecated file was
        // left in place with no log and no counter, while the comment below
        // claimed the stat had succeeded.
        let sz = match stat_deprecated(&p) {
            DeprecatedStat::File(len) => Some(len),
            DeprecatedStat::AlreadyGone => {
                tracing::debug!(path = %n, "deprecated file is already gone");
                continue;
            }
            DeprecatedStat::NotAFile => {
                tracing::debug!(path = %n, "deprecated path is not a file; kept");
                continue;
            }
            DeprecatedStat::Failed(e) => {
                // A stat failure is not "0 bytes" and not "not a file": the
                // delete still runs (the entry is server-named and contained),
                // but the figure is left out of the counter instead of being
                // under-reported, and the dry-run line says the size is
                // unknown rather than 0.
                tracing::warn!(
                    path = %n,
                    error = %e,
                    "deprecated file could not be stat'd; deleting without its size"
                );
                None
            }
        };
        if dry_run {
            match sz {
                Some(sz) => tracing::info!("would delete deprecated {n} ({sz} bytes)"),
                None => tracing::info!("would delete deprecated {n} (size unavailable)"),
            }
        } else {
            match std::fs::remove_file(&p) {
                Ok(()) => {
                    if let Some(sz) = sz {
                        summary.deleted_extra_bytes.fetch_add(sz, Ordering::Relaxed);
                    }
                    tracing::info!("deleted deprecated {n}");
                }
                Err(e) => {
                    // Not fatal: a locked or vanished file is not something a
                    // retry of the whole run would fix, but it must not look
                    // like a delete either.
                    tracing::warn!(
                        path = %n,
                        error = %e,
                        "deprecated file could not be deleted; left in place"
                    );
                }
            }
        }
    }
}

/// What a deprecated entry's one `metadata` call said.
///
/// Keeping the four states apart is the point: `Path::is_file()` is
/// `metadata().map(|m| m.is_file()).unwrap_or(false)`, which reads a stat
/// failure as "not a file" and loses the reason.
#[derive(Debug)]
enum DeprecatedStat {
    /// A regular file, with the size the delete can report.
    File(u64),
    /// A directory or other non-file: never a delete candidate.
    NotAFile,
    /// The path is already absent, which is the delete's desired end state.
    AlreadyGone,
    /// The path exists but could not be stat'd (ACL, sharing violation, …).
    Failed(std::io::Error),
}

/// Classify the entry's stat result. Split from [`stat_deprecated`] so the
/// error-vs-not-a-file distinction is unit-testable without an injection seam.
fn classify_deprecated(stat: std::io::Result<std::fs::Metadata>) -> DeprecatedStat {
    match stat {
        Ok(m) if m.is_file() => DeprecatedStat::File(m.len()),
        Ok(_) => DeprecatedStat::NotAFile,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => DeprecatedStat::AlreadyGone,
        Err(e) => DeprecatedStat::Failed(e),
    }
}

fn stat_deprecated(path: &Path) -> DeprecatedStat {
    classify_deprecated(std::fs::metadata(path))
}

/// Move `audio_pkg_cache_dir/**/*` into `audio_pkg_res_dir`, Starward parity
/// (`GameInstallService.cs:427-445,643-661`).
///
/// Both dirs must be configured and distinct. Leftover empty cache dirs are
/// cleaned by the files-cleanup's emptied-dir sweep, whenever a purge is
/// enabled.
///
/// A scoped run does not relocate audio: a file whose source or destination is
/// outside the scope is left alone, since moving it is a mutation of a path the
/// scope may not have admitted.
///
/// `dry_run` logs the move and does none of it.
///
/// The summary line at the end is **derived from the outcomes**, not emitted
/// because the function was reached: `moved` counts the sources that are gone
/// from the cache (renamed, or copied and then removed) and `kept` counts the
/// ones still there. An empty cache and a move that was refused both end
/// without a success claim.
fn move_audio_cache(
    game_dir: &Path,
    config: &crate::hyp::GameConfig,
    scope: &PathScope,
    dry_run: bool,
) {
    let (cache_dir, res_dir) = (&config.audio_pkg_cache_dir, &config.audio_pkg_res_dir);
    if cache_dir.is_empty() || res_dir.is_empty() || cache_dir == res_dir {
        return;
    }
    // `metadata::fetch` normalised both spellings to the `/` form the scope
    // matches on, at the boundary where `getGameConfigs` enters the run, so the
    // gate below and `contained_join` see one string.
    let cache_rel = cache_dir.as_str();
    let res_rel = res_dir.as_str();
    if !scope.is_visible(cache_rel) || !scope.is_visible(res_rel) {
        tracing::debug!("audio cache->res out of scope, skipped");
        return;
    }
    let (cache, res) = match (
        util::contained_join(game_dir, cache_dir),
        util::contained_join(game_dir, res_dir),
    ) {
        (Ok(cache), Ok(res)) => (cache, res),
        (cache, res) => {
            // Both are server data and this sink **moves files**: an escape is
            // skipped, never followed. `--print-plan`/`--dry-run` cannot reach
            // this (the post-phase is not run), but a real race would.
            tracing::warn!(
                cache = %cache_dir,
                res = %res_dir,
                cache_ok = cache.is_ok(),
                res_ok = res.is_ok(),
                "audio cache/res dir escapes the game dir; skipped"
            );
            return;
        }
    };
    if !cache.is_dir() {
        return;
    }
    let files: Vec<std::path::PathBuf> = WalkDir::new(&cache)
        .into_iter()
        .filter_map(|e| match e {
            Ok(e) => Some(e),
            Err(e) => {
                // The move is best effort: an unreadable entry is logged and
                // the rest of the cache still moves.
                tracing::warn!(
                    error = %format!("{e:#}"),
                    "audio cache walk: entry unreadable; skipped"
                );
                None
            }
        })
        .filter(|e| e.file_type().is_file())
        .map(|e| e.path().to_path_buf())
        .collect();
    if dry_run {
        for src in &files {
            let Ok(rel) = src.strip_prefix(&cache) else {
                continue;
            };
            if !scope.is_visible(&join_rel(cache_rel, rel)) {
                continue;
            }
            tracing::info!("would move audio cache {} -> res", rel.display());
        }
        return;
    }
    let mut moved = 0u64;
    let mut kept = 0u64;
    for src in files {
        let Ok(rel) = src.strip_prefix(&cache) else {
            continue;
        };
        if !scope.is_visible(&join_rel(cache_rel, rel)) {
            continue;
        }
        let target = res.join(rel);
        // `rename` first (cheap, same volume); fall back to copy+remove when
        // the file is in use or the rename is otherwise refused.
        if let Err(e) = std::fs::create_dir_all(target.parent().unwrap_or(&res)) {
            tracing::warn!(
                src = %src.display(),
                error = %e,
                "audio cache move: destination directory could not be created; source kept"
            );
            kept += 1;
            continue;
        }
        match std::fs::rename(&src, &target) {
            Ok(()) => moved += 1,
            Err(rename_err) => {
                match std::fs::copy(&src, &target) {
                    Ok(_) => {
                        // Only now is there a destination: the source is
                        // removed **after** a successful copy, never before.
                        // Removing it unconditionally would delete a cache
                        // file that had no copy if the copy failed.
                        match std::fs::remove_file(&src) {
                            Ok(()) => moved += 1,
                            Err(e) => {
                                kept += 1;
                                tracing::warn!(
                                    src = %src.display(),
                                    error = %e,
                                    "audio cache move: copied, but the source could not be removed"
                                );
                            }
                        }
                    }
                    Err(copy_err) => {
                        // Hard data-loss bug in the old shape: it removed the
                        // source even when the copy failed. Keep the source and
                        // say why.
                        kept += 1;
                        tracing::warn!(
                            src = %src.display(),
                            target = %target.display(),
                            rename_error = %rename_err,
                            error = %copy_err,
                            "audio cache move failed; source kept"
                        );
                    }
                }
            }
        }
    }
    // The one line an operator greps for: only when a relocation was
    // attempted, and with both halves of the outcome, so "moved" cannot stand
    // for a run that moved nothing.
    if moved + kept > 0 {
        tracing::info!(moved, kept, "audio cache -> res");
    }
}

/// Join two rel path fragments into the `/`-separated form the scope matches on.
fn join_rel(dir_rel: &str, name: &std::path::Path) -> String {
    let name = name.to_string_lossy().replace('\\', "/");
    if dir_rel.is_empty() {
        name
    } else {
        format!("{dir_rel}/{name}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hyp::GameConfig;
    use crate::repair::test_support::logs_of;

    fn config(cache: &str, res: &str) -> GameConfig {
        serde_json::from_value(serde_json::json!({
            "audio_pkg_cache_dir": cache,
            "audio_pkg_res_dir": res,
        }))
        .unwrap()
    }

    /// N-3: the copy fallback used to remove the source **after** a failed
    /// copy, so a locked destination or a cross-volume refusal deleted the
    /// cache file without a destination. The source is now removed only after
    /// a successful copy.
    ///
    /// A directory at the destination makes both `rename` and `copy` fail on
    /// every platform, which is the deterministic shape of that failure.
    #[test]
    fn a_failed_copy_keeps_the_audio_cache_source() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_audio_move", "copy_fail");
        let base = &*dir;
        std::fs::create_dir_all(base.join("Audio/cache")).unwrap();
        std::fs::write(base.join("Audio/cache/a.pck"), b"cache bytes").unwrap();
        std::fs::create_dir_all(base.join("Audio/res/a.pck")).unwrap();

        let logs = logs_of(|| {
            move_audio_cache(
                base,
                &config("Audio/cache", "Audio/res"),
                &PathScope::default(),
                false,
            )
        });

        assert_eq!(
            std::fs::read(base.join("Audio/cache/a.pck")).unwrap(),
            b"cache bytes",
            "the source must survive a failed copy"
        );
        assert!(
            base.join("Audio/res/a.pck").is_dir(),
            "the destination that made the move fail is untouched"
        );

        // The summary line must not claim a move: the source is still there,
        // and the counts say so.
        assert!(
            logs.contains("audio cache -> res")
                && logs.contains("moved=0")
                && logs.contains("kept=1"),
            "a refused move is reported as kept, not moved: {logs}"
        );
    }

    /// The other half of the same line: a relocation that did happen is
    /// reported with its counts.
    #[test]
    fn a_completed_audio_move_logs_the_count() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_audio_move", "moved");
        let base = &*dir;
        std::fs::create_dir_all(base.join("Audio/cache/sub")).unwrap();
        std::fs::write(base.join("Audio/cache/sub/a.pck"), b"cache bytes").unwrap();

        let logs = logs_of(|| {
            move_audio_cache(
                base,
                &config("Audio/cache", "Audio/res"),
                &PathScope::default(),
                false,
            )
        });

        assert!(
            !base.join("Audio/cache/sub/a.pck").exists(),
            "the source moved"
        );
        assert_eq!(
            std::fs::read(base.join("Audio/res/sub/a.pck")).unwrap(),
            b"cache bytes"
        );
        assert!(
            logs.contains("audio cache -> res")
                && logs.contains("moved=1")
                && logs.contains("kept=0"),
            "the summary names what happened: {logs}"
        );
    }

    /// An empty cache relocates nothing, so there is nothing to report.
    #[test]
    fn an_empty_audio_cache_logs_no_relocation() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_audio_move", "empty");
        let base = &*dir;
        std::fs::create_dir_all(base.join("Audio/cache")).unwrap();

        let logs = logs_of(|| {
            move_audio_cache(
                base,
                &config("Audio/cache", "Audio/res"),
                &PathScope::default(),
                false,
            )
        });

        assert!(
            !logs.contains("audio cache -> res"),
            "nothing moved, so no move is claimed: {logs}"
        );
    }

    /// S-3: `Path::is_file()` folded "not a file" and every stat error into
    /// `false`, so an ACL or sharing violation skipped the deprecated delete
    /// silently. The classifier keeps the error a separate state.
    #[test]
    fn a_failed_deprecated_stat_is_not_a_non_file() {
        let denied = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
        assert!(
            matches!(classify_deprecated(Err(denied)), DeprecatedStat::Failed(_)),
            "an ACL failure is a stat failure, not a non-file"
        );
        let gone = std::io::Error::new(std::io::ErrorKind::NotFound, "gone");
        assert!(matches!(
            classify_deprecated(Err(gone)),
            DeprecatedStat::AlreadyGone
        ));
    }

    /// The three real filesystem answers the delete turns on.
    #[test]
    fn stat_deprecated_separates_file_non_file_and_absent() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_deprecated", "stat");
        std::fs::write(dir.join("old.dll"), b"12345").unwrap();
        std::fs::create_dir_all(dir.join("old_dir")).unwrap();

        assert!(matches!(
            stat_deprecated(&dir.join("old.dll")),
            DeprecatedStat::File(5)
        ));
        assert!(matches!(
            stat_deprecated(&dir.join("old_dir")),
            DeprecatedStat::NotAFile
        ));
        assert!(matches!(
            stat_deprecated(&dir.join("gone.dll")),
            DeprecatedStat::AlreadyGone
        ));
    }
}
