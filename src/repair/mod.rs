//! The one command, split into phases.
//!
//! [`run`] is the coordinator and holds no logic of its own beyond phase order
//! and the exit-code contract; the work lives in the submodules:
//!
//! | Phase | Module | docs/02 |
//! |---|---|---|
//! | local state + server metadata | [`metadata`] | steps 1–2 |
//! | begin `REPORT` | [`crate::report`] | step 2.5 |
//! | manifests + local chunk map | [`manifest`] | step 3 |
//! | work list + purge candidates | [`plan`] | steps 4, 5, 7 |
//! | `--check-only` verify | [`check`] | step 6.5 |
//! | per-file repair | [`file`] | step 6 |
//! | post-phase | [`post`] | step 7 |
//!
//! [`audio`] owns the audio scan-file format used by [`metadata`] and
//! [`post`].
//!
//! Since docs/05 M0 the run has two halves:
//!
//! **Plan** — metadata → `REPORT` → manifests → [`plan::build_plan`] → the
//! server's deprecated list. Nothing on disk has been touched at the end of
//! this half, so every decision the run will act on is already made and
//! inspectable.
//!
//! **Execute** — `create_dir_all` → `--purge-before` → `--check-only` → the
//! repair loop → the post-phase. Every write and delete lives here, which is
//! what lets the scope-of-work ceilings refuse *before* any of it runs.
//!
//! Phase order overall:
//! 1. prepare `game_dir` (create when missing, unless read-only),
//! 2. metadata, then the begin report, so a parser sees the on-disk and target
//!    versions before anything is downloaded,
//! 3. manifests → work list → deprecated list (nothing written yet),
//! 4. `create_dir_all` + `--purge-before` cleanup, to free room for the repair,
//! 5. `--check-only` returns here, before any download (exit `0` clean / `4`
//!    damaged),
//! 6. the repair loop; any failed file returns exit `3` and skips the
//!    post-phase, so `config.ini` never advances over a damaged install,
//! 7. post-phase, then the caller prints `SUMMARY`.

mod audio;
mod check;
mod file;
mod manifest;
mod metadata;
mod plan;
mod post;
mod scope;

pub use metadata::Meta;
pub use plan::purge::PurgeVerdict;
pub use plan::{PlannedFile, PurgeCandidate, PurgePlan, RepairPlan};
pub use scope::PathScope;

use std::sync::Arc;
use std::sync::atomic::Ordering;

use anyhow::Context;

use crate::config::{RunCtx, RunFailure};
use crate::hyp::HypClient;
use crate::report::{self, Summary};

/// Bring one install to the latest live version.
///
/// Returns the run counters and the process exit code:
/// `0` ok (or `--check-only` clean) · `1` usage/config · `2` metadata/network ·
/// `3` write/verify · `4` `--check-only` found damage. Fatal classes come back
/// as [`RunFailure`], which carries the same codes.
pub async fn run(ctx: RunCtx) -> Result<(Summary, i32), RunFailure> {
    let mut summary = Summary::default();
    // Read-only modes must not write anything: `--dry-run` logs actions only,
    // `--check-only` verifies only.
    let readonly = ctx.readonly();
    // NOTE: `game_dir` is NOT created here. M0 moved `create_dir_all` into the
    // execution phase so a run that refuses before doing any work leaves no
    // directory behind (docs/05 M0 step 3 — the one documented behavior
    // change of that milestone). Reads tolerate a missing dir, so the plan
    // phase can proceed against it.
    let game_dir = ctx
        .game_dir
        .canonicalize()
        .unwrap_or_else(|_| ctx.game_dir.clone());

    // NOTE: no unconditional temp sweep. All cleanup is the single
    // Collapse-style files-cleanup (`plan::purge`), run before and/or
    // after patching only when the corresponding flag is set.
    // `--purge-before` and `--purge-after` delete the same things; only the
    // timing differs.

    // 1 + 2. local state and server metadata.
    let meta = metadata::fetch(&ctx, &game_dir).await?;

    // 2.5 begin report to stdout (mirrored to log): versions + API-sourced
    // metadata, so a stdout parser sees what is on disk and what it will
    // become before any repair starts.
    let report_line = report::format_report_line(
        &meta.local_version,
        &meta.latest,
        ctx.biz.as_str(),
        &meta.config,
        &meta.branch,
        &meta.latest_build,
        &meta.audio,
        ctx.json_summary,
    );
    report::emit(&report_line);

    // 3. manifests (latest, per-manifest prefix kept) + local chunk map.
    let http = meta.hyp.http();
    let per_manifest =
        manifest::fetch_latest(&http, &meta.latest_build, &meta.audio, &meta.ignore).await?;
    let local_map =
        manifest::local_reuse_map(&http, meta.local_build.as_ref(), &meta.audio, &meta.ignore)
            .await;

    // 4. work list. The plan outlives the repair loop (the post-phase purge
    // needs it), so it is shared rather than moved into the tasks.
    let plan = Arc::new(plan::build_plan(
        per_manifest,
        &meta.blacklist,
        &meta.latest,
    ));
    tracing::info!("work list: {} files", plan.files.len());

    // 4.5 the server's deprecated-file list, fetched here rather than lazily in
    // the post-phase: it is part of what the run will act on, so it belongs with
    // the rest of the plan. Still best-effort (the list is advisory).
    let deprecated = fetch_deprecated(&meta.hyp).await;

    // === execution phase: everything below may touch the disk ===

    // 5. prepare `game_dir` (create when missing, unless read-only). Deferred
    // from the top of the run on purpose — see the note above.
    if !readonly {
        std::fs::create_dir_all(&ctx.game_dir)
            .context("create game dir")
            .map_err(RunFailure::usage)?;
    }
    // The directory may have been created by the line above, so canonicalize
    // again before any path is built from it.
    let game_dir = game_dir.canonicalize().unwrap_or(game_dir);

    // 5.2 files-cleanup before: same Collapse-style cleanup as purge-after,
    // planned then executed here to free space for the repair itself. Skipped
    // entirely in `--check-only` (verification reports damage; it must not
    // delete). In `--dry-run` it runs in log-only mode.
    let mut purge_logged = false;
    if ctx.purge_before && !ctx.check_only {
        let purge_plan = plan::purge::plan_purge(&game_dir, &plan, &plan::purge::keep_set());
        let bytes =
            plan::purge::apply_purge(&purge_plan, ctx.dry_run).map_err(RunFailure::write)?;
        summary
            .deleted_extra_bytes
            .fetch_add(bytes, Ordering::Relaxed);
        purge_logged = true;
    }

    // 5.5 check-only: verify only, then return. Nothing is written.
    summary.files_total = plan.files.len() as u64;
    if ctx.check_only {
        let bad = check::verify_plan(&game_dir, &plan, &summary);
        summary.files_failed.store(bad, Ordering::Relaxed);
        return Ok((summary, if bad == 0 { 0 } else { 4 }));
    }

    // 6. repair files (bounded file parallelism, sequential chunks per file).
    summary = file::repair_all(
        http,
        game_dir.clone(),
        plan.clone(),
        local_map,
        summary,
        ctx.jobs,
        ctx.dry_run,
    )
    .await;
    if summary.files_failed.load(Ordering::Relaxed) > 0 {
        return Ok((summary, 3));
    }

    // 7. post phase. `readonly` covers both `--dry-run` and `--check-only`
    // (check-only already returned, but gating on `readonly` keeps the
    // invariant obvious).
    if !readonly {
        post::run(&ctx, &game_dir, &plan, &meta, &summary, &deprecated).await?;
    } else if ctx.dry_run && ctx.purge_after && !purge_logged {
        // Dry-run logging only: list what the files-cleanup would delete (writes
        // nothing; the byte count stays 0 by design). Skipped when the
        // purge-before pass above already listed the same candidates, so
        // `--purge-before --purge-after --dry-run` does not log each file twice.
        let purge_plan = plan::purge::plan_purge(&game_dir, &plan, &plan::purge::keep_set());
        plan::purge::apply_purge(&purge_plan, true).map_err(RunFailure::write)?;
    }

    Ok((summary, 0))
}

/// The server's deprecated-file list, fetched with the rest of the plan.
/// Best-effort: the list is advisory, so a failed call only warns and the run
/// continues with nothing to delete.
async fn fetch_deprecated(hyp: &HypClient) -> Vec<String> {
    match hyp.deprecated_files().await {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!("deprecated list unavailable: {e:#}");
            Vec::new()
        }
    }
}
