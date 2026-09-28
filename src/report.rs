//! The run's counters and every stdout line the automation contract promises.
//!
//! One rule lives here: data-plane lines go to stdout *and* the log, so a
//! caller that captures either stream sees the same `REPORT` / `PROGRESS` /
//! `SUMMARY` sequence. Formatting is separated from emission so both shapes
//! (`key=value` vs JSON) can be unit-tested without a subscriber installed.

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::hyp::{ChunkBuild, GameBranchPackage, GameConfig};

/// How often the background reporter emits `PROGRESS` during long phases.
pub const PROGRESS_INTERVAL_SECS: u64 = 10;

/// Run counters. The atomic fields are shared with the per-file tasks, hence
/// [`Summary::snapshot`] to read them back out once the tasks are joined.
#[derive(Default, Debug)]
pub struct Summary {
    pub files_total: u64,
    pub files_skipped: AtomicU64,
    pub files_repaired: AtomicU64,
    pub files_failed: AtomicU64,
    pub download_bytes: AtomicU64,
    /// Unified cleanup counter (Collapse files-cleanup parity): every byte
    /// deleted by `--purge-before` / `--purge-after` (temps, orphans,
    /// deprecated files) lands here. No split accounting.
    pub deleted_extra_bytes: AtomicU64,
}

impl Summary {
    /// Plain-value copy, for handing a shared `Summary` back to the caller.
    pub fn snapshot(&self) -> Self {
        Self {
            files_total: self.files_total,
            files_skipped: AtomicU64::new(self.files_skipped.load(Ordering::Relaxed)),
            files_repaired: AtomicU64::new(self.files_repaired.load(Ordering::Relaxed)),
            files_failed: AtomicU64::new(self.files_failed.load(Ordering::Relaxed)),
            download_bytes: AtomicU64::new(self.download_bytes.load(Ordering::Relaxed)),
            deleted_extra_bytes: AtomicU64::new(
                self.deleted_extra_bytes.load(Ordering::Relaxed),
            ),
        }
    }
}

/// Emit one data-plane line to **both** stdout and the log so unattended runs
/// keep it regardless of which stream is captured.
/// NOTE(design-intent): the dual emission is deliberate (automation contract,
/// see README).
pub fn emit(line: &str) {
    println!("{line}");
    tracing::info!("{line}");
}

/// Format the one-line progress snapshot.
pub fn format_progress(sum: &Summary) -> String {
    let skipped = sum.files_skipped.load(Ordering::Relaxed);
    let repaired = sum.files_repaired.load(Ordering::Relaxed);
    let failed = sum.files_failed.load(Ordering::Relaxed);
    let done = skipped + repaired + failed;
    let dl = sum.download_bytes.load(Ordering::Relaxed);
    format!(
        "PROGRESS done={}/{} skipped={} repaired={} failed={} download_bytes={}",
        done, sum.files_total, skipped, repaired, failed, dl
    )
}

/// Emit the current progress snapshot.
pub fn emit_progress(sum: &Summary) {
    emit(&format_progress(sum));
}

/// Background `PROGRESS` ticker for the repair phase. The first tick fires
/// immediately; it is skipped so we only report after a full interval.
pub fn spawn_progress_reporter(sum: std::sync::Arc<Summary>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(std::time::Duration::from_secs(PROGRESS_INTERVAL_SECS));
        interval.tick().await;
        loop {
            interval.tick().await;
            emit_progress(&sum);
        }
    })
}

/// The one-time begin-report written right after the metadata calls (mirrored to
/// the log). Parsers get both versions before any repair starts:
/// `local_version` is the on-disk `config.ini` value (`none` when absent),
/// `latest_version` is the "will be updated to" tag; the rest come from the
/// HoYoPlay/Sophon APIs, not from config. `audio_langs` is the effective set.
/// Emitted as a `REPORT key=value` line, or as a JSON object with
/// `--json-summary`.
///
/// Provenance: Starward `GameInstallService.cs:788-849` (the same field set the
/// launcher shows the user before installing).
#[allow(clippy::too_many_arguments)] // stable report formatter; params-struct would churn CLI parsing
pub fn format_report_line(
    local_version: &Option<String>,
    latest_version: &str,
    biz: &str,
    cfg: &GameConfig,
    pkg: &GameBranchPackage,
    build: &ChunkBuild,
    audio_langs: &HashSet<String>,
    json: bool,
) -> String {
    let mut langs: Vec<&String> = audio_langs.iter().collect();
    langs.sort();
    let audio_joined: Vec<&str> = langs.iter().map(|s| s.as_str()).collect();
    let audio = audio_joined.join(",");
    let local = local_version.as_deref().unwrap_or("none");
    let diff_tags = pkg.diff_tags.join(",");
    if json {
        serde_json::json!({
            "local_version": local,
            "latest_version": latest_version,
            "biz": biz,
            "exe": cfg.exe_file_name,
            "download_mode": cfg.default_download_mode,
            "branch": pkg.branch,
            "package_id": pkg.package_id,
            "build_id": build.build_id,
            "audio_langs": audio,
            "diff_tags": diff_tags,
        })
        .to_string()
    } else {
        format!(
            "REPORT local_version={local} latest_version={latest_version} biz={biz} exe={} download_mode={} branch={} package_id={} build_id={} audio_langs={audio} diff_tags={diff_tags}",
            cfg.exe_file_name, cfg.default_download_mode, pkg.branch, pkg.package_id, build.build_id
        )
    }
}

/// The final `SUMMARY` line, or the same fields as a JSON object with
/// `--json-summary`.
pub fn format_summary_line(sum: &Summary, exit: i32, json: bool) -> String {
    let skipped = sum.files_skipped.load(Ordering::Relaxed);
    let repaired = sum.files_repaired.load(Ordering::Relaxed);
    let failed = sum.files_failed.load(Ordering::Relaxed);
    let dl = sum.download_bytes.load(Ordering::Relaxed);
    let del = sum.deleted_extra_bytes.load(Ordering::Relaxed);
    if json {
        format!(
            "{{\"total\":{},\"skipped\":{},\"repaired\":{},\"failed\":{},\"download_bytes\":{},\"deleted_extra_bytes\":{},\"exit\":{}}}",
            sum.files_total, skipped, repaired, failed, dl, del, exit
        )
    } else {
        format!(
            "SUMMARY total={} skipped={} repaired={} failed={} download_bytes={} deleted_extra_bytes={} exit={exit}",
            sum.files_total, skipped, repaired, failed, dl, del
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_format_done_counts() {
        let s = Summary {
            files_total: 10,
            files_skipped: AtomicU64::new(3),
            files_repaired: AtomicU64::new(2),
            files_failed: AtomicU64::new(1),
            download_bytes: AtomicU64::new(123),
            ..Summary::default()
        };
        assert_eq!(
            format_progress(&s),
            "PROGRESS done=6/10 skipped=3 repaired=2 failed=1 download_bytes=123"
        );
    }

    #[test]
    fn summary_line_keyvalue_and_json_agree() {
        let s = Summary {
            files_total: 4,
            files_skipped: AtomicU64::new(1),
            files_repaired: AtomicU64::new(2),
            files_failed: AtomicU64::new(1),
            download_bytes: AtomicU64::new(10),
            deleted_extra_bytes: AtomicU64::new(20),
        };
        assert_eq!(
            format_summary_line(&s, 3, false),
            "SUMMARY total=4 skipped=1 repaired=2 failed=1 download_bytes=10 deleted_extra_bytes=20 exit=3"
        );
        let json = format_summary_line(&s, 3, true);
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["total"], 4);
        assert_eq!(v["skipped"], 1);
        assert_eq!(v["repaired"], 2);
        assert_eq!(v["failed"], 1);
        assert_eq!(v["download_bytes"], 10);
        assert_eq!(v["deleted_extra_bytes"], 20);
        assert_eq!(v["exit"], 3);
    }

    #[test]
    fn snapshot_copies_every_counter() {
        let s = Summary {
            files_total: 7,
            files_skipped: AtomicU64::new(1),
            download_bytes: AtomicU64::new(9),
            ..Summary::default()
        };
        let snap = s.snapshot();
        assert_eq!(snap.files_total, 7);
        assert_eq!(snap.files_skipped.load(Ordering::Relaxed), 1);
        assert_eq!(snap.download_bytes.load(Ordering::Relaxed), 9);
        // Mutating the source afterwards must not touch the snapshot.
        s.files_skipped.store(5, Ordering::Relaxed);
        assert_eq!(snap.files_skipped.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn report_line_keyvalue_reports_versions_and_api_fields() {
        let cfg = GameConfig {
            exe_file_name: "YuanShen.exe".into(),
            audio_pkg_scan_dir: "scan.txt".into(),
            audio_pkg_res_dir: "res".into(),
            audio_pkg_cache_dir: "cache".into(),
            default_download_mode: "DOWNLOAD_MODE_CHUNK".into(),
            res_category_dir: "rc.txt".into(),
            blacklist_dir: "bl.txt".into(),
            enable_resource_blacklist: true,
            game: None,
        };
        let pkg = GameBranchPackage {
            package_id: "pkg1".into(),
            branch: "main".into(),
            password: String::new(),
            tag: "5.1.0".into(),
            diff_tags: vec!["5.0.0".into(), "5.1.0".into()],
        };
        let build = ChunkBuild {
            build_id: "build42".into(),
            tag: "5.1.0".into(),
            manifests: Vec::new(),
        };
        let audio: HashSet<String> = ["ja-jp".into(), "en-us".into()].into_iter().collect();
        let line = format_report_line(
            &Some("4.0.0".to_string()),
            "5.1.0",
            "hk4e_global",
            &cfg,
            &pkg,
            &build,
            &audio,
            false,
        );
        assert!(
            line.starts_with("REPORT local_version=4.0.0 latest_version=5.1.0 biz=hk4e_global"),
            "{line}"
        );
        assert!(line.contains("exe=YuanShen.exe"), "{line}");
        assert!(line.contains("download_mode=DOWNLOAD_MODE_CHUNK"), "{line}");
        assert!(line.contains("branch=main"), "{line}");
        assert!(line.contains("package_id=pkg1"), "{line}");
        assert!(line.contains("build_id=build42"), "{line}");
        assert!(line.contains("audio_langs=en-us,ja-jp"), "{line}");
        assert!(line.contains("diff_tags=5.0.0,5.1.0"), "{line}");
    }

    #[test]
    fn report_line_none_local_and_sorted_json() {
        let cfg = GameConfig {
            exe_file_name: "GenshinImpact.exe".into(),
            audio_pkg_scan_dir: String::new(),
            audio_pkg_res_dir: String::new(),
            audio_pkg_cache_dir: String::new(),
            default_download_mode: String::new(),
            res_category_dir: String::new(),
            blacklist_dir: String::new(),
            enable_resource_blacklist: false,
            game: None,
        };
        let pkg = GameBranchPackage::default();
        let build = ChunkBuild {
            build_id: "b1".into(),
            tag: "3.0.0".into(),
            manifests: Vec::new(),
        };
        let audio: HashSet<String> = HashSet::new();
        let line = format_report_line(&None, "3.0.0", "hk4e_cn", &cfg, &pkg, &build, &audio, true);
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["local_version"], "none");
        assert_eq!(v["latest_version"], "3.0.0");
        assert_eq!(v["biz"], "hk4e_cn");
        assert_eq!(v["exe"], "GenshinImpact.exe");
        assert_eq!(v["audio_langs"], "");
    }
}
