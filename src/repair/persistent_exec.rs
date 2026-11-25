//! The execute half of `--with-persistent`: prerequisites and full-file bytes.
//!
//! Everything here is inert unless the caller gated it on the master flag and
//! ruled out `--purge-only`. Read-only modes never reach a write: they list the
//! planned assets and return. The plan (`persistent::PersistentState`) is the
//! sole source of rels, URLs and digests — the exe stem comes from
//! `meta.config.exe_file_name`, never a hardcoded string.
//!
//! This module deliberately publishes no marker and no mirror: that is the
//! post phase's job (commit 8). A failed hotfix counts as a failed file so the
//! run exits 3 and skips the post phase, which is the all-or-nothing gate.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use anyhow::{Context, anyhow};
use futures_util::stream::{StreamExt, iter};
use girpr_fetch::persistent::{
    DownloadOutcome, DownloadRequest, WriteCeilingStopped, download_hotfix, hotfix_is_intact,
};
use girpr_persistent_plan::{PersistentAsset, PersistentRow, data_dir_name};
use girpr_util_path::atomic::write_atomic;
use reqwest::Client;
use walkdir::WalkDir;

use super::persistent::PersistentState;
use crate::config::{RunCtx, RunFailure};
use crate::report::{StoppedBy, Summary};
use crate::util;

/// Execute the persistent half: cheap prerequisites first, then one full-file
/// download per planned asset.
///
/// `summary` is taken by value and returned, like `file::repair_all`, because
/// the bounded window drives tasks that share the atomics. The caller installs
/// the runtime ceilings before this call (through `run_repair`'s normal path)
/// so every byte charged here is bounded by the same meters as a base byte.
pub(crate) async fn repair(
    ctx: &RunCtx,
    http: &Client,
    game_dir: &Path,
    exe_file_name: &str,
    state: &PersistentState,
    mut summary: Summary,
) -> Summary {
    // C4: a read-only run lists the work, spends no byte and touches no tree.
    // `--check-only` returned before this call; `--dry-run` reaches it.
    if ctx.readonly() {
        for asset in &state.plan.assets {
            tracing::info!(
                path = %asset.rel,
                bytes = asset.size,
                "would repair persistent file"
            );
        }
        return summary;
    }

    if let Err(e) = prerequisites(ctx, game_dir, exe_file_name, state) {
        tracing::error!(
            error = %format!("{e:#}"),
            "persistent prerequisites failed; no hotfix was attempted"
        );
        summary.files_failed.fetch_add(1, Ordering::Relaxed);
        summary.flags.persistent_repaired = Some(0);
        summary.flags.persistent_failed = Some(1);
        return summary;
    }

    let shared = Arc::new(summary);
    let tasks = state.plan.assets.iter().map(|asset| {
        let shared = Arc::clone(&shared);
        let backoff = &ctx.backoff;
        async move { download_one(backoff, http, game_dir, asset, &shared).await }
    });
    // `--jobs` is the file-task window; a smaller value simply serialises more.
    let _results = iter(tasks)
        .buffer_unordered(ctx.jobs.max(1))
        .collect::<Vec<_>>()
        .await;

    let mut summary = Arc::try_unwrap(shared).unwrap_or_else(|a| a.snapshot());
    summary.flags.persistent_repaired = Some(summary.files_repaired.load(Ordering::Relaxed));
    summary.flags.persistent_failed = Some(summary.files_failed.load(Ordering::Relaxed));
    summary
}

/// What happened to one planned asset. A stop is its own outcome: no file
/// failure is added, because `stopped=` is the run's reason (exit 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AssetOutcome {
    Repaired,
    Skipped,
    Failed,
    Stopped,
}

/// One asset: skip when intact, reserve download bytes, then download, gate
/// and charge write bytes **before** the publish.
///
/// A ceiling stop does **not** add a file failure: `stopped=` is the run's
/// reason (exit 5), exactly as it is for a base file that hit a live ceiling.
/// The shared summary is held by `Arc` because the pre-publish charge callback
/// must own its meters: with `--jobs N`, N files can be in flight at once, and
/// the write ceiling has to bound all of them at the moment bytes land.
async fn download_one(
    backoff: &crate::retry::BackoffPolicy,
    http: &Client,
    game_dir: &Path,
    asset: &PersistentAsset,
    summary: &Arc<Summary>,
) -> AssetOutcome {
    if summary.stopped_by() != StoppedBy::None || summary.aborted() {
        return AssetOutcome::Stopped;
    }

    let row = row_of(asset);
    let target = match target_of_checked(game_dir, asset) {
        Ok(target) => target,
        Err(e) => {
            tracing::error!(path = %asset.rel, error = %format!("{e:#}"), "persistent path refused");
            summary.files_failed.fetch_add(1, Ordering::Relaxed);
            return AssetOutcome::Failed;
        }
    };

    // An intact target is a pure no-op: no reservation, no request, no write.
    if hotfix_is_intact(&row, &target) {
        tracing::debug!(path = %asset.rel, "persistent file is already intact");
        summary.files_skipped.fetch_add(1, Ordering::Relaxed);
        return AssetOutcome::Skipped;
    }
    tracing::info!(
        path = %asset.rel,
        bytes = asset.size,
        "repairing persistent file"
    );

    // Charge before the request so a live ceiling stops the run exactly, with
    // N assets in flight, instead of overshooting by N-1 bodies.
    if !summary.reserve_download_bytes(asset.size) {
        summary.stop(StoppedBy::Download);
        tracing::error!(path = %asset.rel, "--max-download-bytes stopped persistent repair");
        return AssetOutcome::Stopped;
    }

    // Charge the write meters inside the downloader, before it publishes.
    // Charging after `download_hotfix` returns would let `--jobs N` land N
    // files before the first reservation — `--max-write-bytes 0` still wrote
    // one. Both meters are charged here, with separate checks, so a refused
    // live reservation no longer skips the settled one.
    let rel = asset.rel.clone();
    let shared = Arc::clone(summary);
    let charge_writes: Arc<dyn Fn(u64) -> bool + Send + Sync> = Arc::new(move |bytes: u64| {
        if !shared.reserve_write_bytes(bytes) {
            shared.stop(StoppedBy::Write);
            tracing::error!(path = %rel, "--max-write-bytes stopped persistent repair");
            return false;
        }
        if !shared.note_write_bytes(bytes) {
            shared.stop(StoppedBy::VerifiedWrite);
            tracing::error!(
                path = %rel,
                "--max-verified-write-bytes stopped persistent repair"
            );
            return false;
        }
        true
    });
    let mut request = DownloadRequest::new(
        http,
        backoff,
        &asset.primary_url,
        &asset.alternate_url,
        &row,
        &target,
    );
    request.before_publish = Some(charge_writes);
    match download_hotfix(request).await {
        Ok(outcome) => {
            let bytes = outcome.bytes();
            if outcome.wrote() {
                if !summary.settle_download_bytes(bytes) {
                    summary.stop(StoppedBy::VerifiedDownload);
                }
                summary.files_repaired.fetch_add(1, Ordering::Relaxed);
                if let DownloadOutcome::TempFallback { path, .. } = outcome {
                    tracing::warn!(
                        path = %path.display(),
                        "read-only target kept the verified bytes at its temp sibling"
                    );
                }
                AssetOutcome::Repaired
            } else {
                // A concurrent task may have published the same rel first.
                summary.adjust_download_bytes(-(i64::try_from(bytes).unwrap_or(i64::MAX)));
                summary.files_skipped.fetch_add(1, Ordering::Relaxed);
                AssetOutcome::Skipped
            }
        }
        Err(e) if e.downcast_ref::<WriteCeilingStopped>().is_some() => {
            // The charge callback already tripped the matching hard-stop
            // marker. No bytes were published, so this is not a failed file.
            AssetOutcome::Stopped
        }
        Err(e) => {
            tracing::error!(
                path = %asset.rel,
                error = %format!("{e:#}"),
                "persistent file failed its download or digest gate"
            );
            summary.files_failed.fetch_add(1, Ordering::Relaxed);
            if summary.fail_fast() {
                summary.abort();
            }
            AssetOutcome::Failed
        }
    }
}

fn row_of(asset: &PersistentAsset) -> PersistentRow {
    PersistentRow {
        // `remote_name` is not part of the gate; the rel is the stable label.
        remote_name: asset.rel.clone(),
        md5: asset.md5.clone(),
        hash: asset.hash,
        file_size: asset.size,
    }
}

fn target_of_checked(game_dir: &Path, asset: &PersistentAsset) -> anyhow::Result<PathBuf> {
    util::contained_join(game_dir, &asset.rel)
        .with_context(|| format!("resolve persistent rel {}", asset.rel))
        .map_err(Into::into)
}

// ---------------------------------------------------------------------------
// Cheap prerequisites (docs/03 §7.5.2 item 1, gap rows 9-11)
// ---------------------------------------------------------------------------

/// Create both base trees, drop stale/duplicate state and restore the
/// launcher's `ctable_streaming.dat` when the source is present and differs.
///
/// Every path is resolved through `contained_join`, so a crafted exe stem or a
/// symlink inside the install cannot turn a prerequisite into an escape.
fn prerequisites(
    ctx: &RunCtx,
    game_dir: &Path,
    exe_file_name: &str,
    state: &PersistentState,
) -> Result<(), RunFailure> {
    let data = data_dir_name(exe_file_name)
        .map_err(|e| RunFailure::write(anyhow!("persistent exe name: {e}")))?;
    let persistent_tree = format!("{data}/Persistent");
    let streaming_tree = format!("{data}/StreamingAssets");
    let persistent_dir = join(game_dir, &persistent_tree).map_err(RunFailure::write)?;
    let streaming_dir = join(game_dir, &streaming_tree).map_err(RunFailure::write)?;

    std::fs::create_dir_all(&persistent_dir)
        .with_context(|| format!("create {}", persistent_dir.display()))
        .map_err(RunFailure::write)?;
    std::fs::create_dir_all(&streaming_dir)
        .with_context(|| format!("create {}", streaming_dir.display()))
        .map_err(RunFailure::write)?;

    // A stale `Persistent/ctable.dat` is a launcher artifact, not a game file.
    let stale =
        join(game_dir, &format!("{persistent_tree}/ctable.dat")).map_err(RunFailure::write)?;
    if stale.exists() {
        clear_readonly(&stale);
        std::fs::remove_file(&stale)
            .with_context(|| format!("delete {}", stale.display()))
            .map_err(RunFailure::write)?;
    }
    // `DXSETUP/` produces false-positive integrity errors. It is a directory
    // on a real install; tolerate a file at that name as well.
    let dxsetup = join(game_dir, "DXSETUP").map_err(RunFailure::write)?;
    if dxsetup.is_dir() {
        std::fs::remove_dir_all(&dxsetup)
            .with_context(|| format!("delete {}", dxsetup.display()))
            .map_err(RunFailure::write)?;
    } else if dxsetup.exists() {
        clear_readonly(&dxsetup);
        std::fs::remove_file(&dxsetup)
            .with_context(|| format!("delete {}", dxsetup.display()))
            .map_err(RunFailure::write)?;
    }

    // `ctable_streaming.dat` is a verified Sophon base file. A missing source
    // (torrent/Sophon-only installs) is a no-op, and an equal target is never
    // rewritten with a stale source.
    let source =
        join(game_dir, &format!("{streaming_tree}/ctable.dat")).map_err(RunFailure::write)?;
    let target = join(game_dir, &format!("{streaming_tree}/ctable_streaming.dat"))
        .map_err(RunFailure::write)?;
    if source.is_file() {
        let bytes = std::fs::read(&source)
            .with_context(|| format!("read {}", source.display()))
            .map_err(RunFailure::write)?;
        let differs = match std::fs::read(&target) {
            Ok(existing) => existing != bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => true,
            Err(e) => {
                return Err(RunFailure::write(anyhow!("read {}: {e}", target.display())));
            }
        };
        if differs {
            write_atomic(&target, &bytes, false)
                .with_context(|| format!("copy ctable.dat to {}", target.display()))
                .map_err(RunFailure::write)?;
        }
    }

    move_non_patch_av(ctx, game_dir, &persistent_tree, &streaming_tree, state);
    Ok(())
}

/// Collapse `Check.cs:91-145`: a hotfix that ended leaves `.pck`/`.usm`/
/// `.cuepoint` copies stranded under `Persistent/`. Files the plan still
/// expects there are genuine hotfixes and stay; the rest move back to their
/// `StreamingAssets/` counterpart.
///
/// `plan.expected_rels` is the **unfiltered** expected set, so a hotfix hidden
/// by an active scope is still protected. A hidden source or destination is
/// left alone rather than silently rewritten.
fn move_non_patch_av(
    ctx: &RunCtx,
    game_dir: &Path,
    persistent_tree: &str,
    streaming_tree: &str,
    state: &PersistentState,
) {
    let root = match join(game_dir, persistent_tree) {
        Ok(root) => root,
        Err(e) => {
            tracing::warn!(error = %format!("{e:#}"), "cannot walk the persistent tree");
            return;
        }
    };
    for entry in WalkDir::new(&root).into_iter().filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let is_av = path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| {
                ext.eq_ignore_ascii_case("pck")
                    || ext.eq_ignore_ascii_case("usm")
                    || ext.eq_ignore_ascii_case("cuepoint")
            });
        if !is_av {
            continue;
        }
        let Ok(rel) = rel_from(game_dir, path) else {
            continue;
        };
        if !ctx.path_scope.is_visible(&rel) {
            continue;
        }
        if state.plan.expected_rels.iter().any(|r| r == &rel) {
            continue;
        }
        let Some(rest) = rel.strip_prefix(&format!("{persistent_tree}/")) else {
            continue;
        };
        let dest_rel = format!("{streaming_tree}/{rest}");
        if !ctx.path_scope.is_visible(&dest_rel) {
            continue;
        }
        let Ok(dest) = join(game_dir, &dest_rel) else {
            continue;
        };
        match move_file(path, &dest) {
            Ok(true) => tracing::info!(
                from = %rel,
                to = %dest_rel,
                "moved a non-patch persistent AV file back to StreamingAssets"
            ),
            Ok(false) => {}
            Err(e) => tracing::warn!(
                from = %rel,
                to = %dest_rel,
                error = %format!("{e:#}"),
                "could not move a non-patch persistent AV file"
            ),
        }
    }
}

/// Move `source` to `dest`, replacing a differing destination atomically.
/// `Ok(false)` means the two already carried the same bytes and only the
/// stranded source was removed.
fn move_file(source: &Path, dest: &Path) -> anyhow::Result<bool> {
    let bytes = std::fs::read(source).with_context(|| format!("read {}", source.display()))?;
    let equal = match std::fs::read(dest) {
        Ok(existing) => existing == bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(_) => false,
    };
    if !equal {
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create {}", parent.display()))?;
        }
        write_atomic(dest, &bytes, false).with_context(|| format!("write {}", dest.display()))?;
    }
    std::fs::remove_file(source).with_context(|| format!("remove {}", source.display()))?;
    Ok(!equal)
}

/// Clear the read-only bit on this one prerequisite before deleting it.
/// Tree-wide stripping is never done; only the file about to be removed is
/// touched (C17(b)'s per-path discipline, applied to the stale launcher state).
fn clear_readonly(path: &Path) {
    let Ok(meta) = std::fs::metadata(path) else {
        return;
    };
    let mut permissions = meta.permissions();
    if permissions.readonly() {
        permissions.set_readonly(false);
        if let Err(e) = std::fs::set_permissions(path, permissions) {
            tracing::warn!(
                path = %path.display(),
                error = %e,
                "could not clear read-only before deleting a stale persistent prerequisite"
            );
        }
    }
}

fn join(game_dir: &Path, rel: &str) -> anyhow::Result<PathBuf> {
    util::contained_join(game_dir, rel).with_context(|| format!("resolve {rel}"))
}

fn rel_from(game_dir: &Path, path: &Path) -> anyhow::Result<String> {
    let rel = path.strip_prefix(game_dir)?;
    let rel = rel.to_string_lossy().replace('\\', "/");
    util::clean_rel(&rel)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::retry::BackoffPolicy;
    use girpr_dispatch::ManifestKind;
    use girpr_loopback_mock::{Reply, ScriptedServer};
    use girpr_persistent_plan::BaseTree;
    use girpr_tempdir::TempDir;

    const HOTFIX_A: &[u8] =
        include_bytes!("../../tests/fixtures/persistent_inputs/samples/fixtures/hotfix_a.blk");

    fn asset(server: &ScriptedServer) -> PersistentAsset {
        let base = server.base();
        PersistentAsset {
            rel: "GenshinImpact_Data/StreamingAssets/AssetBundles/blocks/00/a.blk".into(),
            placement: BaseTree::StreamingAssets,
            size: 64,
            md5: "b2d3f56bc197fd985d5965079b5e7148".into(),
            hash: Some(0xf7c6_7301_db67_13f0),
            primary_url: format!("{base}/primary"),
            alternate_url: format!("{base}/alternate"),
            source: ManifestKind::Res,
        }
    }

    /// A live ceiling is charged before the request, so a zero budget refuses
    /// the asset without even opening a socket and trips the hard-stop marker.
    #[tokio::test]
    async fn a_live_download_ceiling_stops_before_the_hotfix_request() {
        let server = ScriptedServer::start(vec![Reply::new(200, HOTFIX_A)]);
        let dir = TempDir::new("girpr_persistent_exec", "ceiling");
        let a = asset(&server);
        let mut plain = Summary::default();
        plain.set_download_limit(Some(0));
        let summary = Arc::new(plain);

        let outcome = download_one(
            &BackoffPolicy::none(),
            &Client::new(),
            dir.as_ref(),
            &a,
            &summary,
        )
        .await;

        assert_eq!(outcome, AssetOutcome::Stopped);
        assert_eq!(summary.stopped_by(), StoppedBy::Download);
        assert_eq!(summary.download.live(), 0, "a refused charge adds nothing");
        assert_eq!(server.hits(), 0, "no request followed a refused charge");
        assert!(!target_of_checked(dir.as_ref(), &a).unwrap().exists());
    }

    /// A verified hotfix feeds both meters exactly once: settled bytes are the
    /// bytes of the published file, and the file counters move with them.
    #[tokio::test]
    async fn a_verified_hotfix_charges_both_meters_once() {
        let server = ScriptedServer::start(vec![Reply::new(200, HOTFIX_A)]);
        let dir = TempDir::new("girpr_persistent_exec", "meters");
        let a = asset(&server);
        let summary = Arc::new(Summary::default());

        let outcome = download_one(
            &BackoffPolicy::none(),
            &Client::new(),
            dir.as_ref(),
            &a,
            &summary,
        )
        .await;

        assert_eq!(outcome, AssetOutcome::Repaired);
        assert_eq!(server.hits(), 1);
        assert_eq!(summary.download.live(), 64);
        assert_eq!(summary.download.settled(), 64);
        assert_eq!(summary.write.live(), 64);
        assert_eq!(summary.write.settled(), 64);
        assert_eq!(summary.files_repaired.load(Ordering::Relaxed), 1);
        assert_eq!(summary.files_failed.load(Ordering::Relaxed), 0);
    }

    /// The write ceiling is charged inside the downloader, before publish: a
    /// zero budget stops with exit 5's marker and leaves no hotfix on disk.
    #[tokio::test]
    async fn a_zero_write_ceiling_stops_before_the_hotfix_is_published() {
        let server = ScriptedServer::start(vec![Reply::new(200, HOTFIX_A)]);
        let dir = TempDir::new("girpr_persistent_exec", "write_ceiling");
        let a = asset(&server);
        let mut plain = Summary::default();
        plain.set_write_limit(Some(0));
        let summary = Arc::new(plain);

        let outcome = download_one(
            &BackoffPolicy::none(),
            &Client::new(),
            dir.as_ref(),
            &a,
            &summary,
        )
        .await;

        assert_eq!(outcome, AssetOutcome::Stopped);
        assert_eq!(summary.stopped_by(), StoppedBy::Write);
        assert_eq!(summary.write.live(), 0, "a refused charge adds nothing");
        assert_eq!(
            summary.files_failed.load(Ordering::Relaxed),
            0,
            "a stop, not a failure"
        );
        assert!(
            !target_of_checked(dir.as_ref(), &a).unwrap().exists(),
            "no bytes were published"
        );
    }

    /// The settled variant uses the same pre-publish seam and the same typed
    /// stop, but names the other ceiling in `stopped=`.
    #[tokio::test]
    async fn a_zero_verified_write_ceiling_stops_before_the_hotfix_is_published() {
        let server = ScriptedServer::start(vec![Reply::new(200, HOTFIX_A)]);
        let dir = TempDir::new("girpr_persistent_exec", "verified_write_ceiling");
        let a = asset(&server);
        let mut plain = Summary::default();
        plain.set_verified_write_limit(Some(0));
        let summary = Arc::new(plain);

        let outcome = download_one(
            &BackoffPolicy::none(),
            &Client::new(),
            dir.as_ref(),
            &a,
            &summary,
        )
        .await;

        assert_eq!(outcome, AssetOutcome::Stopped);
        assert_eq!(summary.stopped_by(), StoppedBy::VerifiedWrite);
        // `settle` counts what was charged even when it crossed the ceiling,
        // so the meter still reports 64 — but no bytes were published.
        assert_eq!(summary.write.settled(), 64);
        assert_eq!(summary.files_failed.load(Ordering::Relaxed), 0);
        assert!(!target_of_checked(dir.as_ref(), &a).unwrap().exists());
    }

    /// At the boundary the charge succeeds exactly once and the file lands.
    #[tokio::test]
    async fn a_write_ceiling_at_the_boundary_charges_once() {
        let server = ScriptedServer::start(vec![Reply::new(200, HOTFIX_A)]);
        let dir = TempDir::new("girpr_persistent_exec", "write_boundary");
        let a = asset(&server);
        let mut plain = Summary::default();
        plain.set_write_limit(Some(64));
        let summary = Arc::new(plain);

        let outcome = download_one(
            &BackoffPolicy::none(),
            &Client::new(),
            dir.as_ref(),
            &a,
            &summary,
        )
        .await;

        assert_eq!(outcome, AssetOutcome::Repaired);
        assert_eq!(summary.stopped_by(), StoppedBy::None);
        assert_eq!(summary.write.live(), 64);
        assert_eq!(summary.write.settled(), 64);
        assert_eq!(summary.files_repaired.load(Ordering::Relaxed), 1);
        assert_eq!(summary.files_failed.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn a_row_uses_the_asset_size_and_digest_and_never_the_url() {
        let asset = PersistentAsset {
            rel: "GenshinImpact_Data/Persistent/x.bin".into(),
            placement: girpr_persistent_plan::BaseTree::Persistent,
            size: 64,
            md5: "b2d3f56bc197fd985d5965079b5e7148".into(),
            hash: Some(0xf7c6_7301_db67_13f0),
            primary_url: "http://primary/x".into(),
            alternate_url: "http://alternate/x".into(),
            source: girpr_dispatch::ManifestKind::Res,
        };
        let row = row_of(&asset);
        assert_eq!(row.file_size, 64);
        assert_eq!(row.hash, Some(0xf7c6_7301_db67_13f0));
        assert_eq!(row.md5, asset.md5);
    }
}
