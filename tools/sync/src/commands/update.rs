//! `girsync update` — fully refresh a folder's cache from current disk state.

use anyhow::{Result, bail};
use tracing::info;

use crate::config::{LogCtx, ScanMode, UpdateOpts};
use crate::effective::{open_folder_cache, resolve_folder, scan_stat_only};
use crate::planner::HashPlan;
use crate::report::update_summary;
use crate::util::elapsed_s;

/// Rebuild `<dir>/girpr-cache` from scratch: stat + hash every file, record
/// empty dirs as presence-only, prune rows for deleted or filtered-out paths.
/// The old DB is backed up first.
///
/// "From scratch" describes the row set, not the digests. `update` never trusts
/// a cached digest, so every requested algo is recomputed on every run — but a
/// file whose size+mtime are unchanged keeps any *other* algorithm already on
/// its row, and `--hash none` therefore records stat data without touching
/// stored digests at all. A file whose stat *did* change has all of its digests
/// dropped, including algorithms this run did not request, because the content
/// is assumed to have changed with it. Ask for every algorithm you want
/// refreshed; the pre-run `girpr-cache-backup-*` is the manual way back.
pub fn cmd_update(opts: UpdateOpts, log: &LogCtx) -> Result<i32> {
    let t0 = std::time::Instant::now();
    let UpdateOpts { dir, common } = opts;
    let span = tracing::info_span!(
        "girsync.update",
        pid = std::process::id(),
        dir = %dir.display(),
        algos = ?common.algos,
        include = ?common.includes,
        exclude = ?common.excludes,
        case_sensitive = common.case_sensitive,
        max_depth = common.max_depth,
        ignore_cache = common.ignore_cache,
        dry_run = common.dry_run,
        console_level = %log.level.to_ascii_lowercase(),
        file_level = "trace",
        log_file = %log.file_display(),
        output = %log.output,
    );
    let _span_guard = span.enter();
    info!("start");
    if !dir.is_dir() {
        bail!("--dir {} is not a directory", dir.display());
    }
    // `update`'s whole output *is* the cache, so `--dry-run` here is not a
    // convenience — it is the only way to ask "what would a repopulate do, and
    // how much would it read?" without committing to it. It still hashes every
    // file, because `update` trusts nothing and a dry run that skipped the
    // hashing would not be answering the same question.
    let mode = ScanMode {
        no_trust_cached_hashes: true,
        dry_run: common.dry_run,
    };
    let db = open_folder_cache(&dir, &common, mode, true)?;

    // The three phases, run here rather than inside a helper, which is the shape
    // `compare`, `sync` and `compare-self` use. What makes it *this* command's plan
    // is that there is no other side to be lazy relative to: every file is a
    // candidate, so `plan_one_side` asks for every requested algorithm on every one
    // of them, minus nothing (trust is denied above).
    //
    // Which means `no_trust_cached_hashes` has to be forwarded *here*, into the
    // planner, rather than living inside a call the reader cannot see. That
    // forwarding is the whole behavioural contract of `update`, and
    // `update_recomputes_every_digest_and_repairs_a_wrong_one` in `tests/update.rs`
    // exists to notice if it stops happening — idempotence cannot, because a reused
    // digest and a recomputed one are the same bytes.
    let phase_a = scan_stat_only(&dir, &db, &common, mode)?;
    let plan = HashPlan::plan_one_side(&phase_a.map, &common.algos, mode.no_trust_cached_hashes);
    info!(
        entries = phase_a.map.len(),
        pending = plan.pending().len(),
        digests = plan.digest_count(),
        "planned"
    );
    let resolved = resolve_folder(&dir, &db, mode, &phase_a, &plan)?;

    // One stats value, and it is complete. `resolve_folder` carries phase A's
    // `live`/`files`/`dirs`/`pruned` forward and fills in the three counters phase C
    // owns, so its returned `ScanStats` is the whole run. Which is why the merge
    // `build_effective_folder` performed was duplicating work already done, and why
    // nothing here has to decide which phase a number came from.
    //
    // Mutation-tested both ways: reading `files` from `phase_a.stats` and from
    // `resolved.stats` are indistinguishable, because they are the same value — so
    // this is a choice about legibility, not correctness, and saying so is cheaper
    // than letting a reader assume otherwise. Naming `resolved.stats` once is the
    // version that cannot drift if the two ever stop agreeing.
    //
    // `hashed` is logged because it is the answer to the question `--dry-run` exists
    // to ask — how much would this read? A file count cannot distinguish a run that
    // rehashed everything from one that read nothing.
    update_summary(
        &dir,
        resolved.stats.files,
        resolved.stats.dirs,
        &common.algos,
    )
    .emit(log.output);
    info!(
        files = resolved.stats.files,
        dirs = resolved.stats.dirs,
        hashed = resolved.stats.hashed,
        algos = ?common.algos,
        elapsed_s = elapsed_s(t0),
        exit = 0,
        "end"
    );
    Ok(0)
}
