//! Repair pipeline: metadata → plan → optional purge → per-file chunk repair →
//! post-phase. See `docs/02-repair-chunk-spec.md` for the numbered spec steps
//! referenced in the comments below.
//!
//! Submodule map:
//! - [`check`] — `--check-only` verification pass (size + MD5, writes nothing)
//! - [`file`] — per-file chunk repair + the bounded file-parallelism driver
//! - [`purge`] — Collapse-parity files-cleanup (`--purge-before`/`--purge-after`)
//! - [`audio`] — audio-scan read/write and the cache→res move

mod audio;
mod check;
mod file;
mod purge;

use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use crate::Biz;
use crate::error::RunFailure;
use crate::hyp::HypClient;
use crate::plan::build_plan;
use crate::report::{Summary, format_report_line, start_progress_reporter};
use crate::sophon::{self, SophonChunkFile, WantedManifest};
use crate::util;

use audio::{move_audio_cache, read_current_audio, write_audio_scan};
use check::run_check_only;
use file::repair_all_files;
use purge::{collapse_keep_set, collapse_purge_extra};

pub struct RepairCtx {
    pub game_dir: PathBuf,
    pub biz: Biz,
    pub audio: HashSet<String>,
    pub jobs: usize,
    pub check_only: bool,
    pub dry_run: bool,
    pub purge_after: bool,
    pub purge_before: bool,
    pub json_summary: bool,
}

pub async fn run(ctx: RepairCtx) -> Result<(Summary, i32), RunFailure> {
    let mut summary = Summary::default();
    // Read-only modes must not write anything: `--dry-run` logs actions only,
    // `--check-only` verifies only. Every delete/write below is gated on this
    // (previously `--check-only` still swept temps and `--purge-before` still
    // deleted — both fixed by gating on `readonly`, not just `dry_run`).
    let readonly = ctx.dry_run || ctx.check_only;
    if !readonly {
        std::fs::create_dir_all(&ctx.game_dir)
            .context("create game dir")
            .map_err(RunFailure::usage)?;
    }
    let game_dir = ctx
        .game_dir
        .canonicalize()
        .unwrap_or_else(|_| ctx.game_dir.clone());

    // NOTE: no unconditional temp sweep. All cleanup is the single
    // Collapse-style files-cleanup (`collapse_purge_extra`), run before
    // and/or after patching only when the corresponding flag is set.
    // `--purge-before` and `--purge-after` delete the same things; only the
    // timing differs.

    // 1. local state
    let local_version = util::read_game_version(&game_dir);
    tracing::info!("local game_version: {:?}", local_version);

    // 2. server metadata (up to 5 calls)
    let hyp = HypClient::new(ctx.biz).map_err(RunFailure::metadata)?;
    let cfg = hyp
        .game_config()
        .await
        .context("getGameConfigs")
        .map_err(RunFailure::metadata)?;
    tracing::info!(
        "game config: exe={} audio_scan={} audio_res={} audio_cache={} mode={}",
        cfg.exe_file_name,
        cfg.audio_pkg_scan_dir,
        cfg.audio_pkg_res_dir,
        cfg.audio_pkg_cache_dir,
        cfg.default_download_mode
    );
    if cfg.default_download_mode == "DOWNLOAD_MODE_FILE" {
        return Err(RunFailure::usage(anyhow::anyhow!(
            "game is in legacy FILE mode; chunk repair not supported (unexpected for Genshin)"
        )));
    }
    let branch = hyp
        .game_branch()
        .await
        .context("getGameBranches")
        .map_err(RunFailure::metadata)?;
    let latest = branch.main.tag.clone();
    tracing::info!("latest version: {}", latest);
    let latest_build = hyp
        .chunk_build(&branch.main, None)
        .await
        .context("getBuild(latest)")
        .map_err(RunFailure::metadata)?;

    let mut local_build = None;
    if let Some(lv) = local_version.as_deref() {
        match hyp.chunk_build(&branch.main, Some(lv)).await {
            Ok(b) => {
                tracing::info!("local build manifest loaded for dedup (tag={})", lv);
                local_build = Some(b);
            }
            Err(e) => tracing::warn!("local build unavailable ({}); full-fetch fallback", e),
        }
    }

    let ignore = if cfg.res_category_dir.is_empty() {
        HashSet::new()
    } else {
        util::read_ignore_categories(&game_dir.join(util::normalize_rel(&cfg.res_category_dir)))
    };
    let blacklist = if cfg.enable_resource_blacklist && !cfg.blacklist_dir.is_empty() {
        util::read_blacklist(&game_dir.join(util::normalize_rel(&cfg.blacklist_dir)))
    } else {
        HashSet::new()
    };

    // effective audio langs
    let mut audio = ctx.audio.clone();
    if audio.is_empty() {
        audio = read_current_audio(&game_dir, &cfg.audio_pkg_scan_dir);
        if audio.is_empty() {
            audio.insert("en-us".to_string());
        }
        tracing::info!("keeping current audio langs: {:?}", audio);
    } else if !ctx.dry_run && !ctx.check_only && !cfg.audio_pkg_scan_dir.is_empty() {
        write_audio_scan(&game_dir, &cfg.audio_pkg_scan_dir, &audio)
            .context("write audio scan file")
            .map_err(RunFailure::write)?;
    }

    // 2.5 begin-report to stdout (mirrored to log): versions + API-sourced
    // metadata so a stdout parser sees what is on disk and what it will become
    // before any repair starts. JSON when `--json-summary`.
    // See https://github.com/Scighost/Starward/blob/3e2da5ffecde252211edb74b850ee13d6b93f6dd/src/Starward.RPC/GameInstall/GameInstallService.cs#L788-L849
    let report_line = format_report_line(
        &local_version,
        &latest,
        ctx.biz.as_str(),
        &cfg,
        &branch.main,
        &latest_build,
        &audio,
        ctx.json_summary,
    );
    println!("{report_line}");
    tracing::info!("{report_line}");

    // 3. fetch + verify + parse latest manifests (per-manifest lists kept for prefix mapping)
    let http = hyp.http();
    let wanted = sophon::select_manifests(&latest_build, &audio, &ignore);
    let mut per_manifest: Vec<(String, Vec<SophonChunkFile>)> = Vec::new();
    for m in wanted {
        let files = sophon::fetch_manifest(&http, m)
            .await
            .with_context(|| format!("manifest {}", m.matching_field))
            .map_err(RunFailure::metadata)?;
        tracing::info!("manifest {}: {} entries", m.matching_field, files.len());
        per_manifest.push((m.chunk_download.url_prefix.clone(), files));
    }

    // local manifests for chunk-dedup (best effort)
    let mut local_map: HashMap<String, Vec<(String, i64, i64)>> = HashMap::new();
    if let Some(lb) = local_build.as_ref() {
        let local_wanted = sophon::select_manifests(lb, &audio, &ignore);
        let mut wm: Vec<WantedManifest> = Vec::new();
        for m in local_wanted {
            match sophon::fetch_manifest(&http, m).await {
                Ok(files) => wm.push(WantedManifest {
                    meta: m.clone(),
                    files,
                }),
                Err(e) => tracing::warn!("local manifest {} failed: {}", m.matching_field, e),
            }
        }
        local_map = sophon::build_local_chunk_map(&wm);
    }

    let plan = build_plan(per_manifest, &blacklist, &latest);
    tracing::info!("work list: {} files", plan.files.len());

    // Purge-before: same Collapse-style files-cleanup as purge-after,
    // run here to free space for the repair itself.
    // Skipped entirely in `--check-only` (verification reports damage; it must
    // not delete). In `--dry-run` the purge runs in log-only mode.
    // See docs/02 Step 5 and
    // https://github.com/CollapseLauncher/Collapse/blob/dc47259171794596331dffcf90db85a6ac0415ac/CollapseLauncher/Classes/InstallManagement/Genshin/GenshinInstall.cs#L177-L263
    if ctx.purge_before && !ctx.check_only {
        let server_keep = collapse_keep_set();
        let bytes = collapse_purge_extra(&game_dir, &plan, &server_keep, ctx.dry_run)
            .map_err(RunFailure::write)?;
        summary
            .deleted_extra_bytes
            .fetch_add(bytes, Ordering::Relaxed);
    }

    // 4. check-only mode
    if ctx.check_only {
        let code = run_check_only(&game_dir, &plan, &mut summary);
        return Ok((summary, code));
    }

    // 5. repair files (bounded file parallelism, sequential chunks per file).
    // See `file::repair_all_files` for the span/counter contract.
    summary.files_total = plan.files.len() as u64;
    let plan_a = Arc::new(plan);
    let sum_a = Arc::new(summary);
    let progress = start_progress_reporter(sum_a.clone());
    repair_all_files(
        http,
        game_dir.clone(),
        plan_a.clone(),
        local_map,
        sum_a.clone(),
        ctx.jobs,
        ctx.dry_run,
    )
    .await;
    progress.abort();
    let summary = Arc::try_unwrap(sum_a).unwrap_or_else(|a| Summary {
        files_total: a.files_total,
        files_skipped: AtomicU64::new(a.files_skipped.load(Ordering::Relaxed)),
        files_repaired: AtomicU64::new(a.files_repaired.load(Ordering::Relaxed)),
        files_failed: AtomicU64::new(a.files_failed.load(Ordering::Relaxed)),
        download_bytes: AtomicU64::new(a.download_bytes.load(Ordering::Relaxed)),
        deleted_extra_bytes: AtomicU64::new(a.deleted_extra_bytes.load(Ordering::Relaxed)),
    });
    if summary.files_failed.load(Ordering::Relaxed) > 0 {
        return Ok((summary, 3));
    }

    // 6. post phase (see docs/02 Step 7).
    // `readonly` covers both `--dry-run` and `--check-only` (check-only returns
    // before this point, but gating on `readonly` keeps the invariant obvious).
    if !readonly {
        // Starward-handling remainder: deprecated list + audio cache→res move
        // + config.ini bump. Deprecated files are also covered by the purge
        // below when enabled (they are not in the live manifest); the explicit
        // loop keeps them cleaned even without any purge flag.
        match hyp.deprecated_files().await {
            Ok(list) => {
                for n in list {
                    let p = game_dir.join(util::normalize_rel(&n));
                    if p.is_file() {
                        let sz = util::file_len(&p).unwrap_or(0);
                        if std::fs::remove_file(&p).is_ok() {
                            summary.deleted_extra_bytes.fetch_add(sz, Ordering::Relaxed);
                            tracing::info!("deleted deprecated {}", n);
                        }
                    }
                }
            }
            Err(e) => tracing::warn!("deprecated list unavailable: {:#}", e),
        }
        if !cfg.audio_pkg_cache_dir.is_empty()
            && !cfg.audio_pkg_res_dir.is_empty()
            && cfg.audio_pkg_cache_dir != cfg.audio_pkg_res_dir
        {
            move_audio_cache(&game_dir, &cfg.audio_pkg_cache_dir, &cfg.audio_pkg_res_dir);
        }
        // Purge-after: same files-cleanup as purge-before, run after patching.
        if ctx.purge_after {
            let server_keep = collapse_keep_set();
            let bytes = collapse_purge_extra(&game_dir, &plan_a, &server_keep, false)
                .map_err(RunFailure::write)?;
            summary
                .deleted_extra_bytes
                .fetch_add(bytes, Ordering::Relaxed);
        }
        let (ch, sub, cps) = ctx.biz.channel_tuple();
        util::write_config_ini(
            &game_dir,
            &latest,
            ctx.biz.as_str(),
            (ch, sub, cps),
            "",
            false,
        )
        .context("write config.ini")
        .map_err(RunFailure::write)?;
    } else if ctx.dry_run && ctx.purge_after {
        // Dry-run logging only: list what the files-cleanup would
        // delete (writes nothing; byte count stays 0 by design).
        let server_keep = collapse_keep_set();
        let plan_ref = &*plan_a;
        let _ = collapse_purge_extra(&game_dir, plan_ref, &server_keep, true)
            .map_err(RunFailure::write)?;
    }

    Ok((summary, 0))
}
