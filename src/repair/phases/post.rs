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
//! The whole phase is skipped by `--check-only`/`--print-plan` and whenever
//! any file failed. `--dry-run` reaches it, but every destructive step below
//! lists instead of acting, and the `config.ini` bump is skipped.
//! The deprecated list itself is **not** fetched here — it arrives as an
//! argument, having been read during the plan phase.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use anyhow::{Context, Result};
use walkdir::WalkDir;

use crate::config::{RunCtx, RunFailure};
use crate::report::Summary;
use crate::util;
use crate::util::path::{join_rel, rel_from_base};

use super::metadata::Meta;
use crate::repair::purge;
use crate::repair::scope::PathScope;
use crate::repair::spawn_blocking_traced;

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
/// **This phase owns its own `--dry-run` discipline.** It has more than one
/// call site — the repair run, `--purge-only`, and now a dry run — and a gate
/// every caller has to remember is not a gate. Each destructive step below
/// therefore consults `ctx.dry_run` itself.
pub async fn run(
    ctx: &RunCtx,
    game_dir: &Path,
    purge_keys: Option<Arc<purge::PurgeKeys>>,
    before_purge: Option<&purge::PurgePlan>,
    meta: &Meta,
    summary: &Summary,
    deprecated: &[String],
) -> Result<PostOutcome, RunFailure> {
    let timer = crate::phase::PhaseTimer::start("post");
    match run_inner(
        ctx,
        game_dir,
        purge_keys,
        before_purge,
        meta,
        summary,
        deprecated,
    )
    .await
    {
        Ok(outcome) => {
            timer.finish();
            Ok(outcome)
        }
        // The error path leaves the timer to its drop, which reports the phase
        // as incomplete rather than completed.
        Err(e) => Err(e),
    }
}

/// The body of [`run`], wrapped so the timer only claims a completion on `Ok`.
async fn run_inner(
    ctx: &RunCtx,
    game_dir: &Path,
    purge_keys: Option<Arc<purge::PurgeKeys>>,
    before_purge: Option<&purge::PurgePlan>,
    meta: &Meta,
    summary: &Summary,
    deprecated: &[String],
) -> Result<PostOutcome, RunFailure> {
    let dry = ctx.dry_run;
    // The deprecated list can be long and every entry is a stat plus a delete:
    // that is blocking filesystem work, not a task the async worker should sit
    // through. The audio move below and the cleanup pass use the same discipline.
    let delete_game_dir = game_dir.to_path_buf();
    let delete_list = deprecated.to_vec();
    let delete_scope = ctx.path_scope.clone();
    let deleted = spawn_delete_deprecated(delete_game_dir, delete_list, delete_scope, dry).await;
    summary
        .deleted_extra_bytes
        .fetch_add(deleted, Ordering::Relaxed);
    // The audio move walks the cache and renames/copies its files. It is
    // best-effort, so a panicked task is logged and the phase carries on; it
    // must not stall the runtime while it runs.
    let move_game_dir = game_dir.to_path_buf();
    let move_config = meta.config.clone();
    let move_scope = ctx.path_scope.clone();
    if let Err(e) = tokio::task::spawn_blocking(move || {
        move_audio_cache(&move_game_dir, &move_config, &move_scope, dry);
    })
    .await
    {
        tracing::warn!(
            error = %e,
            "audio cache move task failed; the post-phase carries on"
        );
    }
    if let Some(outcome) =
        purge_after_pass(ctx, game_dir, purge_keys, before_purge, summary).await?
    {
        return Ok(outcome);
    }
    // `config.ini` is bumped last so the on-disk version only
    // advances once the bytes are in place. A dry run never put any bytes in
    // place, so it stops here: the mode lists what the phase would do and must
    // leave the launcher's version untouched.
    if dry {
        tracing::info!("dry run: not bumping config.ini");
        return Ok(PostOutcome::DeletedNothing);
    }
    // Two situations make that premise false by construction — the bytes were
    // deliberately *not* put in place:
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
    util::write_config_ini(game_dir, &meta.latest, ctx.biz.as_str(), (ch, sub, cps))
        .context("write config.ini")
        .map_err(RunFailure::write)?;
    Ok(PostOutcome::Ok)
}

/// The post-repair files-cleanup, when `--purge-after` or `--purge-only`
/// asked for it.
///
/// Returns `Some` only when the pass ends the phase: a tripped delete ceiling.
/// A completed pass returns `None` and leaves `config.ini` handling to the
/// caller.
async fn purge_after_pass(
    ctx: &RunCtx,
    game_dir: &Path,
    purge_keys: Option<Arc<purge::PurgeKeys>>,
    before_purge: Option<&purge::PurgePlan>,
    summary: &Summary,
) -> Result<Option<PostOutcome>, RunFailure> {
    if !(ctx.purge_after || ctx.purge_only) {
        return Ok(None);
    }
    let dry = ctx.dry_run;
    // The plan-derived keep inputs were folded once in the plan phase; only the
    // walked candidates are recomputed here.
    let keys = purge_keys.ok_or_else(|| {
        RunFailure::write(anyhow::anyhow!(
            "purge planning task started without the run's keyed keep sets"
        ))
    })?;
    let purge_game_dir = game_dir.to_path_buf();
    let purge_scope = ctx.path_scope.clone();
    let purge_plan =
        spawn_purge_plan(move || purge::plan_purge(&purge_game_dir, &keys, &purge_scope)).await?;
    // The post-repair re-plan is the only purge pass whose candidates were not
    // computed at assert time, so `--max-delete-bytes` is checked against it
    // again here. It can only be *smaller* than the up-front plan — a
    // successful repair promotes every `*_tmp` it creates, and a run that
    // leaves one behind has already failed (`3`) or stopped (`5`), both of
    // which skip this phase. That is a property of the pipeline, not a
    // property of the ceiling, so the check is here rather than assumed:
    // deletion is the one irreversible act in the tool. The same check refuses
    // an incomplete plan (`purge.errors > 0`) when a ceiling is set, because
    // its byte total is only a lower bound.
    if let Some(hit) = ctx.limits.check_delete(&purge_plan) {
        tracing::error!("limit exceeded: {hit}");
        // A dry run stops on the same bound as a real run: it cannot list an
        // after-pass it is not allowed to perform. The outcome keeps the
        // ceiling visible in `SUMMARY`, and the exit is the limit class in both
        // modes.
        return Ok(Some(PostOutcome::DeleteCeiling));
    }
    // In a dry run nothing was deleted by the before pass, so its candidates
    // are still on disk and the after walk would name them a second time. Only
    // the *additional* candidates belong to the after pass; a real run has
    // already removed them and is unaffected. A candidate that appears between
    // the passes is still listed.
    //
    // The operator contract this produces: each candidate is named once per
    // run, and `apply_purge`'s "purge dry-run: N bytes would be freed" line is
    // per pass - emitted once for each pass that reaches the delete step,
    // including a pass with no candidates (a `0`-byte line). With both flags
    // the before line covers what was present before the repair; when every
    // after-pass candidate was already listed, the after pass returns before
    // `apply_purge` and emits no second byte line. Summing the per-pass byte
    // lines therefore gives the run's listing total, while the candidate lines
    // are the exactly-once record.
    let mut all_already_listed = false;
    let mut purge_plan = purge_plan;
    if dry
        && ctx.purge_before
        && let Some(before) = before_purge
    {
        let listed: std::collections::HashSet<&Path> =
            before.files.iter().map(|c| c.path.as_path()).collect();
        let before_len = purge_plan.files.len();
        purge_plan
            .files
            .retain(|c| !listed.contains(c.path.as_path()));
        if purge_plan.files.len() != before_len {
            purge_plan.total_bytes = purge_plan
                .files
                .iter()
                .fold(0u64, |n, c| n.saturating_add(c.bytes));
            all_already_listed = purge_plan.files.is_empty();
        }
    }
    if all_already_listed {
        tracing::debug!(
            "purge dry-run: every after-pass candidate was already listed before the repair"
        );
        return Ok(None);
    }
    let bytes = spawn_apply_purge(purge_plan, dry).await?;
    // A dry run lists the candidates; nothing is deleted, so nothing may be
    // counted as freed space. `apply_purge` returns 0 in dry-run today, and the
    // gate keeps that invariant local to the counter as well.
    if !dry {
        summary
            .deleted_extra_bytes
            .fetch_add(bytes, Ordering::Relaxed);
    }
    Ok(None)
}

/// Run the blocking deprecated-file delete on the blocking pool, keeping the
/// step best-effort: a panicked task is logged and the phase carries on, the
/// same way the audio move does. Returns the bytes it deleted, which is `0` in
/// a dry run.
async fn spawn_delete_deprecated(
    game_dir: PathBuf,
    list: Vec<String>,
    scope: PathScope,
    dry_run: bool,
) -> u64 {
    match spawn_blocking_traced(move || delete_deprecated(&game_dir, &list, &scope, dry_run)).await
    {
        Ok(bytes) => bytes,
        Err(e) => {
            tracing::warn!(
                error = %e,
                "deprecated-file delete task failed; the post-phase carries on"
            );
            0
        }
    }
}

/// Run the blocking purge delete on the blocking pool. The plan is moved, not
/// cloned, so the worker threads only pay for the filesystem work.
async fn spawn_apply_purge(plan: purge::PurgePlan, dry_run: bool) -> Result<u64, RunFailure> {
    spawn_blocking_traced(move || purge::apply_purge(&plan, dry_run))
        .await
        .map_err(|e| {
            RunFailure::write(anyhow::anyhow!(
                "cleanup delete task failed during the post-repair cleanup: {e}"
            ))
        })?
        .map_err(RunFailure::write)
}

/// Run the blocking purge planner on its own task and give its join failure
/// the run's write class.
///
/// The split out of [`purge_after_pass`] keeps the exit-class decision testable
/// without a filesystem that can make the planner panic.
async fn spawn_purge_plan(
    planning: impl FnOnce() -> purge::PurgePlan + Send + 'static,
) -> Result<purge::PurgePlan, RunFailure> {
    spawn_blocking_traced(planning).await.map_err(|e| {
        RunFailure::write(anyhow::anyhow!(
            "purge planning task failed before the post-repair cleanup: {e}"
        ))
    })
}

/// How the post-phase ended. The post-phase mutates the disk, so a caller that
/// reached it through a path the `readonly` gate does not cover still needs the
/// outcome spelled out rather than assumed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostOutcome {
    /// Finished, everything it was asked to do is done.
    Ok,
    /// `--max-delete-bytes` blocked the post-repair cleanup pass. Nothing in
    /// that pass was deleted; earlier passes were already applied. A dry run
    /// reports the same outcome because it stops on the same bound.
    DeleteCeiling,
    /// `--dry-run`: the post-phase listed what it would do and applied
    /// nothing.
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
/// `dry_run` lists instead of deleting, and returns `0` so the caller adds
/// nothing to the byte counter. A real run returns the bytes it deleted.
fn delete_deprecated(game_dir: &Path, list: &[String], scope: &PathScope, dry_run: bool) -> u64 {
    let mut deleted_bytes = 0u64;
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
                        // A size the file system reported can never make the
                        // sum wrap; saturation only documents that a total of
                        // deleted bytes is allowed to clamp rather than panic.
                        deleted_bytes = deleted_bytes.saturating_add(sz);
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
    deleted_bytes
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

/// Copy `src` to `target` without ever publishing a partial file.
///
/// The bytes go to a temp sibling in the destination directory (same volume, so
/// the publish is a rename, not another copy), and only a complete temp file is
/// renamed over `target`. [`util::path::temp_sibling`] names that sibling, so
/// this writer and the other atomic publishes agree on the whole-name rule. On
/// any failure the temp sibling is removed, `target` is left exactly as it was,
/// and the caller keeps `src`.
///
/// The caller creates the destination directory and removes `src` only after
/// this returns `Ok`.
fn copy_publish_with(
    src: &Path,
    target: &Path,
    copy: impl Fn(&Path, &Path) -> std::io::Result<u64>,
) -> std::io::Result<()> {
    // `temp_sibling` would name a sibling even for a path that has no file
    // name, so keep the explicit refusal rather than publishing under a
    // different name.
    if target.file_name().is_none() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "audio cache destination has no file name",
        ));
    }
    let tmp = util::path::temp_sibling(target);
    if let Err(e) = copy(src, &tmp) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    if let Err(e) = std::fs::rename(&tmp, target) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
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
    move_audio_cache_with(
        game_dir,
        config,
        scope,
        dry_run,
        |from, to| std::fs::rename(from, to),
        |from, to| std::fs::copy(from, to),
    );
}

/// The body of [`move_audio_cache`], with the two filesystem operations a
/// caller can inject so the rename-then-copy fallback is exercised without
/// relying on a host-specific rename failure.
fn move_audio_cache_with(
    game_dir: &Path,
    config: &crate::hyp::GameConfig,
    scope: &PathScope,
    dry_run: bool,
    rename: impl Fn(&Path, &Path) -> std::io::Result<()>,
    copy: impl Fn(&Path, &Path) -> std::io::Result<u64>,
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
            // skipped, never followed.
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
            let Some(rel) = rel_from_base(src, &cache) else {
                continue;
            };
            if !scope.is_visible(&join_rel(cache_rel, Path::new(&rel))) {
                continue;
            }
            tracing::info!("would move audio cache {} -> res", rel);
        }
        return;
    }
    let mut moved = 0u64;
    let mut kept = 0u64;
    for src in files {
        // The scope key is the rel spelling; the filesystem operation keeps the
        // on-disk path itself, so a name that is not UTF-8 is not rewritten.
        let Some(rel) = rel_from_base(&src, &cache) else {
            continue;
        };
        let Ok(rel_path) = src.strip_prefix(&cache) else {
            continue;
        };
        if !scope.is_visible(&join_rel(cache_rel, Path::new(&rel))) {
            continue;
        }
        let target = res.join(rel_path);
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
        match rename(&src, &target) {
            Ok(()) => moved += 1,
            Err(rename_err) => {
                // The copy fallback publishes through a temp sibling, so a
                // truncated copy can never be mistaken for a complete one.
                match copy_publish_with(&src, &target, &copy) {
                    Ok(()) => {
                        // Only now is there a complete destination: the source
                        // is removed **after** it, never before. Removing it
                        // unconditionally would delete a cache file that had no
                        // copy if the copy failed.
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
                        // Keep the source when the copy fails, say why, and do
                        // not claim the move: the target was never published.
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

    /// The source is removed only after a successful copy: a locked
    /// destination or a cross-volume refusal never deletes the cache file
    /// without a destination.
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

    /// A copy that fails partway must not publish its truncated bytes: the temp
    /// sibling is removed, the destination stays absent, and the source stays.
    #[test]
    fn a_partial_copy_publishes_nothing() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_audio_move", "partial_copy");
        let base = &*dir;
        std::fs::create_dir_all(base.join("Audio/cache")).unwrap();
        std::fs::create_dir_all(base.join("Audio/res")).unwrap();
        let src = base.join("Audio/cache/a.pck");
        let target = base.join("Audio/res/a.pck");
        let tmp = base.join("Audio/res/a.pck.girpr_tmp");
        std::fs::write(&src, b"cache bytes").unwrap();

        let err = copy_publish_with(&src, &target, |_, dst| {
            std::fs::write(dst, b"partial")?;
            Err(std::io::Error::other("injected copy failure"))
        })
        .unwrap_err();

        assert_eq!(err.to_string(), "injected copy failure");
        assert!(!target.exists(), "a partial copy is never published");
        assert!(!tmp.exists(), "the temp sibling is cleaned up on failure");
        assert_eq!(
            std::fs::read(&src).unwrap(),
            b"cache bytes",
            "the source survives a failed copy"
        );
    }

    /// The same failure with a destination already in place: the original file
    /// keeps its bytes, since only a complete temp file is renamed over it.
    #[test]
    fn a_partial_copy_leaves_an_existing_target_alone() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_audio_move", "partial_keeps_target");
        let base = &*dir;
        std::fs::create_dir_all(base.join("Audio/cache")).unwrap();
        std::fs::create_dir_all(base.join("Audio/res")).unwrap();
        let src = base.join("Audio/cache/a.pck");
        let target = base.join("Audio/res/a.pck");
        let tmp = base.join("Audio/res/a.pck.girpr_tmp");
        std::fs::write(&src, b"cache bytes").unwrap();
        std::fs::write(&target, b"old bytes").unwrap();

        copy_publish_with(&src, &target, |_, dst| {
            std::fs::write(dst, b"partial")?;
            Err(std::io::Error::other("injected copy failure"))
        })
        .unwrap_err();

        assert_eq!(
            std::fs::read(&target).unwrap(),
            b"old bytes",
            "the previous target is untouched"
        );
        assert!(!tmp.exists(), "the temp sibling is cleaned up on failure");
        assert_eq!(std::fs::read(&src).unwrap(), b"cache bytes");
    }

    /// A complete copy publishes exactly the source bytes and leaves nothing
    /// behind; the caller removes the source only after that.
    #[test]
    fn a_completed_copy_publishes_the_whole_file() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_audio_move", "copy_publish");
        let base = &*dir;
        std::fs::create_dir_all(base.join("Audio/cache")).unwrap();
        std::fs::create_dir_all(base.join("Audio/res")).unwrap();
        let src = base.join("Audio/cache/a.pck");
        let target = base.join("Audio/res/a.pck");
        let tmp = base.join("Audio/res/a.pck.girpr_tmp");
        std::fs::write(&src, b"cache bytes").unwrap();

        copy_publish_with(&src, &target, |from, to| std::fs::copy(from, to)).unwrap();

        assert_eq!(
            std::fs::read(&target).unwrap(),
            b"cache bytes",
            "the published file is the complete source"
        );
        assert!(!tmp.exists(), "no temp sibling outlives the publish");
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

    /// A rename the caller refuses still publishes the file through the copy
    /// fallback: the destination gets the whole file, no temp survives, and the
    /// source is removed only after the publish. The first rename is injected
    /// because a same-volume rename does not fail portably while the copy of
    /// the same pair succeeds.
    #[test]
    fn a_refused_rename_falls_back_to_a_complete_copy() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_audio_move", "rename_fallback");
        let base = &*dir;
        std::fs::create_dir_all(base.join("Audio/cache")).unwrap();
        std::fs::create_dir_all(base.join("Audio/res")).unwrap();
        let src = base.join("Audio/cache/a.pck");
        let target = base.join("Audio/res/a.pck");
        let tmp = base.join("Audio/res/a.pck.girpr_tmp");
        std::fs::write(&src, b"cache bytes").unwrap();

        let logs = logs_of(|| {
            move_audio_cache_with(
                base,
                &config("Audio/cache", "Audio/res"),
                &PathScope::default(),
                false,
                |_, _| Err(std::io::Error::other("injected rename refusal")),
                |from, to| std::fs::copy(from, to),
            )
        });

        assert_eq!(
            std::fs::read(&target).unwrap(),
            b"cache bytes",
            "the fallback publishes the complete file"
        );
        assert!(!tmp.exists(), "the fallback leaves no temp sibling");
        assert!(
            !src.exists(),
            "the source is removed only after the copy is published"
        );
        assert!(
            logs.contains("audio cache -> res")
                && logs.contains("moved=1")
                && logs.contains("kept=0"),
            "the fallback move is reported as moved: {logs}"
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

    /// A dry run names the deprecated file it would delete, leaves it in
    /// place, and counts no bytes.
    #[test]
    fn a_dry_run_lists_the_deprecated_delete() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_deprecated", "dry_run");
        let base = &*dir;
        std::fs::create_dir_all(base.join("old")).unwrap();
        std::fs::write(base.join("old/legacy.dll"), b"12345").unwrap();
        let summary = Summary::new();

        let logs = logs_of(|| {
            let deleted = delete_deprecated(
                base,
                &["old/legacy.dll".to_string()],
                &PathScope::default(),
                true,
            );
            assert_eq!(deleted, 0, "a dry run counts no deleted bytes");
        });

        assert!(
            logs.contains("would delete deprecated old/legacy.dll (5 bytes)"),
            "the dry run names the delete and its size: {logs}"
        );
        assert_eq!(
            std::fs::read(base.join("old/legacy.dll")).unwrap(),
            b"12345",
            "a dry run deletes nothing"
        );
        assert_eq!(summary.deleted_extra_bytes.load(Ordering::Relaxed), 0);
    }

    /// A dry run names each audio cache file it would move and leaves both
    /// sides alone.
    #[test]
    fn a_dry_run_lists_the_audio_cache_move() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_audio_move", "dry_run");
        let base = &*dir;
        std::fs::create_dir_all(base.join("Audio/cache")).unwrap();
        std::fs::write(base.join("Audio/cache/track_1.pcm"), b"cache bytes").unwrap();

        let logs = logs_of(|| {
            move_audio_cache(
                base,
                &config("Audio/cache", "Audio/res"),
                &PathScope::default(),
                true,
            )
        });

        assert!(
            logs.contains("would move audio cache track_1.pcm -> res"),
            "the dry run names the move: {logs}"
        );
        assert!(
            base.join("Audio/cache/track_1.pcm").exists(),
            "the source stays in place"
        );
        assert!(
            !base.join("Audio/res/track_1.pcm").exists(),
            "nothing is published at the destination"
        );
    }

    fn purge_ctx(game_dir: std::path::PathBuf) -> RunCtx {
        RunCtx {
            game_dir,
            biz: crate::biz::Biz::Hk4eGlobal,
            audio: std::collections::HashSet::new(),
            audio_explicit: false,
            path_scope: PathScope::default(),
            limits: crate::repair::scope::Limits::default(),
            repair_mode: crate::config::RepairMode::Auto,
            repair_mode_explicit: false,
            jobs: 1,
            check_only: false,
            scan: false,
            purge_only: false,
            fail_fast: false,
            dry_run: false,
            print_plan: false,
            plan_out: None,
            purge_after: false,
            purge_before: false,
            output: crate::report::OutputFormat::Text,
            hyp_base_override: None,
            sophon_base_override: None,
            sleep: Arc::new(crate::sleep::NoSleep),
        }
    }

    fn purge_keys() -> Arc<purge::PurgeKeys> {
        let plan = crate::repair::phases::plan::RepairPlan::from_files("5.0", Vec::new());
        Arc::new(purge::PurgeKeys::new(
            &plan,
            purge::keep_set(),
            &std::collections::HashSet::new(),
        ))
    }

    /// A dry run with both purge flags names each candidate once: the before
    /// pass lists what is on disk, and the after pass adds only what changed
    /// since. Nothing is deleted, so the before candidates are still there.
    #[test]
    fn a_dry_run_after_pass_lists_only_candidates_the_before_pass_did_not() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_post", "dry_purge_once");
        let base = &*dir;
        std::fs::write(base.join("orphan_before.dat"), b"a").unwrap();
        let keys = purge_keys();
        let before = purge::plan_purge(base, &keys, &PathScope::default());

        let logs = logs_of(|| {
            purge::apply_purge(&before, true).unwrap();
            std::fs::write(base.join("orphan_after.dat"), b"bb").unwrap();
            let mut ctx = purge_ctx(base.to_path_buf());
            ctx.dry_run = true;
            ctx.purge_before = true;
            ctx.purge_after = true;
            let summary = Summary::new();
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            let outcome = rt
                .block_on(purge_after_pass(
                    &ctx,
                    base,
                    Some(Arc::clone(&keys)),
                    Some(&before),
                    &summary,
                ))
                .unwrap();
            assert_eq!(outcome, None, "the pass completed without a ceiling");
        });

        assert_eq!(
            logs.matches("would purge extra").count(),
            2,
            "each candidate is named once: {logs}"
        );
        assert_eq!(
            logs.matches("orphan_before.dat").count(),
            1,
            "the before candidate is not listed again: {logs}"
        );
        assert_eq!(
            logs.matches("orphan_after.dat").count(),
            1,
            "the candidate that appeared between passes is listed: {logs}"
        );
        assert!(base.join("orphan_before.dat").exists());
        assert!(base.join("orphan_after.dat").exists());
    }

    /// A dry run stops on the post-repair delete ceiling with the same limit
    /// class as a real run. It does not log an error and then exit clean.
    #[tokio::test]
    async fn a_dry_run_post_delete_ceiling_is_a_limit() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_post", "dry_delete_ceiling");
        let base = &*dir;
        std::fs::write(base.join("orphan.dat"), b"12345").unwrap();
        let mut ctx = purge_ctx(base.to_path_buf());
        ctx.dry_run = true;
        ctx.purge_after = true;
        ctx.limits.delete_bytes = Some(1);
        let summary = Summary::new();

        let outcome = purge_after_pass(&ctx, base, Some(purge_keys()), None, &summary)
            .await
            .unwrap();

        assert_eq!(
            outcome,
            Some(PostOutcome::DeleteCeiling),
            "a tripped ceiling is not a successful dry run"
        );
        assert!(
            base.join("orphan.dat").exists(),
            "the dry run deleted nothing"
        );
        assert_eq!(summary.deleted_extra_bytes.load(Ordering::Relaxed), 0);
    }

    /// A post-repair re-plan that cannot complete is the same write-class
    /// failure in a dry run as in a real one; the dry run did not cause it.
    #[tokio::test]
    async fn a_failed_post_purge_plan_is_a_write_failure_for_a_dry_run() {
        let failure = spawn_purge_plan(|| panic!("injected purge planning failure"))
            .await
            .expect_err("a panicked planning task is a failure");
        assert_eq!(failure.exit_code, crate::config::RunExit::Write);
        assert!(
            failure.source.to_string().contains("post-repair cleanup"),
            "the failure names the pass: {failure}"
        );
    }
}
