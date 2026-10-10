//! `girsync sync` — mirror src onto dst.
//!
//! This module is only the coordinator; the work lives in the submodules:
//!
//! - [`rename`] — fix dst casing to match src (insensitive mode)
//! - [`plan`] — turn the diff into a work list, and print it for `--dry-run`
//! - [`apply`] — execute that work list, in an order that is load-bearing
//!
//! Phase order overall:
//! 1. validate inputs, back up both caches, snapshot dst's pre-sync state,
//! 2. open both caches and run phase A on each side,
//! 3. plan across both sides at once, so a file is read only if the pair needs it,
//! 4. phase C, then rename dst paths to src's casing,
//! 5. diff, **print it**, plan, then either print the actions (`--dry-run`) or
//!    apply them.
//!
//! ## The plan print
//!
//! The diff is reported the way `compare` reports it, through
//! [`crate::report::verdict`], so `sync` and `compare` speak one vocabulary — one
//! definition of what a `CHANGED` line means, not two. Then the actions are
//! reported as they happen.
//!
//! Two segments, and the split is the point: the **plan** answers *"why did it
//! decide that"* and the **actions** answer *"what is this tool doing"*. A consumer
//! wanting every `CHANGED` with its cause reads the plan; a consumer wanting what
//! the run did reads the actions. Neither has to join streams, and an action line
//! never repeats a reason.
//!
//! A `MISSING` file therefore appears **twice** — once in the plan, once as a
//! `COPY`. That is deliberate: it is what makes the plan independently parseable
//! without correlating it against what followed.
//!
//! ## The exit code stays 0
//!
//! Printing a plan makes this command *look* like `compare`, which exits 4 on a
//! difference. **It must not.** `sync`'s exit code reports whether the run
//! *succeeded*, not whether it had anything to do, and returning 4 here would break
//! every script that syncs a partly-different tree — which is the normal case. The
//! diff never reaches [`super::report_diff`], so `Diff::total()` is not an exit code
//! on this path at all.
//! Steps 4's rename still precedes the diff and always will: the diff is taken
//! case-sensitively on exact keys. The plan in step 3 therefore runs *before* the
//! rename, paired by lowercase in insensitive mode — see the comment at the call
//! site, which is the one place the two case flags deliberately disagree.
//!
//! A record path (`girpr-cache*`) is not a valid sync input — that is
//! `compare`-only. Errors propagate as `Err`; the exit code is always 0.

mod apply;
mod plan;
mod rename;

use anyhow::{Result, bail};
use tracing::info;

use crate::cache::{CACHE_PREFIX, backup_db, remove_cache_path, snapshot_old};
use crate::config::{LogCtx, ScanMode, SyncOpts};
use crate::diff::diff_maps;
use crate::effective::{
    SideCapability, classify, ensure_distinct_sides, open_folder_cache, resolve_folder,
    scan_stat_only,
};
use crate::planner::{SideRequest, plan_pairs};
use crate::report::verdict;
use crate::util::{elapsed_s, is_record_path};

use apply::Applier;
use plan::build_plan;
use rename::rename_to_src_casing;

/// Mirror `src` onto `dst`. Returns the process exit code.
pub fn cmd_sync(opts: SyncOpts, log: &LogCtx) -> Result<i32> {
    let t0 = std::time::Instant::now();
    // One writer for every record this run emits, in the requested format. Held
    // across all three phases so `--dry-run` and a real run go through exactly
    // the same code path to say the same thing.
    let report = log.report();
    let SyncOpts {
        src,
        dst,
        trust,
        missing_only,
        keep_extra,
        jobs,
        common,
    } = opts;
    // One flag, two halves. The cache half reaches the scans through
    // `ScanMode::dry_run`; this local is the filesystem half, threaded into the
    // rename and apply phases below.
    let dry_run = common.dry_run;

    let span = tracing::info_span!(
        "girsync.sync",
        pid = std::process::id(),
        src = %src.display(),
        dst = %dst.display(),
        algos = ?common.algos,
        no_trust_src = trust.no_trust_src,
        no_trust_dst = trust.no_trust_dst,
        missing_only,
        keep_extra,
        dry_run,
        jobs,
        include = ?common.includes,
        exclude = ?common.excludes,
        case_sensitive = common.case_sensitive,
        max_depth = common.max_depth,
        ignore_cache = common.ignore_cache,
        console_level = %log.level.to_ascii_lowercase(),
        file_level = "trace",
        log_file = %log.file_display(),
        output = %log.output,
    );
    let _span_guard = span.enter();
    info!("start");

    validate_inputs(&src, &dst, jobs)?;

    // 1. Backups and snapshots, before either cache is touched.
    let src_db_path = src.join(CACHE_PREFIX);
    let dst_db_path = dst.join(CACHE_PREFIX);
    if dry_run {
        info!("dry-run: skipping backups and cache writes");
    } else {
        if src_db_path.exists() {
            backup_db(&src_db_path)?;
        }
        if dst_db_path.exists() {
            backup_db(&dst_db_path)?;
            // old-state snapshot before any dst changes
            snapshot_old(&dst_db_path)?;
        }
        if common.ignore_cache {
            for p in [&src_db_path, &dst_db_path] {
                if p.exists() {
                    remove_cache_path(p)?;
                    info!(path = %p.display(), "ignore-cache removed");
                }
            }
        }
    }

    // 2. Open both caches, then phase A on each side.
    //
    // `backup_first: false` for both: this command has already taken its own
    // backups and dst's old-state snapshot above, and doing it again here would
    // produce a second timestamped pair mid-run. `open_folder_cache` is also the
    // only thing that decides how a dry run opens a cache, so this stays a plain
    // call rather than a second policy.
    let scan_mode = |no_trust_cached_hashes| ScanMode {
        no_trust_cached_hashes,
        dry_run,
    };
    let src_mode = scan_mode(trust.no_trust_src);
    let dst_mode = scan_mode(trust.no_trust_dst);
    let src_db = open_folder_cache(&src, &common, src_mode, false)?;
    let dst_db = open_folder_cache(&dst, &common, dst_mode, false)?;
    info!("loading src side");
    let src_a = scan_stat_only(&src, &src_db, &common, src_mode)?;
    info!("loading dst side");
    let dst_a = scan_stat_only(&dst, &dst_db, &common, dst_mode)?;
    info!(
        src_entries = src_a.map.len(),
        dst_entries = dst_a.map.len(),
        "phase A done"
    );

    // 3. One decision, both sides in — the same shape `compare` uses. A file is
    // read only if it is on *both* sides with equal size and mtime; every other
    // pair state is already decided, and `diff_maps` short-circuits the digest
    // comparison anyway.
    //
    // `common.case_sensitive` here, and a hardcoded `true` in the `diff_maps` call
    // below. That asymmetry is deliberate. This pairs the sides as they are *on
    // disk*, which in insensitive mode means by lowercase — and the rename pass in
    // step 4 then collapses exactly those pairs onto exact keys, so the diff goes
    // on to form the same pairs. Passing `true` here would make a case-only pair
    // look one-sided and skip a digest it needs; passing `false` in the diff would
    // resurrect the `CASE-MISMATCH` bucket the rename pass exists to eliminate.
    let plans = plan_pairs(
        SideRequest {
            entries: &src_a.map,
            algos: &common.algos,
            no_trust: trust.no_trust_src,
            // Both sides are folders, so both can hash and `sync` can never fail
            // coverage. That is a property of the shape, not an omission: the check
            // is the same one `compare` uses, and here it is vacuous.
            cap: SideCapability::for_folder(&src_db, src_mode),
            label: &format!("folder {}", src.display()),
        },
        SideRequest {
            entries: &dst_a.map,
            algos: &common.algos,
            no_trust: trust.no_trust_dst,
            cap: SideCapability::for_folder(&dst_db, dst_mode),
            label: &format!("folder {}", dst.display()),
        },
        common.case_sensitive,
        common.hash_mode,
        common.stat,
    )?;
    info!(
        src_pending = plans.src.by_rel.len(),
        dst_pending = plans.dst.by_rel.len(),
        "planned"
    );

    // 4. Resolve, then rename, then diff.
    //
    // Resolve before rename, always. `resolve_folder` hashes `root.join(rel)`, so
    // re-keying dst first would have phase C read `dst/A.txt` while the file was
    // still at `dst/a.txt` — silent success on a case-insensitive filesystem, a
    // failure or a wrong file on a case-sensitive one. Planning pre-rename and
    // re-keying only here makes that unrepresentable, and it is why
    // `rename_to_src_casing` still takes resolved `EffRec` maps.
    let sm = resolve_folder(&src, &src_db, src_mode, &src_a, &plans.src)?;
    info!(side = "src", hashed = sm.stats.hashed, "src map built");
    let sm = sm.map;
    let dm = resolve_folder(&dst, &dst_db, dst_mode, &dst_a, &plans.dst)?;
    info!(side = "dst", hashed = dm.stats.hashed, "dst map built");
    let mut dm = dm.map;

    // 5. Align casing before diffing, so the diff can be case-sensitive.
    let renamed = if common.case_sensitive {
        0
    } else {
        rename_to_src_casing(&sm, &dst, &dst_db, &mut dm, dry_run, &report)?
    };

    // 6. Plan, then print or apply.
    //
    // `plans.required`, and it is keyed by **src** path — which survives the rename,
    // because the rename re-keys dst onto src's casing and never touches src's own
    // keys. That is the third reason the plan has to precede the rename: a per-path
    // answer is only usable if the side it is keyed by is the side that does not
    // move.
    // `false` for `want_identical`: `sync` never reports its diff — it applies it — and
    // step 4 gives it a plan print that takes the equal set only if asked.
    let diff = diff_maps(
        &sm,
        &dm,
        &plans.required,
        plans.stat,
        true, /* post-rename: exact keys */
        common.show_identical,
    );

    // The plan print, the way `compare` prints it — every record except its
    // `SUMMARY`. `sync` emits its **own** summary at the end, counting actions
    // rather than differences, so keeping the diff's would put two differently-shaped
    // summaries in one stream; a consumer reading "the last line is the summary"
    // would get the right one only by luck.
    //
    // `want_identical` above is what fills the bucket this reads, so the two flags
    // are the same question asked in two places — a plan print that could not show
    // the equal set would make `--show-identical` look like it works everywhere but
    // here.
    for rec in verdict(&diff, common.why, common.show_identical)
        .into_iter()
        .filter(|rec| rec.label() != "SUMMARY")
    {
        report.emit(rec);
    }

    let plan = build_plan(&sm, &dm, &diff, missing_only, keep_extra);
    info!(
        renamed,
        mkdir = plan.mkdir.len(),
        copy = plan.copy.len(),
        delete_files = plan.delete_files.len(),
        fix_dirs = plan.fix_dirs.len(),
        dry_run,
        "plan"
    );
    tracing::debug!(copy = ?plan.copy, "plan copy list");
    tracing::debug!(mkdir = ?plan.mkdir, "plan mkdir list");

    if dry_run {
        for rec in plan::dry_run_records(&plan, renamed, missing_only, keep_extra) {
            report.emit(rec);
        }
        info!(
            renamed,
            mkdir = plan.mkdir.len(),
            copy = plan.copy.len(),
            delete = plan.deletes(),
            dry_run = true,
            elapsed_s = elapsed_s(t0),
            exit = 0,
            "end"
        );
        return Ok(0);
    }

    let applied = Applier {
        src: &src,
        dst: &dst,
        dst_db: &dst_db,
        dm: &dm,
        common: &common,
        jobs,
        report: &report,
    }
    .apply(&plan)?;
    // No final flush: every apply phase commits its own mutations, and the
    // pre-drop commit before the filesystem changes is the crash-safety
    // point (a rerun re-copies rather than trusting half-written files).

    report.emit(plan::summary_record(
        renamed,
        &plan,
        &applied,
        missing_only,
        keep_extra,
    ));
    info!(
        renamed,
        mkdir = plan.mkdir.len(),
        copied = applied.copied,
        deleted = applied.deleted,
        rmdir = applied.removed_dirs,
        missing_only,
        keep_extra,
        dry_run = false,
        elapsed_s = elapsed_s(t0),
        exit = 0,
        "end"
    );
    Ok(0)
}

/// Reject the inputs that make a mirror meaningless, before anything is touched.
fn validate_inputs(src: &std::path::Path, dst: &std::path::Path, jobs: usize) -> Result<()> {
    if is_record_path(src) || is_record_path(dst) {
        bail!("sync needs folder vs folder (record inputs are compare-only)");
    }
    if !src.is_dir() || !dst.is_dir() {
        bail!("src and dst must both be directories");
    }
    // One cache per run: two spellings of one folder would otherwise make the
    // mirror delete src out from under itself, on top of the handle clash.
    ensure_distinct_sides(&classify(src), &classify(dst))?;
    if jobs == 0 {
        bail!("--jobs must be >= 1");
    }
    Ok(())
}
