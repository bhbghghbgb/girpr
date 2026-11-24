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
//! **Plan** — metadata → `REPORT` → manifests (with the local reuse map and the
//! server's deprecated list fetched alongside) → `girpr_repair_plan::build_plan`
//! → the purge plan. Nothing on disk has been touched at the end of this half,
//! so every decision the run will act on is already made and inspectable. That
//! is what lets the scope-of-work ceilings refuse *before* any of it runs.
//!
//! **Execute** — `create_dir_all` → `--purge-before` → `--check-only` → the
//! repair loop → the post-phase. Every write and delete lives here.
//!
//! Phase order overall:
//! 1. prepare `game_dir` (create when missing, unless read-only),
//! 2. metadata, then the begin report, so a parser sees the on-disk and target
//!    versions before anything is downloaded,
//! 3. manifests + reuse map + deprecated list (in parallel) → work list (nothing written yet),
//! 4. `create_dir_all` + `--purge-before` cleanup, to free room for the repair,
//! 5. `--check-only` returns here, before any download (exit `0` clean / `4`
//!    damaged),
//! 6. the repair loop; any failed file returns exit `3` and skips the
//!    post-phase, so `config.ini` never advances over a damaged install,
//! 7. post-phase, then the caller prints `SUMMARY`.

// The audio scan-file boundary lives in its own acyclic leaf crate. The alias
// keeps every existing `audio::…` call site (here and in `phases::metadata`)
// pointing at the same names, while the facade depends on the crate directly
// so an edit to the repair, report or CLI modules leaves it cached and narrow
// consumers can link it without this facade.
use girpr_audio as audio;
pub(crate) mod diagnostics;
mod file;
pub(crate) mod persistent;
pub(crate) mod persistent_exec;
pub mod phases;
pub mod purge;
pub mod reuse;
pub mod scan;
pub mod scope;
#[cfg(test)]
pub(crate) mod test_support;

use girpr_repair_plan::{PurgePlan, RepairPlan};
pub use phases::metadata::Meta;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use anyhow::Context;

// The plan values and their pure decisions live in an acyclic leaf. This module
// keeps only the I/O, fetch, walk and publish policies that consume them.
use girpr_persistent_plan::ManifestInput;
use girpr_repair_plan::{
    DownloadEstimate, LocalChunkMap, PlanSummaryValues, PurgeKeys, check_scope_selection,
};

// The single-writer lock lives in its own leaf crate, so a repair-phase edit
// leaves it cached and a lock-only test target can skip the product facade.
use girpr_lock as lock;

use crate::config::{RunCtx, RunExit, RunFailure};
use crate::hyp::HypClient;
use crate::phase::PhaseTimer;
use crate::report::{self, Summary};
use crate::sophon::SophonChunkFile;
use crate::util;

use phases::manifest::ManifestBudget;
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
    /// them. Shared rather than cloned: the before pass hands the blocking
    /// delete task the same plan the post-phase later reads.
    purge_plan: Option<Arc<PurgePlan>>,
    /// The plan-derived keep inputs, folded once here and reused by the
    /// post-repair pass. `None` when no purge flag or `--print-plan` asked for
    /// a purge plan at all.
    purge_keys: Option<Arc<PurgeKeys>>,
    /// The install scan, when `--scan` asked for it. Shared with the
    /// `--check-only` verifier so neither path deep-clones it.
    scan: Option<Arc<ScanReport>>,
    /// The server's deprecated-file list (empty in read-only modes, or when the
    /// best-effort fetch failed). Shared by `Arc` so the post-phase's blocking
    /// delete task borrows the same elements instead of cloning them for
    /// `'static`.
    deprecated: Arc<[String]>,
    /// The local chunk map, shared with the repair loop and the scan.
    local_map: Arc<LocalChunkMap>,
    /// The persistent plan and its scope verdict, when the flag was passed.
    /// The post phase reads it; this commit only lists it (commits 7/8 spend
    /// bytes and publish the marker block from it).
    persistent: Option<Arc<persistent::PersistentState>>,
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
    // NOTE: `game_dir` is NOT created here. `create_dir_all` lives in the
    // execution phase so a run that refuses before doing any work leaves no
    // directory behind. Reads tolerate a missing dir, so the plan phase can
    // proceed against it.
    // Canonicalize once here and keep the answer: a successful canonicalize is
    // idempotent, so the root can take the canonical form directly instead of
    // resolving the same base a second time.
    let (game_dir, game_dir_canonical) = match ctx.game_dir.canonicalize() {
        Ok(canonical) => (canonical, true),
        Err(_) => (ctx.game_dir.clone(), false),
    };
    // Resolve the install root once for the plan-phase callers. `None` means
    // the base itself could not be resolved; those callers then fall back to
    // `contained_join`, so a refusal stays per file and keeps its message.
    let root = if game_dir_canonical {
        Some(util::PathRoot::new_canonical(&game_dir))
    } else {
        util::PathRoot::new(&game_dir).ok()
    };

    // NOTE: no unconditional temp sweep. All cleanup is the single
    // Collapse-style files-cleanup (`plan::purge`), run before and/or
    // after patching only when the corresponding flag is set.
    // `--purge-before` and `--purge-after` delete the same things; only the
    // timing differs.

    // 1 + 2 + 2.5. local state, server metadata, and the begin report.
    let meta = fetch_and_report(ctx, &game_dir).await?;

    // 3. manifests (latest, per-manifest prefix kept), local chunk map and the
    // best-effort deprecated list — three independent network phases under one
    // shared client and budget.
    let http = meta.hyp.http();
    let (per_manifest, local_map, deprecated, persistent_fetched) =
        fetch_plan_network(ctx, &meta, &http, summary).await?;

    // 4. work list, purge candidates, scan, estimates, and the ceilings —
    // everything decided before a byte is written. The deprecated list was
    // already fetched in step 3; only its use happens here.
    let Some(work) = build_and_assert_plan(PlanInputs {
        ctx,
        summary,
        game_dir: &game_dir,
        root: root.as_ref(),
        meta: &meta,
        per_manifest,
        local_map: &local_map,
        deprecated,
        persistent_fetched,
    })
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
    //
    // The document is published after the purge candidates were walked, so an
    // in-install `--plan-out` is not among them: `--purge-before` executes that
    // exact pre-flight set and leaves the document in place, while the
    // post-repair re-plan sees it as any other extra file.
    publish_plan(ctx, &work).await?;
    if ctx.print_plan {
        return Ok(PlanOutcome::Finished(RunExit::Success));
    }

    Ok(PlanOutcome::Ready(Box::new(PlanPhase {
        game_dir,
        meta,
        http,
        plan: work.plan,
        purge_plan: work.purge_plan.map(Arc::new),
        purge_keys: work.purge_keys,
        scan: work.scan,
        deprecated: work.deprecated,
        local_map,
        persistent: work.persistent,
    })))
}

/// The plan half's decision, before `--print-plan` / `--plan-out` publish it.
///
/// This is an output bag for [`build_and_assert_plan`], not a new phase: the
/// caller hands it straight to [`publish_plan`] and then to [`PlanPhase`].
struct PlanWork {
    plan: Arc<RepairPlan>,
    purge_plan: Option<PurgePlan>,
    purge_keys: Option<Arc<PurgeKeys>>,
    deprecated: Arc<[String]>,
    scan: Option<Arc<ScanReport>>,
    /// The per-file download estimate, owned by the `girpr-repair-plan` leaf.
    /// One derivation feeds both the `PLAN` total and the `--plan-out` document.
    estimate: DownloadEstimate,
    /// The disk figures, computed by a blocking task started alongside the scan
    /// so its per-file stat pass overlaps the scan's reads instead of following
    /// them. `None` when neither `--print-plan` nor `--plan-out` needs them.
    disk: Option<scope::report::DiskPlan>,
    /// The persistent tier's decided work, when `--with-persistent` was passed.
    persistent: Option<Arc<persistent::PersistentState>>,
}

/// Steps 1 + 2 of the plan half: local state and server metadata, then the
/// begin report to stdout (mirrored to log), so a stdout parser sees what is on
/// disk and what it will become before any repair starts.
async fn fetch_and_report(ctx: &RunCtx, game_dir: &Path) -> Result<Meta, RunFailure> {
    let timer = PhaseTimer::start("metadata");
    let mut meta = metadata::fetch(ctx, game_dir).await?;
    // The persistent query is part of the plan half: it touches nothing. It
    // runs for `--print-plan` and `--check-only` (both need the set to list or
    // check), but never for `--purge-only`, which puts no bytes in place (C20).
    // An unflagged run never constructs the provider at all.
    if ctx.with_persistent && !ctx.purge_only {
        let http = meta.hyp.http();
        meta.dispatch = Some(persistent::query(ctx, &meta.latest, &http).await?);
    }
    timer.finish();
    report::emit(
        &report::report_record(&report::ReportProvenance {
            local_version: &meta.local_version,
            latest_version: &meta.latest,
            biz: ctx.biz.as_str(),
            exe_file_name: meta.config.exe_file_name.as_str(),
            default_download_mode: meta.config.default_download_mode.as_str(),
            branch: meta.branch.branch.as_str(),
            package_id: meta.branch.package_id.as_str(),
            build_id: meta.latest_build.build_id.as_str(),
            diff_tags: meta.branch.diff_tags.as_slice(),
            audio_langs: &meta.audio,
        }),
        ctx.output,
    );
    Ok(meta)
}

/// Step 3: the latest manifests (with each manifest's chunk URL prefix kept),
/// the local chunk-reuse map, and the advisory deprecated-file list.
///
/// These are three independent network requests: none consumes another's
/// output, and the deprecated list is best-effort. They are started together
/// under one shared client (the manifest fetches use `http`, a clone of the
/// metadata client's pool; the deprecated fetch uses the same pool through
/// `meta.hyp`) and one shared [`ManifestBudget`], so the wall-clock is the
/// slowest phase rather than their sum and the peak decoded-manifest memory is
/// unchanged. `try_join!` keeps `fetch_latest`'s fatal error short-circuiting
/// the other two; the two best-effort phases are wrapped so their success is
/// never mistaken for a reason to fail the run.
async fn fetch_plan_network(
    ctx: &RunCtx,
    meta: &Meta,
    http: &reqwest::Client,
    summary: &mut Summary,
) -> Result<
    (
        Vec<(String, Vec<SophonChunkFile>)>,
        Arc<LocalChunkMap>,
        Arc<[String]>,
        Vec<ManifestInput>,
    ),
    RunFailure,
> {
    let timer = PhaseTimer::start("manifest");
    let backoff = &ctx.backoff;
    let budget = ManifestBudget::new();
    // The persistent branch is required, not best-effort: a failed manifest
    // fails the run with exit 2. It is absent unless the query above ran.
    let persistent_dispatch = meta.dispatch.as_ref();

    let (per_manifest, local_map, deprecated, persistent_fetched) = tokio::try_join!(
        manifest::fetch_latest(
            http,
            backoff,
            &meta.latest_build,
            &meta.audio,
            &meta.ignore,
            ctx.jobs,
            budget.clone(),
        ),
        async {
            Ok::<_, RunFailure>(
                manifest::local_reuse_map(
                    http,
                    backoff,
                    meta.local_build.as_ref(),
                    &meta.audio,
                    &meta.ignore,
                    ctx.jobs,
                    budget,
                )
                .await,
            )
        },
        async { Ok::<_, RunFailure>(load_deprecated(ctx, meta).await) },
        async {
            match persistent_dispatch {
                Some(dispatch) => persistent::fetch_manifests(http, backoff, dispatch).await,
                None => Ok(Vec::new()),
            }
        },
    )?;
    let local_map = Arc::new(local_map);
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
    Ok((per_manifest, local_map, deprecated, persistent_fetched))
}

/// The inputs to [`build_and_assert_plan`]: the values `plan_phase` already
/// resolved, grouped so the signature names one run rather than eight roles.
struct PlanInputs<'a> {
    ctx: &'a RunCtx,
    summary: &'a mut Summary,
    game_dir: &'a Path,
    root: Option<&'a util::PathRoot>,
    meta: &'a Meta,
    per_manifest: Vec<(String, Vec<SophonChunkFile>)>,
    local_map: &'a Arc<LocalChunkMap>,
    /// The advisory list fetched alongside the manifests in step 3; passed
    /// through to the plan so step 4 only decides how it is used.
    deprecated: Arc<[String]>,
    /// The three fetched-and-verified persistent manifests, when the query ran.
    persistent_fetched: Vec<ManifestInput>,
}

/// Step 4: the work list, the purge candidates, the scan, the download
/// estimate, and the scope-of-work ceilings. The deprecated list is fetched in
/// step 3 and only consumed here.
///
/// Returns `Ok(None)` when a ceiling refused the run; the caller owns the
/// [`PlanOutcome::Finished`] return so a helper never hands back an exit code.
///
/// The body is the remaining conceptual steps in order (plan + purge, the
/// include guard, the scan, the estimate + ceilings); each one keeps its own
/// side effects and log lines.
async fn build_and_assert_plan(inputs: PlanInputs<'_>) -> Result<Option<PlanWork>, RunFailure> {
    let PlanInputs {
        ctx,
        summary,
        game_dir,
        root,
        meta,
        per_manifest,
        local_map,
        deprecated,
        persistent_fetched,
    } = inputs;

    // Build the persistent plan from already-verified bytes. Its input map is
    // the unfiltered Sophon manifest: a row that collides with a base file the
    // scope hides is still a hotfix. No byte is spent and no file is written.
    let persistent = match meta.dispatch.as_ref() {
        Some(dispatch) => {
            let base_md5 = persistent::base_md5_map(&per_manifest);
            let source = persistent::read_base_hash_source(game_dir, &meta.config.exe_file_name)?;
            let state = persistent::build(
                ctx,
                game_dir,
                &meta.config.exe_file_name,
                &meta.config.audio_pkg_scan_dir,
                dispatch,
                persistent_fetched,
                &base_md5,
                source.as_deref(),
            )?;
            Some(Arc::new(state))
        }
        None => None,
    };
    // Every key with a `*` glob needs this; the persistent paths join the
    // expected set so a cleanup cannot delete a hotfix or a marker it was told
    // to keep.
    let persistent_expected: Vec<String> = persistent
        .as_ref()
        .map(|p| p.plan.expected_rels.iter().cloned().collect())
        .unwrap_or_default();

    // A scoped run whose *only* selected files are persistent ones is not an
    // `--include` typo: the base plan is empty by design but there is work to
    // do. The base rule stays untouched for unflagged runs.
    let persistent_selected = persistent
        .as_ref()
        .is_some_and(|p| !p.plan.assets.is_empty());

    // Only a purge-enabled run can have a candidate that rescues an empty
    // selection, so only that run needs the post-walk check below. An unpurged
    // run's verdict is decidable the moment the plan exists; it is checked
    // inside `plan_work_and_purge` before the full install walk.
    let purge_wanted = ctx.purge_before || ctx.purge_after || ctx.purge_only || ctx.print_plan;
    let (plan, purge_keys, purge_plan) = plan_work_and_purge(
        ctx,
        summary,
        game_dir,
        meta,
        per_manifest,
        purge_wanted,
        &persistent_expected,
        persistent_selected,
    )
    .await?;

    if purge_wanted && !persistent_selected {
        let purge_candidates = purge_plan.as_ref().map_or(0, |p| p.files.len());
        // The rule and its diagnosis live in the plan leaf; only the mapping to
        // this program's usage error stays here.
        if let Err(e) = check_scope_selection(
            &ctx.path_scope,
            &plan,
            purge_candidates,
            purge_plan.as_ref(),
        ) {
            return Err(RunFailure::usage(anyhow::anyhow!("{}", e.message())));
        }
    }

    // The disk figures stat every planned file, and under `--scan` the scan
    // stats the same files as it reads them. Start the disk pass as a blocking
    // task before the scan so the two O(n) passes overlap on the blocking pool
    // instead of running back to back. They cannot share one stat: the scan
    // follows symlinks for a modification-time guard and uses the length it
    // read, while the disk plan uses `symlink_metadata` for the symlink/dir →
    // temp decision and the on-disk length. Only their wall-clock overlaps;
    // every figure keeps the probe that gives it the right meaning.
    let disk_handle = (ctx.print_plan || ctx.plan_out.is_some()).then(|| {
        let disk_game_dir = game_dir.to_path_buf();
        let disk_root = root.cloned();
        let disk_plan = Arc::clone(&plan);
        let repair_mode = ctx.repair_mode;
        let jobs = ctx.jobs;
        // Raw `spawn_blocking`: `DiskPlan::new` emits only best-effort
        // warnings (a rel that escapes the install, a file that cannot be
        // stat'd). No test drives them through this spawn site; a test that
        // needed to would use `spawn_blocking_traced`.
        tokio::task::spawn_blocking(move || {
            scope::report::DiskPlan::new(
                &disk_game_dir,
                disk_root.as_ref(),
                &disk_plan,
                repair_mode,
                jobs,
            )
        })
    });

    let scan = scan_if_requested(ctx, game_dir, root, &plan, local_map).await;

    // The estimate is a pure computation; the summary values it derives are
    // written once, here, in the same order and under the same conditions as
    // before.
    let (estimate, plan_summary) =
        estimate_download(ctx, scan.as_deref(), &plan, game_dir, root, local_map);
    if let Some(bytes) = plan_summary.reported_bytes {
        tracing::info!("estimated download: {bytes} bytes");
        summary.flags.plan_download_bytes = Some(bytes);
    }
    if let Some(damaged_chunks) = plan_summary.damaged_chunks {
        summary.flags.damaged_chunks = Some(damaged_chunks);
    }
    summary.files_total = plan_summary.files_total;
    summary.flags.limited = plan_summary.limited;
    if ctx.with_persistent {
        let (files, status, omitted) = match persistent.as_deref() {
            Some(p) => (p.plan.assets.len() as u64, p.status, p.base_hash_omitted),
            None => (0, "skipped", false),
        };
        summary.flags.persistent = Some(status);
        summary.flags.persistent_files = Some(files);
        summary.flags.persistent_repaired = Some(0);
        summary.flags.persistent_failed = Some(0);
        if omitted {
            summary.flags.persistent_base_hash = Some("omitted");
        }
    }
    let (persistent_files, persistent_bytes) = persistent
        .as_deref()
        .map_or((0, 0), |p| (p.plan.assets.len() as u64, p.plan.total_bytes));
    if !within_limits(
        ctx,
        &plan,
        purge_plan.as_ref(),
        estimate.bytes,
        persistent_files,
        persistent_bytes,
    ) {
        // A refusal never publishes a plan; do not leave the speculative disk
        // pass running on the pool after we return.
        if let Some(handle) = &disk_handle {
            handle.abort();
        }
        return Ok(None);
    }

    // The scan (if any) has finished; the disk task has been running alongside
    // it. Await it here, still before anything is written, so its figures are
    // ready to publish without a second stat pass.
    let disk = match disk_handle {
        Some(handle) => Some(handle.await.map_err(|e| {
            RunFailure::write(anyhow::anyhow!(
                "disk planning task failed before any write: {e}"
            ))
        })?),
        None => None,
    };

    Ok(Some(PlanWork {
        plan,
        purge_plan,
        purge_keys,
        deprecated,
        scan,
        estimate,
        disk,
        persistent,
    }))
}

/// Step 4.0: the work list and, when a purge will happen, its candidates.
///
/// The `plan` timer covers both, and stops before the include guard and the
/// network work that follow. An unpurged run applies the include guard here,
/// right after the plan is built: with no walk there can be no purge candidate
/// to rescue an empty selection, so a typo must not pay for that walk.
async fn plan_work_and_purge(
    ctx: &RunCtx,
    summary: &mut Summary,
    game_dir: &Path,
    meta: &Meta,
    per_manifest: Vec<(String, Vec<SophonChunkFile>)>,
    purge_wanted: bool,
    persistent_expected: &[String],
    persistent_selected: bool,
) -> Result<(Arc<RepairPlan>, Option<Arc<PurgeKeys>>, Option<PurgePlan>), RunFailure> {
    let scope = &ctx.path_scope;
    let timer = PhaseTimer::start("plan");
    let plan = Arc::new(
        girpr_repair_plan::build_plan(per_manifest, &meta.blacklist, &meta.latest, scope)
            .map_err(RunFailure::metadata)?,
    );

    // No purge flag means no walk will run, so `purge_candidates` is necessarily
    // 0 and the scope-typo verdict is already final. Refuse now rather than
    // after a full install walk. Passing `None` for the purge plan keeps the
    // exact "matched no file in the live manifest" wording an unpurged run has
    // always produced; the alternative wording belongs to purge-enabled runs,
    // whose post-walk check stays in `build_and_assert_plan`.
    if !purge_wanted
        && !persistent_selected
        && let Err(e) = check_scope_selection(scope, &plan, 0, None)
    {
        return Err(RunFailure::usage(anyhow::anyhow!("{}", e.message())));
    }

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
    // The keep inputs are derived only from the immutable plan and the run's
    // configuration, so fold them once and let both purge passes (and the
    // `--print-plan` numbers) share them. The walked candidates stay
    // per-pass: the filesystem can change between the before and after walks.
    //
    // The fold keys every expected rel with `path_key`, which is proportional
    // to the manifest size (hundreds of milliseconds on a large install). It
    // runs in the same blocking task as the walk, so neither the fold nor the
    // walk occupies an async worker.
    let (purge_keys, purge_plan) = if purge_wanted {
        let purge_game_dir = game_dir.to_path_buf();
        let keys_plan = Arc::clone(&plan);
        let blacklist = meta.blacklist.clone();
        let purge_scope = ctx.path_scope.clone();
        let persistent_expected = persistent_expected.to_vec();
        // Raw `spawn_blocking`: the walk emits only best-effort diagnostics
        // (a path it could not read, a candidate it could not stat), which
        // nothing asserts through a scoped subscriber. A test that needed them
        // would use `spawn_blocking_traced`.
        let (keys, plan) = tokio::task::spawn_blocking(move || {
            let mut keys = PurgeKeys::new(&keys_plan, purge::keep_set(), &blacklist);
            keys.extend_expected(persistent_expected.iter().map(String::as_str));
            let plan = purge::plan_purge(&purge_game_dir, &keys, &purge_scope);
            (keys, plan)
        })
        .await
        .map_err(|e| {
            RunFailure::write(anyhow::anyhow!(
                "purge planning task failed before any delete: {e}"
            ))
        })?;
        // Surface the walk's own counters on the caller thread from the
        // finished plan, never from a scoped worker (those carry a thread-local
        // dispatcher). They are independent of any ceiling: a lower-bound
        // delete total is worth a machine-readable warning even when no
        // `--max-delete-bytes` was asked for. Emitted only when non-zero, so a
        // clean purge leaves `SUMMARY` byte-identical.
        if plan.errors > 0 {
            summary.flags.purge_errors = Some(plan.errors);
        }
        if plan.hidden > 0 {
            summary.flags.purge_hidden = Some(plan.hidden);
        }
        (Some(Arc::new(keys)), Some(plan))
    } else {
        (None, None)
    };

    timer.finish();
    Ok((plan, purge_keys, purge_plan))
}

// Step 4.2 is now `girpr_repair_plan::check_scope_selection`: the rule and
// its typed diagnosis live with the two plans they decide over. Exit `1` — a
// property of the request — rather than `5`, and the caller owns only the
// mapping to this binary's usage error. Unpurged runs apply it earlier, in
// `plan_work_and_purge`, where the verdict is already final.

/// The server's deprecated-file list, fetched in step 3 alongside the manifests.
///
/// Fetched in the plan rather than the post-phase: it is part of what the run
/// will act on. Still best-effort (the list is advisory) and independent of the
/// manifests and the reuse map, so it overlaps with both.
///
/// `--dry-run` needs it because the post-phase now runs in a dry run and lists
/// the deprecated deletes it would make. `--check-only` and `--print-plan` still
/// skip it: neither reaches the post-phase, so the request would only add a
/// network round-trip and throw the answer away.
///
/// Those two modes are the whole exclusion. `readonly()` is
/// `dry_run || check_only`, so once `check_only` is excluded the remaining
/// `dry_run || !readonly` reduces to `dry_run || !dry_run` — always true — and
/// the gate is just `!print_plan && !check_only`.
///
/// Overlapping the fetch means a run refused by the step 4.2 scope guard may
/// have made this one advisory request before the refusal. The request is
/// read-only and best-effort, and the guard's outcome and message are unchanged.
async fn load_deprecated(ctx: &RunCtx, meta: &Meta) -> Arc<[String]> {
    let wants_deprecated = !ctx.print_plan && !ctx.check_only;
    if wants_deprecated {
        // The fetch hands back the shared list itself, so the post-phase can
        // share it with its blocking task for a reference bump.
        fetch_deprecated(&meta.hyp).await
    } else {
        tracing::debug!("read-only mode: not fetching the deprecated-file list");
        Vec::new().into()
    }
}

/// The scan, when it was asked for. It reads every planned file once and turns
/// the plan's estimates from upper bounds into figures.
async fn scan_if_requested(
    ctx: &RunCtx,
    game_dir: &Path,
    root: Option<&util::PathRoot>,
    plan: &Arc<RepairPlan>,
    local_map: &Arc<LocalChunkMap>,
) -> Option<Arc<ScanReport>> {
    if !ctx.scan {
        return None;
    }
    Some(Arc::new(
        scan::scan_plan(
            game_dir,
            root,
            Arc::clone(plan),
            Arc::clone(local_map),
            ctx.jobs,
        )
        .await,
    ))
}

/// The download estimate and the `SUMMARY` values it derives now live in the
/// `girpr-repair-plan` leaf as [`DownloadEstimate`] and [`PlanSummaryValues`].
///
/// The estimate is exactly `gircap`'s `patch_fetch` formula, but a **lower
/// bound, not a promise**: a local slice that fails its MD5 falls back to a
/// download, so the runtime hard stop still enforces the ceiling. Under
/// `--scan` it is the exact figure for each file's write path, with the mode
/// resolved by the repair-policy leaf. The split is computed once and both
/// consumers fold it: the `PLAN` total and the `--plan-out` document.
fn estimate_download(
    ctx: &RunCtx,
    scan: Option<&ScanReport>,
    plan: &Arc<RepairPlan>,
    game_dir: &Path,
    root: Option<&util::PathRoot>,
    local_map: &Arc<LocalChunkMap>,
) -> (DownloadEstimate, PlanSummaryValues) {
    let by_file = match scan {
        Some(s) => s.est_download_bytes_by_file(plan, game_dir, root, ctx.repair_mode, local_map),
        None => plan.est_download_bytes_by_file(local_map),
    };
    let bytes = by_file
        .iter()
        .fold(0u64, |total, n| total.saturating_add(*n));
    (
        DownloadEstimate { by_file, bytes },
        PlanSummaryValues {
            reported_bytes: ctx.limits.download_bytes.map(|_| bytes),
            damaged_chunks: scan.map(|s| s.damaged_chunks),
            files_total: plan.len() as u64,
            limited: ctx.limits.is_active(),
        },
    )
}

/// === assert: the ceilings, before anything is mutated ===
///
/// Filter → assert → work, in that order. Checking the ceilings *after* the
/// scope filter is what makes them usable at all: before filtering,
/// `--max-files 8` would refuse on every real install.
///
/// Returns `false` when a ceiling refused the run, after logging every hit.
fn within_limits(
    ctx: &RunCtx,
    plan: &Arc<RepairPlan>,
    purge_plan: Option<&PurgePlan>,
    est_download_bytes: u64,
    persistent_files: u64,
    persistent_bytes: u64,
) -> bool {
    let hits = ctx.limits.check_preflight(scope::Preflight {
        plan,
        est_download_bytes,
        purge: purge_plan.filter(|_| !ctx.check_only),
        persistent_files,
        persistent_bytes,
    });
    if hits.is_empty() {
        return true;
    }
    for hit in &hits {
        tracing::error!("limit exceeded: {hit}");
    }
    false
}

/// Step 4.7: publish the plan when `--print-plan` or `--plan-out` asked for it.
///
/// Both consumers of the disk figures need the same table: the `PLAN` line and
/// the `--plan-out` document. Compute it when either was asked for, so the
/// document is written for a run that does the work as well as for
/// `--print-plan`.
#[allow(clippy::expect_used)] // each `disk` use is under a flag that made `build_and_assert_plan` compute it
async fn publish_plan(ctx: &RunCtx, work: &PlanWork) -> Result<(), RunFailure> {
    let plan = &work.plan;
    let purge_plan = work.purge_plan.as_ref();
    let scan = work.scan.as_deref();
    let est_download_by_file = &work.estimate.by_file;
    let est_download_bytes = work.estimate.bytes;
    // The figures were computed by a blocking task started alongside the scan
    // (see `build_and_assert_plan`); publishing itself is now just the two
    // consumers folding the same table. No per-file stat happens here.
    let disk = work.disk.as_ref();
    // Publishing only costs anything when it produces a plan; an ordinary run's
    // call is a no-op and must not add a zero-duration line.
    let timer = (ctx.print_plan || ctx.plan_out.is_some()).then(|| PhaseTimer::start("publish"));
    let persistent_totals =
        work.persistent
            .as_deref()
            .map(|p| scope::report::PersistentPlanTotals {
                files: p.plan.assets.len() as u64,
                bytes: p.plan.total_bytes,
                scan_excludes_persistent: ctx.scan,
                status: p.status,
                assets: p.plan.assets.iter().map(|a| a.rel.clone()).collect(),
                markers: p.plan.marker_paths.clone(),
                mirrors: p.plan.mirrors.iter().map(|(rel, _)| rel.clone()).collect(),
            });
    if ctx.print_plan {
        let totals = scope::report::PlanTotals::new(
            plan,
            purge_plan,
            est_download_bytes,
            ctx.path_scope.is_active(),
            disk.expect("the print-plan branch asks for it"),
            scan.map(|s| s.bytes),
            persistent_totals.clone(),
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
            disk.expect("the plan-out branch asks for it"),
        );
        let doc = match persistent_totals {
            Some(p) => doc.with_persistent(scope::report::PersistentDocument {
                files: p.files,
                bytes: p.bytes,
                scan_excludes_persistent: p.scan_excludes_persistent,
                scope: p.status,
                assets: p.assets,
                markers: p.markers,
                mirrors: p.mirrors,
            }),
            None => doc,
        };
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
        persistent,
    } = phase;
    let readonly = ctx.readonly();

    // === execution phase: everything below may touch the disk ===

    // 5. prepare `game_dir`, then take the single-writer lock.
    let (game_dir, root, game_dir_canonical) = prepare_install(ctx, game_dir, readonly)?;
    let _lock = acquire_lock(ctx, &game_dir, game_dir_canonical, readonly)?;

    // 5.1 the explicit `--audio` selection.
    write_explicit_audio(ctx, &game_dir, &meta, readonly)?;

    // 5.2 files-cleanup before.
    run_purge_before(ctx, purge_plan.clone(), &mut summary).await?;

    // 5.5 check-only: verify only, then return. Nothing is written.
    if ctx.check_only {
        if ctx.with_persistent {
            summary.flags.persistent = Some("listed");
        }
        return run_check_only(
            ctx,
            &game_dir,
            root.as_ref(),
            &plan,
            scan,
            persistent.as_deref(),
            summary,
        )
        .await;
    }

    // 5.6 purge-only: every phase but the repair loop.
    if ctx.purge_only {
        return run_purge_only(
            ctx,
            &game_dir,
            purge_keys,
            purge_plan.as_deref(),
            &meta,
            &deprecated,
            persistent.as_deref(),
            summary,
        )
        .await;
    }

    // 6a. persistent full-file repair. The master flag gated this state in the
    // plan half; `--purge-only` returned above and a read-only run only lists
    // the work (the helper handles that branch itself). Prerequisites and hotfix
    // bytes land here, before the base repair and the post phase.
    if let Some(state) = persistent.as_deref() {
        install_run_limits(ctx, &mut summary);
        summary = persistent_exec::repair(
            ctx,
            &http,
            &game_dir,
            &meta.config.exe_file_name,
            state,
            summary,
        )
        .await;
    }

    // 6. repair files (bounded file parallelism, sequential chunks per file).
    summary = run_repair(
        ctx,
        file::RepairInputs {
            http,
            game_dir: game_dir.to_path_buf(),
            root,
            plan: Arc::clone(&plan),
            local_map,
            scan,
            summary,
            jobs: ctx.jobs,
            dry_run: ctx.dry_run,
            output: ctx.output,
            repair_mode: ctx.repair_mode,
            backoff: ctx.backoff.clone(),
        },
    )
    .await;
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
        return run_post(
            ctx,
            &game_dir,
            purge_keys,
            purge_plan.as_deref(),
            &meta,
            &deprecated,
            persistent.as_deref(),
            summary,
        )
        .await;
    }

    Ok((summary, RunExit::Success))
}

/// Step 5 of the execute half: create `game_dir` when the run may write, then
/// canonicalize it again and resolve the execute-phase install root.
///
/// `create_dir_all` is deferred from the top of the run so a refusal in the
/// plan half creates nothing.
/// The second element reports whether `game_dir` is canonical, so the lock can
/// reuse that form instead of resolving it a third time. A `false` keeps the
/// full canonicalizing lock path: a non-canonical spelling must still map all
/// runs of one install onto the same lock file.
fn prepare_install(
    ctx: &RunCtx,
    game_dir: PathBuf,
    readonly: bool,
) -> Result<(PathBuf, Option<util::PathRoot>, bool), RunFailure> {
    if !readonly {
        // A failed mkdir (permission, full disk, a file in the path) is a
        // write-path failure, not a bad request: name the path and let the OS
        // error carry the reason.
        std::fs::create_dir_all(&ctx.game_dir)
            .with_context(|| format!("create game dir {}", ctx.game_dir.display()))
            .map_err(RunFailure::write)?;
    }
    // The directory may have been created by the line above, so canonicalize
    // again before any path is built from it. This is the phase's one
    // canonicalize; the root and the lock both reuse the result.
    let (game_dir, game_dir_canonical) = match game_dir.canonicalize() {
        Ok(canonical) => (canonical, true),
        Err(_) => (game_dir, false),
    };
    let root = if game_dir_canonical {
        Some(util::PathRoot::new_canonical(&game_dir))
    } else {
        util::PathRoot::new(&game_dir).ok()
    };
    Ok((game_dir, root, game_dir_canonical))
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
    game_dir_canonical: bool,
    readonly: bool,
) -> Result<Option<lock::Lock>, RunFailure> {
    if !lock::takes_lock(readonly, ctx.check_only, ctx.print_plan) {
        return Ok(None);
    }
    // The execute phase already canonicalized this path. Reuse that form so the
    // lock does not touch the filesystem again; a path that could not be
    // canonicalized still goes through the full form to keep the lock keyed by
    // a single spelling of the install.
    // A live lock is a write-path refusal, not a bad request. The error from
    // `held()` already names the holder; classifying it with the write
    // failures stops automation reading a busy install as a bad invocation.
    let lock = if game_dir_canonical {
        lock::Lock::acquire_canonical(game_dir)
    } else {
        lock::Lock::acquire(game_dir)
    }
    .map_err(|e| RunFailure::write(e.context("single-writer lock")))?;
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

/// Run a blocking closure on the blocking pool with the caller's tracing
/// dispatcher installed.
///
/// `spawn_blocking` starts on a pool thread with no thread-local subscriber.
/// Production installs a global one, so its lines are unaffected; this keeps a
/// caller that scoped a subscriber (a test, or any explicit `with_default`)
/// seeing the step's log lines, which are part of the operator contract.
///
/// The dispatcher is read from the calling thread at call time, so the call
/// must be made on the thread carrying the subscriber — not from inside a
/// spawned task, which a runtime may run on a worker without it. Called from
/// such a task, this captures whatever dispatcher that worker carries instead
/// of the caller's (for a scoped subscriber, none), so the closure's lines
/// silently go there rather than to the scoped subscriber: the work still runs,
/// the log record does not reach the caller.
///
/// The task is spawned before the handle is returned, so a caller can start a
/// read alongside other work and await it later. Dropping the handle detaches
/// the task rather than cancelling it: it still runs to completion on the
/// blocking pool.
fn spawn_blocking_traced<F, R>(f: F) -> tokio::task::JoinHandle<R>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    let dispatcher = tracing::dispatcher::get_default(|d| d.clone());
    tokio::task::spawn_blocking(move || tracing::dispatcher::with_default(&dispatcher, f))
}

/// Step 5.2: the up-front files-cleanup.
///
/// The plan built in step 4.1 is executed here to free space for the repair
/// itself. Skipped entirely in `--check-only` (verification reports damage; it
/// must not delete). In `--dry-run` it runs in log-only mode.
///
/// The delete is a sequence of blocking syscalls over the whole candidate
/// list, so it runs on the blocking pool; the async worker only waits for the
/// byte total. The plan is shared by `Arc`, not cloned, so this costs one
/// reference bump.
async fn run_purge_before(
    ctx: &RunCtx,
    purge_plan: Option<Arc<PurgePlan>>,
    summary: &mut Summary,
) -> Result<(), RunFailure> {
    if ctx.purge_before
        && !ctx.check_only
        && let Some(pp) = purge_plan
    {
        let timer = PhaseTimer::start("purge-before");
        let dry = ctx.dry_run;
        let bytes = spawn_blocking_traced(move || purge::apply_purge(&pp, dry))
            .await
            .map_err(|e| {
                RunFailure::write(anyhow::anyhow!(
                    "purge delete task failed before the repair: {e}"
                ))
            })?
            .map_err(RunFailure::write)?;
        summary.add_deleted_bytes(bytes);
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
    persistent: Option<&persistent::PersistentState>,
    summary: Summary,
) -> Result<(Summary, RunExit), RunFailure> {
    // The execute-phase root was built after `create_dir_all` and the second
    // canonicalize, so it is the current form for these files; no per-file
    // re-resolution is needed.
    let summary = Arc::new(summary);
    let mut bad = phases::check::verify_plan(
        game_dir,
        root,
        Arc::clone(plan),
        Arc::clone(&summary),
        ctx.output,
        scan,
        ctx.jobs,
    )
    .await?;
    // C5: with the flag, the persistent tier is part of "intact". Without it
    // `persistent` is `None`, so this is a no-op and the unflagged check stays
    // byte-identical (plan, counters, exit code).
    if let Some(state) = persistent {
        bad += phases::check::verify_persistent(game_dir, state, &summary);
    }
    // Success path only. The ticker guard is dropped when `verify_plan`
    // returns, but its aborted task may still hold the `Arc` clone until the
    // runtime polls the drop; `try_unwrap` then fails and `snapshot` reads the
    // same counters. The panic path above returned with `?` before this line,
    // so it takes no snapshot at all.
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
    purge_keys: Option<Arc<PurgeKeys>>,
    before_purge: Option<&PurgePlan>,
    meta: &Meta,
    deprecated: &Arc<[String]>,
    persistent: Option<&persistent::PersistentState>,
    mut summary: Summary,
) -> Result<(Summary, RunExit), RunFailure> {
    summary.flags.config_bumped_zero = true;
    if ctx.with_persistent {
        summary.flags.persistent = Some("skipped");
    }
    tracing::warn!("--purge-only: cleaning up without repairing; config.ini is not bumped");
    let outcome = phases::post::run(
        ctx,
        game_dir,
        purge_keys,
        before_purge,
        meta,
        &summary,
        deprecated,
        persistent,
    )
    .await?;
    let code = finish(outcome, &mut summary);
    Ok((summary, code))
}

/// Install the run's live ceilings on the shared `Summary`.
///
/// Called once before the persistent execute path and again at the entry of the
/// base repair loop. Setting the same values twice is harmless; doing it before
/// the persistent path is what makes a hotfix byte subject to the very same
/// ceiling and hard-stop marker as a base chunk.
fn install_run_limits(ctx: &RunCtx, summary: &mut Summary) {
    summary.set_download_limit(ctx.limits.download_bytes);
    summary.set_verified_download_limit(ctx.limits.verified_download_bytes);
    summary.set_write_limit(ctx.limits.write_bytes);
    summary.set_verified_write_limit(ctx.limits.verified_write_bytes);
    summary.set_ram_limit(ctx.limits.ram_bytes);
    summary.set_fail_fast(ctx.fail_fast);
}

/// Step 6: the repair loop.
async fn run_repair(ctx: &RunCtx, mut inputs: file::RepairInputs) -> Summary {
    install_run_limits(ctx, &mut inputs.summary);
    tracing::info!(
        mode = ctx.repair_mode.as_str(),
        ram_budget = ?ctx.ram_budget(),
        "repair mode"
    );
    let timer = PhaseTimer::start("repair");
    let summary = file::repair_all(inputs).await;
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
    purge_keys: Option<Arc<PurgeKeys>>,
    before_purge: Option<&PurgePlan>,
    meta: &Meta,
    deprecated: &Arc<[String]>,
    persistent: Option<&persistent::PersistentState>,
    mut summary: Summary,
) -> Result<(Summary, RunExit), RunFailure> {
    if ctx.with_persistent && ctx.dry_run {
        summary.flags.persistent = Some("listed");
    }
    let outcome = phases::post::run(
        ctx,
        game_dir,
        purge_keys,
        before_purge,
        meta,
        &summary,
        deprecated,
        persistent,
    )
    .await?;
    let code = finish(outcome, &mut summary);
    Ok((summary, code))
}

/// Fill in the conditional `SUMMARY` keys, then name any runtime hard stop.
///
/// Every key here is emitted **only when its flag was passed**: the
/// `is_some` / `explicit` checks are the whole rule, kept in one function so
/// it can be read against the rendering order in
/// `girpr_report::summary::conditional_extras`. Returns
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
    // A failed persistent file behaves like any other failed file: no marker
    // block is published and the run exits 3. The execute half fills
    // `persistent_repaired`/`persistent_failed` when it starts downloading.
    if ctx.with_persistent && summary.files_failed.load(Ordering::Relaxed) > 0 {
        summary.flags.persistent = Some("failed");
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
async fn fetch_deprecated(hyp: &HypClient) -> Arc<[String]> {
    match hyp.deprecated_files().await {
        Ok(mut list) => {
            // Filter in place: the fetch already allocated the `Vec`, and the
            // retain reuses its buffer instead of collecting the survivors
            // into another one. The `Arc<[String]>` copies the survivors into
            // its own buffer; its header means it cannot reuse the `Vec`'s.
            normalize_deprecated(&mut list);
            list.into_iter().collect()
        }
        Err(e) => {
            tracing::warn!("deprecated list unavailable: {e:#}");
            Vec::new().into()
        }
    }
}

/// Normalise each deprecated name to the `/` form the post-phase compares
/// against, dropping (with a warning) anything that is not a plain relative
/// path. The list is a delete sink, so a name that would escape the install is
/// never followed.
fn normalize_deprecated(list: &mut Vec<String>) {
    list.retain_mut(|name| match util::clean_rel(name) {
        Ok(rel) => {
            *name = rel;
            true
        }
        Err(e) => {
            tracing::warn!(
                path = %name,
                error = %format!("{e:#}"),
                "deprecated path is not a plain relative path; ignored"
            );
            false
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{
        RunCtx, RunExit, Summary, fetch_deprecated, finish, normalize_deprecated, note_stopped,
        phases, prepare_install, spawn_blocking_traced,
    };
    use crate::Biz;
    use crate::hyp::HypClient;
    use crate::retry::BackoffPolicy;

    /// A blocking task still reaches the caller's scoped subscriber: the pool
    /// thread has no thread-local dispatcher, and the cleanup passes log the
    /// deletes they perform.
    #[tokio::test(flavor = "current_thread")]
    async fn a_blocking_task_keeps_the_callers_subscriber() {
        let (_, logs) = girpr_log::test_support::logs_of_async(async {
            spawn_blocking_traced(|| tracing::info!("blocking line"))
                .await
                .unwrap();
        })
        .await;
        assert!(logs.contains("blocking line"), "{logs}");
    }

    /// `spawn_blocking_traced` spawns eagerly: the closure runs before its
    /// handle is awaited, which is what lets a caller start a read alongside
    /// other work and collect it later.
    ///
    /// The receive is the proof: a handle that only spawned on `await` would
    /// leave it to time out. The deadline is a guard against hanging the suite,
    /// not a timing bound, because the closure signals as soon as it runs.
    #[tokio::test]
    async fn a_blocking_task_starts_before_its_handle_is_awaited() {
        const RECEIVE_WAIT: std::time::Duration = std::time::Duration::from_secs(2);
        let (tx, rx) = std::sync::mpsc::channel();
        let handle = spawn_blocking_traced(move || {
            tx.send(()).expect("the test receiver is alive");
        });

        // Nothing has awaited `handle` yet.
        rx.recv_timeout(RECEIVE_WAIT)
            .expect("the blocking closure ran before its handle was awaited");

        handle.await.expect("the blocking task finishes");
    }

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

    /// The deprecated list is normalised in place and keeps its order, while a
    /// name that is not a plain relative path is dropped and warned about: the
    /// list is a delete sink, so an absolute or escaping name must never reach
    /// it.
    #[test]
    fn deprecated_names_are_normalised_and_unsafe_ones_dropped() {
        let mut list = vec![
            "old\\legacy.dll".to_string(),
            "../victim.dll".to_string(),
            "./kept//file.dat".to_string(),
            "/abs/evil.dll".to_string(),
            "other\\stale.bin".to_string(),
            "C:/abs/evil.dll".to_string(),
            "a//b\\c.dat".to_string(),
        ];

        let logs = crate::repair::test_support::logs_of(|| normalize_deprecated(&mut list));

        assert_eq!(
            list,
            vec![
                "old/legacy.dll",
                "kept/file.dat",
                "other/stale.bin",
                "a/b/c.dat",
            ],
            "survivors keep their input order and take the / form"
        );
        assert_eq!(
            logs.matches("deprecated path is not a plain relative path; ignored")
                .count(),
            3,
            "each dropped name warns exactly once: {logs}"
        );
        for dropped in ["../victim.dll", "/abs/evil.dll", "C:/abs/evil.dll"] {
            assert!(
                logs.contains(dropped),
                "the warning names the dropped path {dropped:?}: {logs}"
            );
        }
    }

    /// The fetch path normalises the server's names before they reach the
    /// post-phase: unsafe entries never enter the delete list, and the
    /// survivors are in `/` form. Calling the helper directly would not catch
    /// a fetch that skipped it.
    #[tokio::test]
    async fn fetch_deprecated_normalises_the_server_list() {
        let server = crate::test_http::serve_bodies(vec![
            r#"{"retcode":0,"message":"OK","data":{"deprecated_file_configs":[{"deprecated_files":[{"name":"old\\legacy.dll"},{"name":"../victim.dll"},{"name":"./kept//file.dat"},{"name":"/abs/evil.dll"},{"name":"a//b\\c.dat"}]}]}}"#,
        ])
        .await;
        let hyp = HypClient::new_with_bases(
            Biz::Hk4eGlobal,
            server.uri(),
            "http://sophon.test/api".into(),
            BackoffPolicy::none(),
        )
        .expect("build the stub client");

        let list = fetch_deprecated(&hyp).await;

        assert_eq!(
            &list[..],
            ["old/legacy.dll", "kept/file.dat", "a/b/c.dat"],
            "the fetch drops unsafe names and hands on / forms in input order"
        );
    }

    /// A failed `create_dir_all` is a write-path failure (exit 3), not a bad
    /// request. The refusal carries the path and the underlying OS error so
    /// the caller can tell a full disk or a permissions problem from a typo.
    #[test]
    fn a_failed_game_dir_creation_is_a_write_failure_naming_the_path() {
        use std::collections::HashSet;

        use crate::repair::scope::Limits;
        use crate::report::OutputFormat;
        use girpr_repair_plan::PathScope;
        use girpr_repair_policy::RepairMode;

        let dir = girpr_tempdir::TempDir::new("girpr_test_repair", "mkdir_fail");
        let blocker = dir.join("blocker");
        std::fs::write(&blocker, b"not a directory").expect("a regular file, not a directory");
        let game_dir = blocker.join("child");

        let ctx = RunCtx {
            game_dir: game_dir.clone(),
            biz: Biz::Hk4eGlobal,
            audio: HashSet::new(),
            audio_explicit: false,
            path_scope: PathScope::default(),
            limits: Limits::default(),
            repair_mode: RepairMode::Auto,
            repair_mode_explicit: false,
            jobs: 1,
            with_persistent: false,
            persistent_dispatch_override: None,
            check_only: false,
            scan: false,
            purge_only: false,
            fail_fast: false,
            dry_run: false,
            print_plan: false,
            plan_out: None,
            purge_after: false,
            purge_before: false,
            output: OutputFormat::Text,
            hyp_base_override: None,
            sophon_base_override: None,
            backoff: BackoffPolicy::none(),
        };

        let err = prepare_install(&ctx, game_dir.clone(), false)
            .expect_err("a path under a regular file cannot be created");
        assert_eq!(err.exit_code, RunExit::Write);
        let msg = format!("{err:#}");
        assert!(msg.contains("create game dir"), "{msg}");
        assert!(
            msg.contains(game_dir.to_string_lossy().as_ref()),
            "the refusal names the path: {msg}"
        );
    }
}
