//! `girsync update` — fully refresh a folder's cache from current disk state.

use anyhow::{Result, bail};
use tracing::info;

use crate::config::{LogCtx, ScanMode, UpdateOpts};
use crate::effective::{build_effective_folder, open_folder_cache};
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
    // update always populates: it never trusts a cached digest, so there is
    // no flag to override here.
    let scan = build_effective_folder(&dir, &db, &common, mode)?;
    let stats = scan.stats;
    // The one record `update` reports. `dir` is a field rather than part of a
    // sentence, so `--output json` can attribute the counts without parsing them
    // out of prose — and the text form keeps the historical wording.
    update_summary(&dir, stats.files, stats.dirs, &common.algos).emit(log.output);
    info!(
        files = stats.files,
        dirs = stats.dirs,
        algos = ?common.algos,
        elapsed_s = elapsed_s(t0),
        exit = 0,
        "end"
    );
    Ok(0)
}
