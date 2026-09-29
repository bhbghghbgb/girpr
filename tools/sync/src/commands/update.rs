//! `girsync update` — fully refresh a folder's cache from current disk state.

use anyhow::{Result, bail};
use tracing::info;

use crate::cache::{CACHE_PREFIX, open_db};
use crate::config::{LogCtx, ScanMode, UpdateOpts};
use crate::effective::build_effective_folder;
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
        console_level = %log.level.to_ascii_lowercase(),
        file_level = "trace",
        log_file = %log.file_display(),
    );
    let _span_guard = span.enter();
    info!("start");
    if !dir.is_dir() {
        bail!("--dir {} is not a directory", dir.display());
    }
    let db_path = dir.join(CACHE_PREFIX);
    info!(cache = %db_path.display(), "open cache");
    let db = open_db(&db_path, common.case_sensitive, common.ignore_cache, true)?;
    // update always populates: it never trusts a cached digest, so there is
    // no flag to override here.
    let eff = build_effective_folder(
        &dir,
        &db,
        &common,
        ScanMode {
            no_trust_cached_hashes: true,
            dry_run: false,
        },
    )?;
    let files = eff.values().filter(|r| r.kind == "file").count();
    let dirs = eff.values().filter(|r| r.kind == "dir").count();
    println!(
        "update {} files={} dirs={} algos=[{}]",
        dir.display(),
        files,
        dirs,
        common.algos.join(",")
    );
    info!(
        files,
        dirs,
        algos = ?common.algos,
        elapsed_s = elapsed_s(t0),
        exit = 0,
        "end"
    );
    Ok(0)
}
