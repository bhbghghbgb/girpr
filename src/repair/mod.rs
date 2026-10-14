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
pub mod scope;

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
    let scope = &ctx.path_scope;
    let plan = Arc::new(plan::build_plan(
        per_manifest,
        &meta.blacklist,
        &meta.latest,
        scope,
    ));
    tracing::info!(
        "work list: {} files{}",
        plan.files.len(),
        if scope.is_active() {
            " (filtered by --include/--exclude)"
        } else {
            ""
        }
    );

    // 4.1 A `--include` that selected nothing is a typo, and a typo that costs
    // 130 GB to discover is the failure mode this whole feature exists to
    // remove. Exit 1 (a property of the request), not 5 (the answer was too
    // big) — see docs/04 §7.2.1.
    if scope.has_includes() && plan.files.is_empty() {
        return Err(RunFailure::usage(anyhow::anyhow!(
            "--include '{}' matched no file in the live manifest (plan_files=0); \
             check the glob spelling (`*` also crosses `/`) or the installed version",
            scope.describe_includes(),
        )));
    }
    summary.flags.scoped = scope.is_active();

    // 4.5 the server's deprecated-file list, fetched here rather than lazily in
    // the post-phase: it is part of what the run will act on, so it belongs with
    // the rest of the plan. Still best-effort (the list is advisory).
    let deprecated = fetch_deprecated(&meta.hyp).await;

    // 4.6 the purge candidates, planned up front and unconditionally whenever a
    // purge flag is set — for the same reason: "how much would this delete?" is
    // only answerable before the first delete. `game_dir` may not exist yet;
    // planning a missing directory yields an empty plan, which is what planning
    // the freshly-created empty directory would have yielded too.
    //
    // This is the union both purge passes would act on as of now. A `*_tmp` the
    // repair creates mid-run is deliberately not counted: it did not exist at
    // plan time, it is purged after the repair by a re-plan, and the write
    // ceiling is the flag that bounds it.
    let purge_plan = (ctx.purge_before || ctx.purge_after)
        .then(|| plan::purge::plan_purge(&game_dir, &plan, &plan::purge::keep_set(), scope));

    // === assert: the ceilings, before anything is mutated ===
    //
    // Filter → assert → work, in that order (docs/04 §4.1). Checking the
    // ceilings *after* the scope filter is what makes them usable at all:
    // before filtering, `--max-files 8` would refuse on every real install.
    //
    // The download estimate is `Σ compressed_size` over the chunks the local
    // reuse map cannot supply. It is exactly `gircap`'s `patch_fetch` formula,
    // and because capture invariant A6 says `len(blob) == compressed_size` it is
    // the same quantity the runner counts — but it is a **lower bound, not a
    // promise**: `try_reuse_slice` falls back to a download whenever a local
    // slice fails its MD5, so a file estimated at 0 bytes can still download its
    // whole body. That is why the runtime hard stop exists alongside this.
    let est_download_bytes = plan.est_download_bytes_with(&local_map);
    if ctx.limits.download_bytes.is_some() {
        tracing::info!("estimated download: {est_download_bytes} bytes");
        summary.flags.plan_download_bytes = Some(est_download_bytes);
    }
    summary.files_total = plan.files.len() as u64;
    summary.flags.limited = ctx.limits.is_active();
    let hits = ctx.limits.check_preflight(scope::Preflight {
        plan: &plan,
        est_download_bytes,
        purge: purge_plan.as_ref().filter(|_| !ctx.check_only),
    });
    if !hits.is_empty() {
        for hit in &hits {
            tracing::error!("limit exceeded: {hit}");
        }
        // Exit `5`, returned rather than raised: it is "the run failed", not
        // "the run was misconfigured", and routing it through the normal return
        // gives every exit path one uniform SUMMARY tail (docs/04 §7.3.1).
        // Nothing has been written at this point — no directory created, no
        // `_tmp` opened, no file deleted, no `config.ini` touched.
        return Ok((summary, 5));
    }

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

    // 5.2 files-cleanup before: the plan built in 4.6, executed here to free
    // space for the repair itself. Skipped entirely in `--check-only`
    // (verification reports damage; it must not delete). In `--dry-run` it runs
    // in log-only mode.
    let mut purge_logged = false;
    if ctx.purge_before
        && !ctx.check_only
        && let Some(pp) = purge_plan.as_ref()
    {
        let bytes = plan::purge::apply_purge(pp, ctx.dry_run).map_err(RunFailure::write)?;
        summary
            .deleted_extra_bytes
            .fetch_add(bytes, Ordering::Relaxed);
        purge_logged = true;
    }

    // 5.5 check-only: verify only, then return. Nothing is written.
    if ctx.check_only {
        let bad = check::verify_plan(&game_dir, &plan, &summary);
        summary.files_failed.store(bad, Ordering::Relaxed);
        return Ok((summary, if bad == 0 { 0 } else { 4 }));
    }

    // 6. repair files (bounded file parallelism, sequential chunks per file).
    summary.set_download_limit(ctx.limits.download_bytes);
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
    // A runtime hard stop is the one place the run writes *some* of what it was
    // asked to do and then stops. It leaves `*_tmp` behind — safe, because S3
    // resume-by-length makes the next run resume rather than restart — but it is
    // state, so `SUMMARY` says so rather than leaving it to be discovered.
    if summary.download_stopped() {
        tracing::error!(
            "stopped after {} downloaded bytes: --max-download-bytes is a hard stop; \
             *_tmp files left in place are resumed by the next run",
            summary.download_bytes.load(Ordering::Relaxed)
        );
        summary.flags.stopped = Some("bytes");
        return Ok((summary, 5));
    }
    if summary.files_failed.load(Ordering::Relaxed) > 0 {
        return Ok((summary, 3));
    }

    // 7. post phase. `readonly` covers both `--dry-run` and `--check-only`
    // (check-only already returned, but gating on `readonly` keeps the
    // invariant obvious).
    if !readonly {
        post::run(&ctx, &game_dir, &plan, &meta, &summary, &deprecated).await?;
    } else if ctx.dry_run
        && ctx.purge_after
        && !purge_logged
        && let Some(pp) = purge_plan.as_ref()
    {
        // Dry-run logging only: list what the files-cleanup would delete (writes
        // nothing; the byte count stays 0 by design). Skipped when the
        // purge-before pass above already listed the same candidates, so
        // `--purge-before --purge-after --dry-run` does not log each file twice.
        plan::purge::apply_purge(pp, true).map_err(RunFailure::write)?;
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
