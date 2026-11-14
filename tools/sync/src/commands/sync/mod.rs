//! `girsync sync` — mirror src onto dst.
//!
//! This module is only the coordinator; the work lives in the submodules:
//!
//! - [`rename`] — fix dst casing to match src (insensitive mode)
//! - [`plan`] — turn the diff into a work list, and print it for `--dry-run`
//! - [`apply`] — execute that work list, in an order that is load-bearing
//!
//! ## The phases
//!
//! `cmd_sync` is one linear function on purpose: the order of its steps is the
//! invariant, and moving one of them changes what a run reports. The phases are
//! therefore **named rather than extracted** — a phase function would have to
//! take most of `cmd_sync`'s locals, and the order would move into the
//! signatures, where it is harder to see than it is here. The numbers below are
//! the numbered comments in the body.
//!
//! | # | phase | the invariant the order protects |
//! | --- | --- | --- |
//! | 1 | validate the roots, then back up and snapshot | nothing is opened before the roots are refused, and the copies are taken before either side changes; they are a policy, not a step — `--no-backup` skips them and a dry run takes none. |
//! | 2 | open both caches, then phase A on each side | the link-disjointness check runs after **each** side's walk, so it sees the tree the filters kept, and it refuses before a plan exists. |
//! | 3 | pair plan | the sides are paired as they are **on disk**, so the plan sees the pairs the rename then collapses onto exact keys. |
//! | 4 | resolve both sides | phase C hashes only what the plan asked for, and it resolves before the rename, so it reads each path under the spelling it has on disk. |
//! | 5 | mkdir preflight | it runs before the rename, the run's first filesystem change, so a dst blocker a filter hid is refused while dst is untouched. |
//! | 6 | rename dst paths to src casing | the diff is taken case-sensitively on exact keys, so the rename comes first; the pass re-keys `dm` even in a dry run. |
//! | 7 | diff, plan and the guards | the plan print is the diff and the actions follow it; the guards refuse a plan that could not be applied, in both modes. |
//! | 8 | the dry-run records, or the apply | the apply order is not this module's — [`apply`] lists it — and its pre-drop is committed before the change to each path it touches. |
//!
//! ### Three orders that are easy to break
//!
//! **The pair plan comes before the rename.** The planner pairs the sides as
//! they are on disk, and the rename then collapses exactly those pairs onto
//! exact keys. Planning *after* the rename, with exact pairing, would make a
//! case-only pair look one-sided, skip the digest it needs, and leave the diff
//! comparing two entries where one has no digest — `hashes_differ` is silent on
//! a missing digest, size and mtime agree, and the run reports success having
//! copied nothing. Pinned by `a_case_only_difference_in_content_is_copied_not_merely_renamed`
//! in `tests/sync.rs`, whose two sides differ in content, and by the `Case.txt`
//! row of the table in `tests/sync_plan/`.
//!
//! **The rename comes before the diff.** The diff pairs exactly, so it can only
//! run once the rename has collapsed the two spellings: a diff taken first
//! would find src's `Data.txt` and dst's `data.txt` to be two unrelated paths
//! and plan a copy and a delete where the rename belongs, and a diff that
//! paired by the ASCII-case fold instead would report the `CASE-MISMATCH` bucket
//! the rename pass exists to eliminate (`a_case_only_pair_is_reported_but_not_counted_as_a_difference`
//! in `src/diff/mod.rs` is that bucket's own shape). Pinned by
//! `the_dry_run_does_exactly_what_the_table_says` in `tests/sync_plan/`, whose
//! expected records put the rename ahead of the plan and report nothing else
//! for a case-only pair.
//!
//! **The pre-drop comes before the filesystem change.** The dst cache rows for
//! every path a run will copy or delete are dropped *and committed* before
//! those changes, so an interrupted run copies again rather than trusting a
//! half-written file. That order belongs to the apply phases — [`apply`] states
//! it — and it is belt-and-braces rather than the mechanism: truncation, the
//! deferred mtime and the verify each catch a failed copy on their own. What a
//! test can see is the other half of the same fact: a copy which fails leaves
//! no row claiming content the dst does not have
//! (`a_failed_flush_aborts_before_the_copy_is_cached` and
//! `a_failed_copy_discards_only_the_uncommitted_cache_tail` in `tests/sync.rs`),
//! so the rerun copies the file again.
//!
//! ## The plan print
//!
//! The diff is reported the way `compare` reports it, through
//! [`crate::report::visit_verdict`], so `sync` and `compare` speak one vocabulary
//! — one definition of what a `CHANGED` line means, not two. Then the actions are
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
//! The rename still precedes the diff and always will: the diff is taken
//! case-sensitively on exact keys. The plan therefore runs *before* the
//! rename, paired by the ASCII-case fold in insensitive mode — see the comment
//! at the call site, which is the one place the two case flags deliberately
//! disagree.
//!
//! A record path (`girpr-cache*`) is not a valid sync input — that is
//! `compare`-only. Errors propagate as `Err`; the exit code is always 0.

mod apply;
mod plan;
mod rename;

use anyhow::{Context, Result, bail};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::info;

use crate::cache::{CACHE_PREFIX, backup_db, remove_cache_path, snapshot_old};
use crate::config::{LogCtx, ScanMode, SyncOpts};
use crate::diff::diff_maps;
use crate::effective::{
    Side, SideCapability, SideEntry, classify, ensure_distinct_sides, open_folder_cache,
    resolve_folder, resolve_sides_concurrently, scan_stat_only,
};
use crate::planner::{CasePairing, SideRequest, plan_pairs};
use crate::report::visit_verdict;
use crate::rw::{RwRuntime, RwSide};
use crate::util::{canonical_or_raw, elapsed_s, is_record_path, path_within};

use apply::Applier;
use plan::{
    SystemProbes, build_plan, ensure_dirs_applicable, ensure_hard_linked_targets_safe,
    ensure_link_safe, ensure_mkdir_targets_free, spare_link_storage,
};
use rename::{RenameOutcome, rename_to_src_casing};

use super::{side_folder, side_label};

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
        // The shape of the run's concurrency, not just its size: "shared(4)" and
        // "split(src=4, dst=4)" both allow four concurrent operations and schedule
        // very differently, so one number would lose the half that matters.
        rw_mode = common.rw.mode(),
        rw_threads_src = common.rw.src_threads(),
        rw_threads_dst = common.rw.dst_threads(),
        include = ?common.filter.includes(),
        exclude = ?common.filter.excludes(),
        case_sensitive = common.case_sensitive,
        max_depth = common.max_depth,
        ignore_cache = common.ignore_cache,
        console_level = %log.level,
        file_level = %log.file_level,
        log_file = %log.file_display(),
        output = %log.output,
    );
    let _span_guard = span.enter();
    info!("start");

    // 1. Validate the roots, then back up and snapshot, before either cache is
    //    touched.
    let roots = validate_inputs(&src, &dst)?;

    // One runtime for the whole run: one limiter, one worker pool. A runtime per
    // phase would be two limiters wearing one flag's name, and two pools would
    // each size themselves from the same number.
    let rw = RwRuntime::new(common.rw)?;
    let threads = rw.pool()?.current_num_threads();
    info!(
        rw = %rw.describe(),
        threads,
        "rw limits"
    );

    let src_db_path = src.join(CACHE_PREFIX);
    let dst_db_path = dst.join(CACHE_PREFIX);
    if dry_run {
        info!("dry-run: skipping backups and cache writes");
    } else {
        // The copies are a policy, not a step: `--no-backup` skips them, and
        // `--backup-retain N` bounds what they leave behind. Neither changes
        // what the run answers.
        //
        // src gets a `backup-*` copy of its cache; dst gets an `old-*`
        // snapshot of its pre-sync state and nothing else. The snapshot *is*
        // the record of what dst held before the mirror moved, so a second
        // copy of the same bytes under another name would be a third copy of
        // one file.
        //
        // The bound is not a step here either: it is applied when the caches
        // are opened below, because it is about the folder and not about this
        // run's copy — a folder whose cache is absent, and a `--no-backup`
        // run, are bounded just the same.
        if common.backup.wants_backup() {
            if src_db_path.exists() {
                backup_db(&src_db_path)?;
            }
            if dst_db_path.exists() {
                // old-state snapshot before any dst changes
                snapshot_old(&dst_db_path)?;
            }
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
    // backup of src and dst's old-state snapshot above, and doing it again
    // here would produce a second timestamped pair mid-run. `open_folder_cache`
    // is also the only thing that decides how a dry run opens a cache, so this
    // stays a plain call rather than a second policy.
    let scan_mode = |no_trust_cached_hashes| ScanMode::new(no_trust_cached_hashes, dry_run);
    let src_mode = scan_mode(trust.no_trust_src);
    let dst_mode = scan_mode(trust.no_trust_dst);
    let src_db = open_folder_cache(&src, &common, src_mode, false)?;
    let dst_db = open_folder_cache(&dst, &common, dst_mode, false)?;
    info!("loading src side");
    let src_a = scan_stat_only(&src, &src_db, &common, src_mode)?;
    // The effective source tree is what the mirror would copy, so a link in it
    // that resolves to `dst` (or to a directory containing `dst`) is the same
    // self-overlap `validate_inputs` refuses for the roots. It has to be caught
    // here rather than from the raw paths: only the walk knows which links the
    // filters kept and where each one points. Nothing has been written to the
    // mirror at this point, and the cache write in phase A is not a content
    // change.
    validate_src_links(&src, &src_a.map, &roots.dst)?;
    info!("loading dst side");
    let dst_a = scan_stat_only(&dst, &dst_db, &common, dst_mode)?;
    // The mirror image of `validate_src_links`: a link inside dst that resolves
    // to src (or to a directory containing it) makes the two trees overlap
    // through the walk, and a copy through a final-component file link would
    // truncate a source file. Caught after the dst walk and before the plan, the
    // rename and every apply phase, so nothing in either tree has changed yet.
    validate_dst_links(&dst, &dst_a.map, &roots.src)?;
    info!(
        src_entries = src_a.map.len(),
        dst_entries = dst_a.map.len(),
        "phase A done"
    );

    // 3. Pair plan: one decision, both sides in — the same shape `compare`
    // uses. A file is read only if it is on *both* sides with equal size and
    // mtime; every other pair state is already decided, and `diff_maps`
    // short-circuits the digest comparison anyway.
    //
    // `CasePairing::from(common.case_sensitive)` here, and `CasePairing::Exact`
    // in the `diff_maps` call below. **The two must differ**, and the values say
    // so where a bare `true`/`false` pair could not. This pairs the sides as they
    // are *on disk*, which in insensitive mode means by the ASCII-case fold — and
    // the rename pass then collapses exactly those pairs onto exact keys, so the
    // diff goes on to form the same pairs. Planning post-rename would make a
    // case-only pair look one-sided and skip a digest it needs; diffing
    // pre-rename would resurrect the `CASE-MISMATCH` bucket the rename pass
    // exists to eliminate.
    // The two side labels, built before the plan rather than inside it.
    //
    // A label is how a side is named in a coverage error, so it is a value the
    // plan borrows — and a value built inline in the argument list reads as
    // though it were borrowed from a temporary that has already gone. Built
    // here, each label is a name that outlives the call.
    //
    // Built by the shared helper rather than spelled out here: a side is named
    // the same way in every command, and `sync` is no exception — both of its
    // sides are folders, and the helper says so.
    let src_side = Side::Folder(src.clone());
    let dst_side = Side::Folder(dst.clone());
    let src_label = side_label(&src_side);
    let dst_label = side_label(&dst_side);
    // The folders a repopulation would run on, for the same reason the labels
    // are built here: both sides are folders, so neither has a record of its
    // own to repopulate.
    let src_folder = side_folder(&src_side);
    let dst_folder = side_folder(&dst_side);
    let plans = plan_pairs(
        SideRequest {
            entries: &src_a.map,
            algos: &common.algos,
            no_trust: trust.no_trust_src,
            // Both sides are folders, so both can hash and `sync` can never fail
            // coverage. That is a property of the shape, not an omission: the check
            // is the same one `compare` uses, and here it is vacuous.
            cap: SideCapability::for_folder(&src_db, src_mode),
            label: &src_label,
            // And a folder side has no record to repopulate either.
            folder: src_folder,
        },
        SideRequest {
            entries: &dst_a.map,
            algos: &common.algos,
            no_trust: trust.no_trust_dst,
            cap: SideCapability::for_folder(&dst_db, dst_mode),
            label: &dst_label,
            folder: dst_folder,
        },
        CasePairing::from(common.case_sensitive),
        common.hash_mode,
        common.stat,
    )?;
    info!(
        src_pending = plans.src.by_rel.len(),
        dst_pending = plans.dst.by_rel.len(),
        "planned"
    );

    // 4. Resolve both sides, before the rename and the diff.
    //
    // Resolve before rename, always. `resolve_folder` hashes `root.join(rel)`, so
    // re-keying dst first would have phase C read `dst/A.txt` while the file was
    // still at `dst/a.txt` — silent success on a case-insensitive filesystem, a
    // failure or a wrong file on a case-sensitive one. Planning pre-rename and
    // re-keying only here makes that unrepresentable, and it is why
    // `rename_to_src_casing` still takes resolved `EffRec` maps.
    //
    // The two resolves run at once. That is a no-op under one shared counter — the
    // gate serialises them, which is why this is not a branch on the mode — and
    // the point of a split limit, where src being read does not wait for dst to
    // finish being read. `ensure_distinct_sides` has already guaranteed the two
    // caches are different files, so the two never touch one database.
    let (src_res, dst_res) = resolve_sides_concurrently(
        || {
            resolve_folder(
                &src,
                &src_db,
                src_mode,
                &src_a,
                &plans.src,
                RwSide::Src,
                &rw,
            )
        },
        || {
            resolve_folder(
                &dst,
                &dst_db,
                dst_mode,
                &dst_a,
                &plans.dst,
                RwSide::Dst,
                &rw,
            )
        },
    );
    // src first, so the message a broken tree produces does not depend on which
    // side's hash happened to fail sooner.
    let sm = src_res?;
    info!(side = "src", hashed = sm.stats.hashed, "src map built");
    let sm = sm.map;
    let dm = dst_res?;
    info!(side = "dst", hashed = dm.stats.hashed, "dst map built");
    let mut dm = dm.map;

    // 5. mkdir preflight — refuse a mkdir dst cannot create **before** the
    // rename, which is the first filesystem change. A filter can hide the
    // blocking dst entry from `dm`, so this asks the disk about every mkdir
    // candidate's path and strict prefixes; the map-based
    // `ensure_dirs_applicable` below cannot see them.
    ensure_mkdir_targets_free(&sm, &dm, &dst, common.case_sensitive, &SystemProbes)?;

    // 6. Rename dst paths to src casing, before the diff, so the diff can be
    // case-sensitive.
    let RenameOutcome { renamed, on_disk } = if common.case_sensitive {
        RenameOutcome::default()
    } else {
        rename_to_src_casing(&sm, &dst, &dst_db, &mut dm, dry_run, &report)?
    };

    // 7. Diff, plan and the guards.
    //
    // `plans.required`, and it is keyed by **src** path — which survives the rename,
    // because the rename re-keys dst onto src's casing and never touches src's own
    // keys. That is the third reason the plan has to precede the rename: a per-path
    // answer is only usable if the side it is keyed by is the side that does not
    // move.
    // `CasePairing::Exact`: post-rename the two maps line up on exact keys, and
    // the plan above was taken as on disk. See the note there on why the two
    // differ.
    // `false` for `want_identical`: `sync` never reports its diff — it applies it — and
    // the plan print takes the equal set only if asked.
    let mut diff = diff_maps(
        &sm,
        &dm,
        &plans.required,
        plans.stat,
        CasePairing::Exact,
        common.show_identical,
    );

    let mut plan = build_plan(&sm, &dm, &diff, missing_only, keep_extra);
    // Before the plan print: a dst file link and a target inside dst are one
    // file. The plan must not delete the target the write-through needs, and it
    // must not ask the two names to hold two contents. The diff's `EXTRA` record
    // is dropped with the delete, so the printed plan and the applied plan name
    // the same set.
    spare_link_storage(
        &sm,
        &dm,
        &mut plan,
        &mut diff,
        &dst,
        on_disk.as_ref(),
        &SystemProbes,
    )?;

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
    visit_verdict(&diff, common.why, common.show_identical, |rec| {
        if rec.label() != "SUMMARY" {
            report.emit(rec);
        }
    });

    // Before the dry-run branch: a plan that cannot be applied is not a plan a
    // dry run should print as if it could be. The apply passes re-check the
    // paths they touch; this is the one place that fails before *any* of them.
    ensure_link_safe(&plan, &dst, missing_only, keep_extra, on_disk.as_ref())?;
    // The phase order makes a file ancestor creatable only when `fix_dirs`
    // replaces it first. Today's planner always does; a future one that stops
    // must fail here, at plan time, rather than midway through apply.
    ensure_dirs_applicable(&plan, &dm)?;
    // The copy guard's hard-link refusal must be a decision both modes take:
    // `copy_one` runs only in a real run, and a dry run that reported `COPY` for
    // a tree the real run refuses would answer a different question. A planned
    // delete of the other name (the default run) is credited; one left in place
    // (`--keep-extra`, or outside `dst`) is refused here.
    ensure_hard_linked_targets_safe(&plan, &dm, &dst, on_disk.as_ref(), &SystemProbes)?;
    info!(
        renamed,
        mkdir = plan.mkdir.len(),
        copy = plan.copy.len(),
        delete_files = plan.delete_files.len(),
        fix_dirs = plan.fix_dirs.len(),
        dry_run,
        "plan"
    );
    tracing::debug!(copy = ?plan.copy.iter().map(|i| &i.rel).collect::<Vec<_>>(), "plan copy list");
    tracing::debug!(mkdir = ?plan.mkdir, "plan mkdir list");

    // 8. The dry-run records, or the apply.
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
        src_canonical: &roots.src,
        dst: &dst,
        dst_db: &dst_db,
        dm: &dm,
        common: &common,
        rw: &rw,
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

/// The two roots as the identity checks compare them.
struct CanonicalRoots {
    src: PathBuf,
    dst: PathBuf,
}

/// Reject the inputs that make a mirror meaningless, before anything is touched.
fn validate_inputs(src: &Path, dst: &Path) -> Result<CanonicalRoots> {
    if is_record_path(src) || is_record_path(dst) {
        bail!("sync needs folder vs folder (record inputs are compare-only)");
    }
    if !src.is_dir() || !dst.is_dir() {
        bail!("src and dst must both be directories");
    }
    // One cache per run: two spellings of one folder would otherwise make the
    // mirror delete src out from under itself, on top of the handle clash.
    ensure_distinct_sides(&classify(src), &classify(dst))?;
    // A mirror may not contain its own source. With `dst` inside `src` the walk
    // of src sees dst as part of the source, and the copy plan writes a parent's
    // namesake over the child's file; with `src` inside `dst` the delete pass
    // removes the source files before the copy that needed them. Canonical
    // paths, so a `..` or a case-only spelling cannot hide the overlap. Component-wise and case-insensitive on
    // Windows, via `path_within`.
    //
    // A link *inside* a root is not visible to this check; `validate_src_links`
    // and `validate_dst_links` cover both directions once the walks have said
    // which links each effective tree actually contains.
    let src_canonical = canonical_or_raw(src);
    let dst_canonical = canonical_or_raw(dst);
    if path_within(&dst_canonical, &src_canonical) {
        bail!(
            "src and dst must be disjoint: dst `{}` is inside src `{}`; \
             a mirror cannot write into its own source",
            dst.display(),
            src.display()
        );
    }
    if path_within(&src_canonical, &dst_canonical) {
        bail!(
            "src and dst must be disjoint: src `{}` is inside dst `{}`; \
             syncing would delete the source",
            src.display(),
            dst.display()
        );
    }
    Ok(CanonicalRoots {
        src: src_canonical,
        dst: dst_canonical,
    })
}

/// Which side of the mirror a link check walks.
#[derive(Clone, Copy)]
enum LinkSide {
    Src,
    Dst,
}

impl LinkSide {
    /// The side's own name, as it appears in diagnostics.
    const fn name(self) -> &'static str {
        match self {
            Self::Src => "src",
            Self::Dst => "dst",
        }
    }

    /// The name of the root the side must stay clear of.
    const fn other(self) -> &'static str {
        match self {
            Self::Src => "dst",
            Self::Dst => "src",
        }
    }

    /// What the link would wrongly reach, for a target inside the other root.
    const fn inside_consequence(self) -> &'static str {
        match self {
            Self::Src => "read its own destination",
            Self::Dst => "write into its own source",
        }
    }

    /// What the link would wrongly reach, for a target containing the other root.
    const fn contains_consequence(self) -> &'static str {
        match self {
            Self::Src => "reach its own destination",
            Self::Dst => "reach its own source",
        }
    }
}

/// Refuse an effective tree that reaches the other root through a link.
///
/// The walk follows links, so a link inside either root can expose the other
/// root's files while the two roots still look like siblings. A link is legal
/// when it points anywhere outside the other root; it is refused when its
/// resolved target and the other root intersect in either direction:
///
/// - the target is the other root or inside it, so the link reaches that side;
/// - the target contains the other root, so walking the link reaches it from
///   above.
///
/// Every link the effective map kept is checked, not only directories: a file
/// link can point at a file on the other side just as well. The caller runs
/// this after its side's phase A and before the plan, the rename and every
/// apply phase, so nothing in either tree has changed yet. Filters and
/// `--max-depth` are already applied here, which is why an excluded link is not
/// refused: it is not part of this run's tree.
fn validate_links_disjoint(
    root: &Path,
    entries: &HashMap<String, SideEntry>,
    other_canonical: &Path,
    side: LinkSide,
) -> Result<()> {
    for (rel, ent) in entries {
        if !ent.is_link {
            continue;
        }
        let target = std::fs::canonicalize(root.join(rel))
            .with_context(|| format!("resolve {} link `{rel}`", side.name()))?;
        if path_within(&target, other_canonical) {
            bail!(
                "src and dst must be disjoint: {} link `{rel}` resolves to `{}`, \
                 which is inside {} `{}`; a mirror may not {} \
                 through a link (remove the link, or exclude it with --exclude)",
                side.name(),
                target.display(),
                side.other(),
                other_canonical.display(),
                side.inside_consequence()
            );
        }
        if path_within(other_canonical, &target) {
            bail!(
                "src and dst must be disjoint: {} link `{rel}` resolves to `{}`, \
                 which contains {} `{}`; a mirror may not {} \
                 through a link (remove the link, or exclude it with --exclude)",
                side.name(),
                target.display(),
                side.other(),
                other_canonical.display(),
                side.contains_consequence()
            );
        }
    }
    Ok(())
}

/// Refuse an effective source tree that reaches `dst` through a link.
///
/// The root check above canonicalizes only the two roots. The walk follows
/// links (`scan::walk_live`), so a directory link inside `src` can expose
/// `dst`'s files under the link's relative path while the two roots still look
/// like siblings. Those files then look `EXTRA` on the dst side, the delete
/// pass removes them, and the copy that needed them fails afterwards: data
/// loss reported only as a copy error.
///
/// A link is legal when it points anywhere outside `dst`; that is ordinary
/// mirrored content (a target *inside* `src` included). It is refused when its
/// resolved target and `dst` intersect in either direction:
///
/// - the target is `dst` or inside it, so the link exposes dst's own files;
/// - the target contains `dst`, so walking the link reaches dst from above.
///
/// This checks every link the effective map kept, not only directories: a
/// file link can point at a dst file just as well. It runs after phase A and
/// before the plan, the rename and every apply phase, so nothing in the mirror
/// has changed yet. Filters and `--max-depth` are already applied here, which
/// is why an excluded link is not refused: it is not part of this run's source.
fn validate_src_links(
    src: &Path,
    entries: &HashMap<String, SideEntry>,
    dst_canonical: &Path,
) -> Result<()> {
    validate_links_disjoint(src, entries, dst_canonical, LinkSide::Src)
}

/// Refuse an effective destination tree that reaches `src` through a link.
///
/// The mirror of [`validate_src_links`], and the same invariant: the two trees
/// must stay disjoint. The root check canonicalizes only the two roots, and the
/// walk follows links inside a root, so a `dst` link into `src` can make a copy
/// land on a source file. A **file** link at the copy path is the dangerous
/// shape: `File::create` follows the final component, truncates the source, and
/// the verify then reads the same file through the link, so the run can exit 0
/// having destroyed source data. A directory link is unlinked rather than
/// traversed, but refusing it here keeps one rule for the whole dst side.
///
/// Refused when the resolved target is `src` or inside it, or when it contains
/// `src` (a write through the link would land above the source root). A link to
/// anywhere else is ordinary dst content: `sync_writes_through_a_dst_file_link`
/// depends on write-through to a target inside `dst` staying legal. A dst-only
/// link into src is refused even when this run would only unlink it — the check
/// is about where the link resolves, not about what today's plan does with it.
///
/// Runs immediately after dst's phase A, so `--include`/`--exclude` and
/// `--max-depth` have already decided what this run's destination is, and before
/// the plan, the rename and every apply phase, so neither tree has changed yet.
fn validate_dst_links(
    dst: &Path,
    entries: &HashMap<String, SideEntry>,
    src_canonical: &Path,
) -> Result<()> {
    validate_links_disjoint(dst, entries, src_canonical, LinkSide::Dst)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::DigestSet;

    fn link_entry() -> SideEntry {
        SideEntry {
            kind: "dir".into(),
            size: 0,
            mtime_ns: 0,
            cached: DigestSet::default(),
            fresh: true,
            is_link: true,
        }
    }

    /// A link the walk kept can still fail to resolve by the time this check
    /// runs; the error has to name the side whose root the user must fix.
    #[test]
    fn an_unresolvable_link_names_its_side() {
        let t = girpr_tempdir::TempDir::new("girsync_sync", "resolve_link");
        let root = t.subdir("root");
        let other = t.subdir("other");
        let mut entries = HashMap::new();
        entries.insert("gone".to_string(), link_entry());

        for (side, expected) in [
            (LinkSide::Src, "resolve src link `gone`"),
            (LinkSide::Dst, "resolve dst link `gone`"),
        ] {
            let err = validate_links_disjoint(&root, &entries, &other, side)
                .expect_err("a missing target cannot be resolved");
            let msg = format!("{err:#}");
            assert!(msg.contains(expected), "expected `{expected}` in: {msg}");
        }
    }
}
