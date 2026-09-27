use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::hyp::{ChunkBuild, GameBranchPackage, GameConfig};

#[derive(Default)]
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

/// How often the background reporter emits `PROGRESS` during long phases.
pub const PROGRESS_INTERVAL_SECS: u64 = 10;

/// Format the one-line progress snapshot. Shared by stdout + log emission.
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

/// Emit progress to **both** stdout and the log so unattended runs keep it
/// regardless of which stream is captured.
/// NOTE(design-intent): the dual emission is deliberate (automation contract,
/// see README).
pub fn emit_progress(sum: &Summary) {
    let line = format_progress(sum);
    println!("{line}");
    tracing::info!("{line}");
}

/// The one-time begin-report written to stdout right after the metadata calls
/// (mirrored to the log). Parsers get both versions before any repair starts:
/// `local_version` is the on-disk `config.ini` value (`none` when absent),
/// `latest_version` is the "will be updated to" tag; the rest come from the
/// HoYoPlay/Sophon APIs, not from config. `audio_langs` is the effective set.
/// Emitted as a `REPORT key=value` line, or as a JSON object with
/// `--json-summary`.
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

/// Background `PROGRESS` ticker for the repair phase; abort the handle to stop.
pub(crate) fn start_progress_reporter(sum: std::sync::Arc<Summary>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(std::time::Duration::from_secs(PROGRESS_INTERVAL_SECS));
        // First tick fires immediately; skip it so we only report after a full interval.
        interval.tick().await;
        loop {
            interval.tick().await;
            emit_progress(&sum);
        }
    })
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
    fn report_line_keyvalue_reports_versions_and_api_fields() {
        use crate::hyp::{ChunkBuild, GameBranchPackage, GameConfig};
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
        use crate::hyp::{ChunkBuild, GameBranchPackage, GameConfig};
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
