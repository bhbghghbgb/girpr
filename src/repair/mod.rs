//! The one command, split into phases.
//!
//! [`run`] is the coordinator and holds no logic of its own beyond phase order
//! and the exit-code contract; the work lives in the submodules:
//!
//! | Phase | Module | docs/02 |
//! |---|---|---|
//! | local state + server metadata | `metadata` | steps 1–2 |
//! | begin `REPORT` | [`crate::report`] | step 2.5 |
//! | manifests + local chunk map | `manifest` | step 3 |
//! | work list + purge candidates | `plan` | steps 4, 5, 7 |
//! | `--check-only` verify | `check` | step 6.5 |
//! | per-file repair | `file` | step 6 |
//! | post-phase | `post` | step 7 |
//!
//! `audio` owns the audio scan-file format used by `metadata` and `post`.
//!
//! The run has two halves:
//!
//! **Plan** — metadata → `REPORT` → manifests → `plan::build_plan` → the
//! purge plan → the server's deprecated list. Nothing on disk has been touched
//! at the end of this half, so every decision the run will act on is already
//! made and inspectable. That is what lets the scope-of-work ceilings refuse
//! *before* any of it runs.
//!
//! **Execute** — `create_dir_all` → `--purge-before` → `--check-only` → the
//! repair loop → the post-phase. Every write and delete lives here.
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
mod file;
pub mod lock;
pub mod phases;
pub mod purge;
pub mod reuse;
pub mod scan;
pub mod scope;
#[cfg(test)]
pub(crate) mod test_support;

pub use phases::metadata::Meta;
pub use phases::plan::RepairPlan;
pub use purge::PurgePlan;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use anyhow::Context;

use crate::config::{RunCtx, RunExit, RunFailure};
use crate::hyp::HypClient;
use crate::phase::PhaseTimer;
use crate::report::{self, Summary};
use crate::sophon::SophonChunkFile;
use crate::util;

use phases::{manifest, metadata}; // temporary single import; grows in later steps

use scan::ScanReport;

/// Bring one install to the latest live version.
///
/// Returns the run counters and the process exit code as a [`RunExit`].
///
/// The work is split in two halves: `plan_phase` decides everything and
/// touches nothing, then `execute_phase` carries it out. This function is
/// only the phase order.
pub async fn run(ctx: RunCtx) -> Result<(Summary, RunExit), RunFailure> {
    // The whole run's duration is not a substitute for the phase durations: it
    // is the context that makes them add up.
    let run_timer = PhaseTimer::start("run");
    // The counters accumulate across both halves and an early plan return still
    // has to hand them back, so `run` owns the `Summary` and lends it out.
    let mut summary = Summary::default();
    let phase = match plan_phase(&ctx, &mut summary).await? {
        PlanOutcome::Finished(code) => {
            run_timer.finish();
            return Ok((summary, code));
        }
        PlanOutcome::Ready(phase) => *phase,
    };
    match execute_phase(&ctx, phase, summary).await {
        Ok(result) => {
            run_timer.finish();
            Ok(result)
        }
        // The error path leaves the timer to its drop, which reports the run as
        // incomplete rather than a completed run.
        Err(e) => Err(e),
    }
}

/// Everything the plan half decided, handed to the execute half unchanged.
///
/// Every value here was computed before `game_dir` was created, which is what
/// makes the plan half inspectable and its refusals free of side effects. The
/// execute half re-derives its own root after `create_dir_all`; the `game_dir`
/// carried here is the plan-phase form, which the second canonicalize refines.
struct PlanPhase {
    /// The install root as the plan phase resolved it (`canonicalize`, or the
    /// raw `ctx.game_dir`). The execute half re-canonicalizes after creating it.
    game_dir: PathBuf,
    /// The metadata the plan was built against; the post-phase and the audio
    /// scan-file write read it.
    meta: Meta,
    /// The HTTP client the manifests were fetched with. The repair loop reuses
    /// it rather than constructing a second one.
    http: reqwest::Client,
    /// The work list, shared with the repair loop. The post-phase purge reads it.
    plan: Arc<RepairPlan>,
    /// The up-front purge candidates, when a purge or `--print-plan` asked for
    /// them.
    purge_plan: Option<PurgePlan>,
    /// The plan-derived keep inputs, folded once here and reused by the
    /// post-repair pass. `None` when no purge flag or `--print-plan` asked for
    /// a purge plan at all.
    purge_keys: Option<Arc<purge::PurgeKeys>>,
    /// The install scan, when `--scan` asked for it. Shared with the
    /// `--check-only` verifier so neither path deep-clones it.
    scan: Option<Arc<ScanReport>>,
    /// The server's deprecated-file list (empty in read-only modes, or when the
    /// best-effort fetch failed).
    deprecated: Vec<String>,
    /// The local chunk map, shared with the repair loop and the scan.
    local_map: Arc<manifest::LocalChunkMap>,
}

/// How [`plan_phase`] ends.
///
/// The assertion and `--print-plan` returns happen before anything is mutated,
/// so they are outcomes of the plan rather than failures. The [`Summary`] is
/// owned by [`run`], which returns it with the code here.
enum PlanOutcome {
    /// The plan is complete and execute half still has work to do.
    ///
    /// Boxed so the enum stays one word; `PlanPhase` is the whole run state.
    Ready(Box<PlanPhase>),
    /// The run ends in the plan half: [`RunExit::Limit`] for a scope ceiling,
    /// [`RunExit::Success`] for `--print-plan`.
    Finished(RunExit),
}

/// The plan half: decide everything, touch nothing.
///
/// Metadata → `REPORT` → manifests → `build_plan` → the purge plan → the
/// deprecated list → the scan → the ceilings → `--print-plan` / `--plan-out`.
/// The two early exits here (`--print-plan` success, a scope ceiling) are
/// [`PlanOutcome::Finished`]; the run never reaches `create_dir_all`.
///
/// `summary` arrives empty and leaves with the plan-phase flags and totals set;
/// `--plan-out` is the one write this half performs. It is the requested
/// artifact, and it lands before the execute half so a write failure is exit `3`
/// with nothing in the install mutated.
async fn plan_phase(ctx: &RunCtx, summary: &mut Summary) -> Result<PlanOutcome, RunFailure> {
    // Read-only modes must not write anything: `--dry-run` logs actions only,
    // `--check-only` verifies only.
    let readonly = ctx.readonly();
    // NOTE: `game_dir` is NOT created here. `create_dir_all` lives in the
    // execution phase so a run that refuses before doing any work leaves no
    // directory behind. Reads tolerate a missing dir, so the plan phase can
    // proceed against it.
    let game_dir = ctx
        .game_dir
        .canonicalize()
        .unwrap_or_else(|_| ctx.game_dir.clone());
    // Resolve the install root once for the plan-phase callers. `None` means
    // the base itself could not be resolved; those callers then fall back to
    // `contained_join`, so a refusal stays per file and keeps its message.
    let root = util::PathRoot::new(&game_dir).ok();

    // NOTE: no unconditional temp sweep. All cleanup is the single
    // Collapse-style files-cleanup (`plan::purge`), run before and/or
    // after patching only when the corresponding flag is set.
    // `--purge-before` and `--purge-after` delete the same things; only the
    // timing differs.

    // 1 + 2 + 2.5. local state, server metadata, and the begin report.
    let meta = fetch_and_report(ctx, &game_dir).await?;

    // 3. manifests (latest, per-manifest prefix kept) + local chunk map.
    let http = meta.hyp.http();
    let (per_manifest, local_map) = load_manifests_and_reuse(ctx, &meta, &http, summary).await?;

    // 4. work list, purge candidates, deprecated list, scan, estimates, and the
    // ceilings — everything decided before a byte is written.
    let Some(work) = build_and_assert_plan(
        ctx,
        summary,
        &game_dir,
        root.as_ref(),
        &meta,
        per_manifest,
        &local_map,
        readonly,
    )
    .await?
    else {
        // A limit, returned rather than raised: it is "the run failed", not
        // "the run was misconfigured", and routing it through the normal return
        // gives every exit path one uniform SUMMARY tail.
        // Nothing has been written at this point — no directory created, no
        // `_tmp` opened, no file deleted, no `config.ini` touched.
        return Ok(PlanOutcome::Finished(RunExit::Limit));
    };

    // 4.7 `--print-plan` / `--plan-out`: report the whole plan and stop.
    // Read-only by construction — this returns before `create_dir_all`, so no
    // `game_dir`, no `_tmp`, no delete, no `config.ini` write. It does **not**
    // imply `--check-only`: it answers "what would you do", not "what is wrong".
    //
    // The ceilings above still apply, and still trip here: narrowing what is
    // verified is not a licence to skip the assertion.
    publish_plan(
        ctx,
        &game_dir,
        root.as_ref(),
        &work.plan,
        work.purge_plan.as_ref(),
        work.scan.as_deref(),
        &work.est_download_by_file,
        work.est_download_bytes,
    )
    .await?;
    if ctx.print_plan {
        return Ok(PlanOutcome::Finished(RunExit::Success));
    }

    Ok(PlanOutcome::Ready(Box::new(PlanPhase {
        game_dir,
        meta,
        http,
        plan: work.plan,
        purge_plan: work.purge_plan,
        purge_keys: work.purge_keys,
        scan: work.scan,
        deprecated: work.deprecated,
        local_map,
    })))
}

/// The plan half's decision, before `--print-plan` / `--plan-out` publish it.
///
/// This is an output bag for [`build_and_assert_plan`], not a new phase: the
/// caller hands it straight to [`publish_plan`] and then to [`PlanPhase`].
struct PlanWork {
    plan: Arc<RepairPlan>,
    purge_plan: Option<PurgePlan>,
    purge_keys: Option<Arc<purge::PurgeKeys>>,
    deprecated: Vec<String>,
    scan: Option<Arc<ScanReport>>,
    /// The per-file download estimate. One derivation feeds both the `PLAN`
    /// total and the `--plan-out` document.
    est_download_by_file: Vec<u64>,
    est_download_bytes: u64,
}

/// Steps 1 + 2 of the plan half: local state and server metadata, then the
/// begin report to stdout (mirrored to log), so a stdout parser sees what is on
/// disk and what it will become before any repair starts.
async fn fetch_and_report(ctx: &RunCtx, game_dir: &Path) -> Result<Meta, RunFailure> {
    let timer = PhaseTimer::start("metadata");
    let meta = metadata::fetch(ctx, game_dir).await?;
    timer.finish();
    report::emit(
        &report::report_record(&report::ReportProvenance {
            local_version: &meta.local_version,
            latest_version: &meta.latest,
            biz: ctx.biz.as_str(),
            cfg: &meta.config,
            pkg: &meta.branch,
            build: &meta.latest_build,
            audio_langs: &meta.audio,
        }),
        ctx.output,
    );
    Ok(meta)
}

/// Step 3: the latest manifests (with each manifest's chunk URL prefix kept)
/// and the local chunk-reuse map.
async fn load_manifests_and_reuse(
    ctx: &RunCtx,
    meta: &Meta,
    http: &reqwest::Client,
    summary: &mut Summary,
) -> Result<
    (
        Vec<(String, Vec<SophonChunkFile>)>,
        Arc<manifest::LocalChunkMap>,
    ),
    RunFailure,
> {
    let timer = PhaseTimer::start("manifest");
    let sleep = ctx.sleep.as_ref();
    let per_manifest = manifest::fetch_latest(
        http,
        sleep,
        &meta.latest_build,
        &meta.audio,
        &meta.ignore,
        ctx.jobs,
    )
    .await?;
    let local_map = Arc::new(
        manifest::local_reuse_map(
            http,
            sleep,
            meta.local_build.as_ref(),
            &meta.audio,
            &meta.ignore,
            ctx.jobs,
        )
        .await,
    );
    timer.finish();
    // The local manifest loading is the one degradation that changes the
    // reported numbers while leaving the exit code at `0`: with no local build
    // the reuse map is empty, so every chunk downloads and both
    // `download_bytes` and `est_download_bytes` rise. The fallback is correct
    // (`docs/02`: more downloads, never a wrong result) — this only makes the
    // degradation visible on the data plane, so a consumer can tell "reused 8,
    // fetched 2" from "fetched all 10". Emitted only when a local version was
    // known and the lookup was therefore actually attempted, so a fresh install
    // and an unflagged run's line are unaffected.
    if meta.local_version.is_some() {
        summary.flags.local_manifest = Some(if meta.local_build.is_some() {
            "ok"
        } else {
            "unavailable"
        });
    }
    Ok((per_manifest, local_map))
}

/// Step 4: the work list, the purge candidates, the deprecated list, the scan,
/// the download estimate, and the scope-of-work ceilings.
///
/// Returns `Ok(None)` when a ceiling refused the run; the caller owns the
/// [`PlanOutcome::Finished`] return so a helper never hands back an exit code.
#[allow(clippy::too_many_arguments)] // the plan inputs are all distinct, already-built values
async fn build_and_assert_plan(
    ctx: &RunCtx,
    summary: &mut Summary,
    game_dir: &Path,
    root: Option<&util::PathRoot>,
    meta: &Meta,
    per_manifest: Vec<(String, Vec<SophonChunkFile>)>,
    local_map: &Arc<manifest::LocalChunkMap>,
    readonly: bool,
) -> Result<Option<PlanWork>, RunFailure> {
    let scope = &ctx.path_scope;
    let timer = PhaseTimer::start("plan");
    let plan = Arc::new(
        phases::plan::build_plan(per_manifest, &meta.blacklist, &meta.latest, scope)
            .map_err(RunFailure::metadata)?,
    );
    tracing::info!(
        "work list: {} files{}",
        plan.len(),
        if scope.is_active() {
            " (filtered by --include/--exclude)"
        } else {
            ""
        }
    );

    summary.flags.scoped = scope.is_active();

    // 4.1 the purge candidates, planned up front and unconditionally whenever a
    // purge is going to happen — a purge flag, `--print-plan` asking for the
    // numbers, or `--purge-only` (which implies the cleanup) — for the same
    // reason: "how much would this delete?" is only answerable before the first
    // delete. `game_dir` may not exist yet; planning a missing directory yields
    // an empty plan, which is what planning the freshly-created empty directory
    // would have yielded too.
    //
    // This is the union both purge passes act on as far as can be known up
    // front. `post::run` re-checks the ceiling against its own post-repair
    // re-plan, whose candidates could in principle differ — so the bound does
    // not rest on the pipeline's current promise that a successful repair
    // promotes every `*_tmp` it creates.
    let purge_wanted = ctx.purge_before || ctx.purge_after || ctx.purge_only || ctx.print_plan;
    // The keep inputs are derived only from the immutable plan and the run's
    // configuration, so fold them once and let both purge passes (and the
    // `--print-plan` numbers) share them. The walked candidates stay
    // per-pass: the filesystem can change between the before and after walks.
    let purge_keys = if purge_wanted {
        Some(Arc::new(purge::PurgeKeys::new(
            &plan,
            purge::keep_set(),
            &meta.blacklist,
        )))
    } else {
        None
    };
    let purge_plan = if let Some(keys) = purge_keys.as_ref() {
        let purge_game_dir = game_dir.to_path_buf();
        let keys = Arc::clone(keys);
        let purge_scope = ctx.path_scope.clone();
        Some(
            tokio::task::spawn_blocking(move || {
                purge::plan_purge(&purge_game_dir, &keys, &purge_scope)
            })
            .await
            .map_err(|e| {
                RunFailure::write(anyhow::anyhow!(
                    "purge planning task failed before any delete: {e}"
                ))
            })?,
        )
    } else {
        None
    };

    let purge_candidates = purge_plan.as_ref().map_or(0, |p| p.files.len());
    timer.finish();

    // 4.2 An `--include` that selected **nothing at all** is a typo, and a typo
    // that costs 130 GB to discover is the failure mode this whole feature
    // exists to remove. Exit `1` — a property of the request — rather than `5`,
    // which means the request was well-formed and the answer was too big.
    //
    // "Nothing at all" spans both plans, which is why this sits after 4.1. An
    // empty *repair* plan on its own is not a typo: a path that is on disk but
    // absent from the new manifest — the ordinary "removed upstream" case, and
    // `gircap`'s `removed` capture — is in no repair plan by construction, and
    // `--purge-only --include <that path>` is precisely the request that deletes
    // it. (This also means a new `--include` on a `--print-plan` run reports
    // `files=0 purge_files=1` instead of erroring, which is what "print the
    // plan" should do.)
    if scope.has_includes() && plan.is_empty() && purge_candidates == 0 {
        // Say which of the two plans came back empty: the diagnosis differs, and
        // so does the fix the user needs.
        let message = if purge_plan.is_some() {
            format!(
                "--include '{}' selected nothing: plan_files=0 and the files-cleanup has no \
                 candidates either; check the glob spelling (`*` also crosses `/`) or the \
                 installed version",
                scope.describe_includes(),
            )
        } else {
            format!(
                "--include '{}' matched no file in the live manifest (plan_files=0); \
                 check the glob spelling (`*` also crosses `/`) or the installed version",
                scope.describe_includes(),
            )
        };
        return Err(RunFailure::usage(anyhow::anyhow!("{message}")));
    }

    // 4.3 the server's deprecated-file list, fetched here rather than in the
    // post-phase: it is part of what the run will act on, so it belongs with the
    // rest of the plan. Still best-effort (the list is advisory).
    //
    // `--dry-run` needs it because the post-phase now runs in a dry run and
    // lists the deprecated deletes it would make. `--check-only` and
    // `--print-plan` still skip it: neither reaches the post-phase, so the
    // request would only add a network round-trip and throw the answer away.
    // (`--print-plan` is not covered by `readonly()` — it leaves by an early
    // return — hence the explicit disjunct.)
    //
    // It stays *after* the 4.2 guard so that a typo'd `--include` still costs no
    // extra network call.
    let wants_deprecated = ctx.dry_run || (!readonly && !ctx.print_plan);
    let deprecated = if wants_deprecated {
        fetch_deprecated(&meta.hyp).await
    } else {
        tracing::debug!("read-only mode: not fetching the deprecated-file list");
        Vec::new()
    };

    // The scan, when it was asked for. It reads every planned file once and
    // turns the plan's estimates from upper bounds into figures.
    let scan = if ctx.scan {
        Some(Arc::new(
            scan::scan_plan(
                game_dir,
                root,
                Arc::clone(&plan),
                Arc::clone(local_map),
                ctx.jobs,
            )
            .await,
        ))
    } else {
        None
    };

    // === assert: the ceilings, before anything is mutated ===
    //
    // Filter → assert → work, in that order. Checking the ceilings
    // *after* the scope filter is what makes them usable at all: before
    // filtering, `--max-files 8` would refuse on every real install.
    //
    // The download estimate is `Σ compressed_size` over the chunks the local
    // reuse map cannot supply. It is exactly `gircap`'s `patch_fetch` formula,
    // and because capture invariant A6 says `len(blob) == compressed_size` it is
    // the same quantity the runner counts — but it is a **lower bound, not a
    // promise**: `try_reuse_slice` falls back to a download whenever a local
    // slice fails its MD5, so a file estimated at 0 bytes can still download its
    // whole body. That is why the runtime hard stop exists alongside this.
    //
    // Under `--scan` the estimate is the **exact** figure for the write path
    // each file takes: the chunks that are wrong where they belong and have no
    // intact source the chosen direction keeps. It resolves the mode itself,
    // including a requested `temp`, so this is one rule for every mode.
    //
    // The split is computed once, per file, and both consumers fold it: the
    // `PLAN` total below and the `--plan-out` document further down. A second
    // derivation would be a second answer.
    let est_download_by_file: Vec<u64> = match &scan {
        Some(s) => s.est_download_bytes_by_file(&plan, game_dir, root, ctx.repair_mode, local_map),
        None => plan.est_download_bytes_by_file(local_map),
    };
    let est_download_bytes = est_download_by_file
        .iter()
        .fold(0u64, |total, n| total.saturating_add(*n));
    if ctx.limits.download_bytes.is_some() {
        tracing::info!("estimated download: {est_download_bytes} bytes");
        summary.flags.plan_download_bytes = Some(est_download_bytes);
    }
    if let Some(s) = &scan {
        summary.flags.damaged_chunks = Some(s.damaged_chunks);
    }
    summary.files_total = plan.len() as u64;
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
        return Ok(None);
    }

    Ok(Some(PlanWork {
        plan,
        purge_plan,
        purge_keys,
        deprecated,
        scan,
        est_download_by_file,
        est_download_bytes,
    }))
}

/// Step 4.7: publish the plan when `--print-plan` or `--plan-out` asked for it.
///
/// Both consumers of the disk figures need the same table: the `PLAN` line and
/// the `--plan-out` document. Compute it when either was asked for, so the
/// document is written for a run that does the work as well as for
/// `--print-plan`.
#[allow(clippy::too_many_arguments)] // the published plan is one document over several borrowed parts
#[allow(clippy::expect_used)] // each `disk` use is inside the branch that computed it
async fn publish_plan(
    ctx: &RunCtx,
    game_dir: &Path,
    root: Option<&util::PathRoot>,
    plan: &Arc<RepairPlan>,
    purge_plan: Option<&PurgePlan>,
    scan: Option<&ScanReport>,
    est_download_by_file: &[u64],
    est_download_bytes: u64,
) -> Result<(), RunFailure> {
    // Publishing only costs anything when it produces a plan; an ordinary run's
    // call is a no-op and must not add a zero-duration line.
    let timer = (ctx.print_plan || ctx.plan_out.is_some()).then(|| PhaseTimer::start("publish"));
    let disk = if ctx.print_plan || ctx.plan_out.is_some() {
        let disk_game_dir = game_dir.to_path_buf();
        let disk_root = root.cloned();
        let disk_plan = Arc::clone(plan);
        let repair_mode = ctx.repair_mode;
        let jobs = ctx.jobs;
        Some(
            tokio::task::spawn_blocking(move || {
                scope::report::DiskPlan::new(
                    &disk_game_dir,
                    disk_root.as_ref(),
                    &disk_plan,
                    repair_mode,
                    jobs,
                )
            })
            .await
            .map_err(|e| {
                RunFailure::write(anyhow::anyhow!(
                    "disk planning task failed before any write: {e}"
                ))
            })?,
        )
    } else {
        None
    };
    if ctx.print_plan {
        let totals = scope::report::PlanTotals::new(
            plan,
            purge_plan,
            est_download_bytes,
            ctx.path_scope.is_active(),
            disk.as_ref().expect("the print-plan branch asks for it"),
            scan.map(|s| s.bytes),
        );
        // One `PLAN` record, rendered once, so the JSON plane carries one
        // object per event rather than a second line repeating the numbers.
        report::emit(&scope::report::plan_record(&totals), ctx.output);
    }
    // A requested artifact is never silently omitted: the flag promises the
    // plan of a run that does the work, and the same document is written for
    // `--print-plan`. It lands before the execution phase, so a write failure
    // is exit `3` with nothing mutated, the same class as the `--print-plan`
    // write.
    if let Some(out) = ctx.plan_out.as_deref() {
        let doc = scope::report::PlanDocument::new(
            plan,
            purge_plan,
            est_download_by_file,
            disk.as_ref().expect("the plan-out branch asks for it"),
        );
        doc.write_to(Path::new(out))
            .map_err(|e| RunFailure::write(e.context("write --plan-out")))?;
    }
    if let Some(timer) = timer {
        timer.finish();
    }
    Ok(())
}

/// The execute half: everything here may touch the disk.
///
/// Entered only with a completed [`PlanPhase`], so every refusal has already
/// happened and `create_dir_all` below is the first mutation. The early returns
/// keep their order and meaning: `--check-only` (success/found damage),
/// `--purge-only` (success/limit), a runtime ceiling (limit), a failed file
/// (write), then the post-phase (success/limit).
async fn execute_phase(
    ctx: &RunCtx,
    phase: PlanPhase,
    mut summary: Summary,
) -> Result<(Summary, RunExit), RunFailure> {
    let PlanPhase {
        game_dir,
        meta,
        http,
        plan,
        purge_plan,
        purge_keys,
        scan,
        deprecated,
        local_map,
    } = phase;
    let readonly = ctx.readonly();

    // === execution phase: everything below may touch the disk ===

    // 5. prepare `game_dir`, then take the single-writer lock.
    let (game_dir, root) = prepare_install(ctx, game_dir, readonly)?;
    let _lock = acquire_lock(ctx, &game_dir, readonly)?;

    // 5.1 the explicit `--audio` selection.
    write_explicit_audio(ctx, &game_dir, &meta, readonly)?;

    // 5.2 files-cleanup before.
    run_purge_before(ctx, purge_plan.as_ref(), &mut summary)?;

    // 5.5 check-only: verify only, then return. Nothing is written.
    if ctx.check_only {
        return run_check_only(ctx, &game_dir, root.as_ref(), &plan, scan, summary).await;
    }

    // 5.6 purge-only: every phase but the repair loop.
    if ctx.purge_only {
        return run_purge_only(ctx, &game_dir, purge_keys, &meta, &deprecated, summary).await;
    }

    // 6. repair files (bounded file parallelism, sequential chunks per file).
    summary = run_repair(ctx, http, &game_dir, root, &plan, local_map, scan, summary).await;
    if let Some(code) = apply_run_flags(ctx, &mut summary) {
        return Ok((summary, code));
    }
    if summary.files_failed.load(Ordering::Relaxed) > 0 {
        return Ok((summary, RunExit::Write));
    }

    // 7. post phase, when the run actually mutates the install or lists what a
    // dry run would mutate.
    let mutates = !readonly;
    let lists_dry_run = ctx.dry_run && !ctx.check_only && !ctx.print_plan;
    if mutates || lists_dry_run {
        return run_post(ctx, &game_dir, purge_keys, &meta, &deprecated, summary).await;
    }

    Ok((summary, RunExit::Success))
}

/// Step 5 of the execute half: create `game_dir` when the run may write, then
/// canonicalize it again and resolve the execute-phase install root.
///
/// `create_dir_all` is deferred from the top of the run so a refusal in the
/// plan half creates nothing.
fn prepare_install(
    ctx: &RunCtx,
    game_dir: PathBuf,
    readonly: bool,
) -> Result<(PathBuf, Option<util::PathRoot>), RunFailure> {
    if !readonly {
        std::fs::create_dir_all(&ctx.game_dir)
            .context("create game dir")
            .map_err(RunFailure::usage)?;
    }
    // The directory may have been created by the line above, so canonicalize
    // again before any path is built from it.
    let game_dir = game_dir.canonicalize().unwrap_or(game_dir);
    // The plan-phase root cached the pre-create form (or a missing tail). The
    // base exists now, so resolve it again for the execute-phase callers.
    let root = util::PathRoot::new(&game_dir).ok();
    Ok((game_dir, root))
}

/// Take the single-writer lock when this run mutates the install.
///
/// Two runs against one install can write conflicting bytes into the same
/// file, so every run that mutates the install takes the lock. The lock lives
/// outside the install, keyed by the canonical game dir, so a path scope cannot
/// hide it from one run while another run mutates the same files.
fn acquire_lock(
    ctx: &RunCtx,
    game_dir: &Path,
    readonly: bool,
) -> Result<Option<lock::Lock>, RunFailure> {
    if !lock::takes_lock(readonly, ctx.check_only, ctx.print_plan) {
        return Ok(None);
    }
    let lock = lock::Lock::acquire(game_dir)
        .map_err(|e| RunFailure::usage(e.context("single-writer lock")))?;
    tracing::info!(path = %lock.path().display(), "single-writer lock taken");
    Ok(Some(lock))
}

/// Step 5.1: write the explicit `--audio` selection now rather than in
/// `metadata`.
///
/// The plan half is documented to leave the install untouched, and this write
/// creates the scan file and its parent. Every refusal is already behind us, and
/// the repair reads the in-memory `meta.audio`, so the timing changes nothing
/// else. The path itself was validated (containment and scope) in `metadata`.
fn write_explicit_audio(
    ctx: &RunCtx,
    game_dir: &Path,
    meta: &Meta,
    readonly: bool,
) -> Result<(), RunFailure> {
    if ctx.audio_explicit && !readonly && !meta.config.audio_pkg_scan_dir.is_empty() {
        audio::write_scan_file(game_dir, &meta.config.audio_pkg_scan_dir, &meta.audio)
            .context("write audio scan file")
            .map_err(RunFailure::write)?;
    }
    Ok(())
}

/// Step 5.2: the up-front files-cleanup.
///
/// The plan built in step 4.1 is executed here to free space for the repair
/// itself. Skipped entirely in `--check-only` (verification reports damage; it
/// must not delete). In `--dry-run` it runs in log-only mode.
fn run_purge_before(
    ctx: &RunCtx,
    purge_plan: Option<&PurgePlan>,
    summary: &mut Summary,
) -> Result<(), RunFailure> {
    if ctx.purge_before
        && !ctx.check_only
        && let Some(pp) = purge_plan
    {
        let timer = PhaseTimer::start("purge-before");
        let bytes = purge::apply_purge(pp, ctx.dry_run).map_err(RunFailure::write)?;
        summary
            .deleted_extra_bytes
            .fetch_add(bytes, Ordering::Relaxed);
        timer.finish();
    }
    Ok(())
}

/// Step 5.5: verify only, then return. Nothing is written.
async fn run_check_only(
    ctx: &RunCtx,
    game_dir: &Path,
    root: Option<&util::PathRoot>,
    plan: &Arc<RepairPlan>,
    scan: Option<Arc<ScanReport>>,
    summary: Summary,
) -> Result<(Summary, RunExit), RunFailure> {
    // The execute-phase root was built after `create_dir_all` and the second
    // canonicalize, so it is the current form for these files; no per-file
    // re-resolution is needed.
    let summary = Arc::new(summary);
    let bad = phases::check::verify_plan(
        game_dir,
        root,
        Arc::clone(plan),
        Arc::clone(&summary),
        ctx.output,
        scan,
        ctx.jobs,
    )
    .await?;
    // The phase's ticker is stopped when `verify_plan` returns, but its
    // aborted task may not have dropped its clone yet; `snapshot` reads the
    // same counters either way.
    let summary = Arc::try_unwrap(summary).unwrap_or_else(|a| a.snapshot());
    let code = if bad == 0 {
        RunExit::Success
    } else {
        RunExit::CheckFailed
    };
    Ok((summary, code))
}

/// Step 5.6: `check_only` inverted — every phase but the repair loop.
///
/// It **implies** the cleanup (a flag that cleaned up nothing would be a lie),
/// and `config.ini` is **not** bumped: a cleanup does not put the bytes in
/// place, so claiming the newer version would desync the launcher the same way
/// a scoped run does, and on an already-current install it is a no-op anyway.
async fn run_purge_only(
    ctx: &RunCtx,
    game_dir: &Path,
    purge_keys: Option<Arc<purge::PurgeKeys>>,
    meta: &Meta,
    deprecated: &[String],
    mut summary: Summary,
) -> Result<(Summary, RunExit), RunFailure> {
    summary.flags.config_bumped_zero = true;
    tracing::warn!("--purge-only: cleaning up without repairing; config.ini is not bumped");
    let outcome = phases::post::run(ctx, game_dir, purge_keys, meta, &summary, deprecated).await?;
    let code = finish(outcome, &mut summary);
    Ok((summary, code))
}

/// Step 6: the repair loop.
#[allow(clippy::too_many_arguments)] // one parameter per input the loop needs
async fn run_repair(
    ctx: &RunCtx,
    http: reqwest::Client,
    game_dir: &Path,
    root: Option<util::PathRoot>,
    plan: &Arc<RepairPlan>,
    local_map: Arc<manifest::LocalChunkMap>,
    scan: Option<Arc<ScanReport>>,
    mut summary: Summary,
) -> Summary {
    summary.set_download_limit(ctx.limits.download_bytes);
    summary.set_verified_download_limit(ctx.limits.verified_download_bytes);
    summary.set_write_limit(ctx.limits.write_bytes);
    summary.set_verified_write_limit(ctx.limits.verified_write_bytes);
    summary.set_ram_limit(ctx.limits.ram_bytes);
    summary.set_fail_fast(ctx.fail_fast);
    tracing::info!(
        mode = ctx.repair_mode.as_str(),
        ram_budget = ?ctx.ram_budget(),
        "repair mode"
    );
    let timer = PhaseTimer::start("repair");
    let summary = file::repair_all(
        http,
        game_dir.to_path_buf(),
        root,
        Arc::clone(plan),
        local_map,
        scan,
        summary,
        ctx.jobs,
        ctx.dry_run,
        ctx.output,
        ctx.repair_mode,
        ctx.sleep.clone(),
    )
    .await;
    timer.finish();
    summary
}

/// Step 7: the post phase.
///
/// It runs when the run actually mutates the install, and for `--dry-run` so
/// the listing covers what the post-phase would do. `--check-only` already
/// returned above and `--print-plan` never reaches the execution phase.
async fn run_post(
    ctx: &RunCtx,
    game_dir: &Path,
    purge_keys: Option<Arc<purge::PurgeKeys>>,
    meta: &Meta,
    deprecated: &[String],
    mut summary: Summary,
) -> Result<(Summary, RunExit), RunFailure> {
    let outcome = phases::post::run(ctx, game_dir, purge_keys, meta, &summary, deprecated).await?;
    let code = finish(outcome, &mut summary);
    Ok((summary, code))
}

/// Fill in the conditional `SUMMARY` keys, then name any runtime hard stop.
///
/// Every key here is emitted **only when its flag was passed**: the
/// `is_some` / `explicit` checks are the whole rule, kept in one function so
/// it can be read against the rendering order in
/// [`crate::report::summary::conditional_extras`]. Returns
/// [`RunExit::Limit`] when a runtime ceiling bound, after recording it in
/// `stopped=`; the caller must return that code without running the post-phase.
fn apply_run_flags(ctx: &RunCtx, summary: &mut Summary) -> Option<RunExit> {
    // The byte figures, when their ceilings were asked for. The **settled**
    // one is what the repair achieved; the **spent** one is what it cost.
    // They are the same number in a clean run and differ exactly when the
    // transport misbehaved, so a reader who has only one of them cannot tell
    // a clean run from a lucky one.
    //
    // The settled write figure is reported when *either* write ceiling is
    // active: a run that asked "how much may I rewrite?" wants to see the
    // answer, whichever of the two ceilings it asked with.
    if ctx.limits.download_bytes.is_some() {
        summary.flags.download_bytes_spent = Some(summary.download.live());
    }
    if ctx.limits.write_bytes.is_some() || ctx.limits.verified_write_bytes.is_some() {
        summary.flags.write_bytes = Some(summary.write.settled());
    }
    if ctx.limits.write_bytes.is_some() {
        summary.flags.write_bytes_spent = Some(summary.write.live());
    }
    // The repair-mode group: what was asked for, what happened, and how many
    // bytes did not have to be written. Emitted only when the flag was
    // passed, so an unflagged run's `SUMMARY` stays byte-identical.
    if ctx.repair_mode_explicit {
        summary.flags.repair_mode = Some(ctx.repair_mode.as_str());
        summary.flags.files_inplace = Some(summary.files_inplace.load(Ordering::Relaxed));
        summary.flags.files_temp = Some(summary.files_temp.load(Ordering::Relaxed));
        summary.flags.kept_bytes = Some(summary.kept_bytes.load(Ordering::Relaxed));
    }
    // The peak staged payload, when a RAM budget was asked for.
    if ctx.limits.ram_bytes.is_some() {
        summary.flags.ram_bytes = Some(summary.ram_bytes.load(Ordering::Relaxed));
    }
    // A runtime hard stop is the one place the run writes *some* of what it was
    // asked to do and then stops. It leaves the work it had done in place —
    // the temp path's `_tmp` is resumed by length, the in-place path's
    // correct chunks stay correct — but it is state, so `SUMMARY` says so
    // rather than leaving it to be discovered.
    //
    // The value is the **flag** that bound, not a vocabulary word: it says
    // which ceiling it was without the reader having to know a mapping, and it
    // scales with the number of ceilings.
    let stopped = summary.stopped_by();
    let flag = stopped.flag()?;
    tracing::error!(
        "stopped: {} is a hard stop, at {} bytes (the {} figure); \
         *_tmp files left in place are resumed by the next run",
        flag,
        stopped.count(summary),
        stopped.figure()
    );
    note_stopped(summary, flag);
    Some(RunExit::Limit)
}

/// Record which ceiling bound the run, keeping the first one that did.
///
/// `SUMMARY`'s `stopped=` names the flag that bound, so a later ceiling (the
/// post-phase delete budget after a clean repair loop) must not rewrite the
/// answer the earlier one already gave.
fn note_stopped(summary: &mut Summary, flag: &'static str) {
    if summary.flags.stopped.is_none() {
        summary.flags.stopped = Some(flag);
    }
}

/// Turn a post-phase outcome into the run's exit code, and say which ceiling
/// bound.
///
/// A runtime hard stop in the cleanup is [`RunExit::Limit`] like every other
/// limit, and `stopped=--max-delete-bytes` says which one — deletion is the one
/// irreversible act in the tool, so a bounded delete that got refused has to be
/// visible.
fn finish(outcome: phases::post::PostOutcome, summary: &mut Summary) -> RunExit {
    match outcome {
        phases::post::PostOutcome::Ok | phases::post::PostOutcome::DeletedNothing => {
            RunExit::Success
        }
        phases::post::PostOutcome::DeleteCeiling => {
            tracing::error!(
                "--max-delete-bytes blocked the post-repair cleanup pass; nothing in it was deleted"
            );
            note_stopped(summary, "--max-delete-bytes");
            RunExit::Limit
        }
    }
}

/// The server's deprecated-file list, fetched with the rest of the plan.
/// Best-effort: the list is advisory, so a failed call only warns and the run
/// continues with nothing to delete.
///
/// Each name is normalised to the `/` form once, here, because the post-phase
/// scope gate and the delete sink must compare the same string. A name that is
/// not a plain relative path is dropped with a warning: it is a delete sink, so
/// it is never followed.
async fn fetch_deprecated(hyp: &HypClient) -> Vec<String> {
    match hyp.deprecated_files().await {
        Ok(list) => list
            .into_iter()
            .filter_map(|name| match util::clean_rel(&name) {
                Ok(rel) => Some(rel),
                Err(e) => {
                    tracing::warn!(
                        path = %name,
                        error = %format!("{e:#}"),
                        "deprecated path is not a plain relative path; ignored"
                    );
                    None
                }
            })
            .collect(),
        Err(e) => {
            tracing::warn!("deprecated list unavailable: {e:#}");
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{RunExit, Summary, finish, note_stopped, phases};

    /// `SUMMARY`'s `stopped=` names the **first** ceiling to bind: a later
    /// phase that also hits one must not rewrite the answer.
    #[test]
    fn the_first_ceiling_to_bind_keeps_the_stopped_flag() {
        let mut summary = Summary::new();
        note_stopped(&mut summary, "--max-write-bytes");
        note_stopped(&mut summary, "--max-delete-bytes");
        assert_eq!(summary.flags.stopped, Some("--max-write-bytes"));
    }

    /// A clean post-phase is success; only a delete ceiling is a limit.
    #[test]
    fn the_post_outcomes_keep_their_documented_codes() {
        let cases = [
            (phases::post::PostOutcome::Ok, RunExit::Success),
            (phases::post::PostOutcome::DeletedNothing, RunExit::Success),
            (phases::post::PostOutcome::DeleteCeiling, RunExit::Limit),
        ];
        for (outcome, expected) in cases {
            let mut summary = Summary::new();
            assert_eq!(finish(outcome, &mut summary), expected);
        }
    }
}
