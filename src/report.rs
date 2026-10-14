//! The run's counters and every stdout line the automation contract promises.
//!
//! One rule lives here: data-plane lines go to stdout *and* the log, so a
//! caller that captures either stream sees the same `REPORT` / `PROGRESS` /
//! `SUMMARY` sequence. Formatting is separated from emission so both shapes
//! (`key=value` vs JSON) can be unit-tested without a subscriber installed.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::hyp::{ChunkBuild, GameBranchPackage, GameConfig};

/// How often the background reporter emits `PROGRESS` during long phases.
pub const PROGRESS_INTERVAL_SECS: u64 = 10;

/// Run counters. The atomic fields are shared with the per-file tasks, hence
/// [`Summary::snapshot`] to read them back out once the tasks are joined.
///
/// `Default` is written by hand rather than derived for exactly one reason: the
/// ceiling sentinel is `u64::MAX`, and a derived `Default` would start every run
/// at a ceiling of **zero** — i.e. refusing the first byte.
#[derive(Debug)]
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
    /// Conditional keys — see [`SummaryFlags`]. Set once, before the tasks
    /// start; plain values, so no atomics are needed.
    pub flags: SummaryFlags,
    /// `--max-download-bytes`, stored as a raw ceiling (`u64::MAX` = none) so
    /// the per-chunk reservation is a compare-and-swap with no `Option` check
    /// on the hot path. Owned by
    /// [`crate::repair::scope::ledger`](crate::repair::scope) — see
    /// [`Summary::set_download_limit`].
    pub download_limit: AtomicU64,
    /// Set when the runtime download hard stop fired. An aborted run leaves
    /// `*_tmp` files behind by design: S3 resume-by-length makes the next run a
    /// smaller run, not a corrupt one.
    pub download_stopped: AtomicBool,
}

impl Default for Summary {
    fn default() -> Self {
        Self::new()
    }
}

impl Summary {
    /// A run with no counters moved and no ceiling requested.
    pub fn new() -> Self {
        Self {
            files_total: 0,
            files_skipped: AtomicU64::new(0),
            files_repaired: AtomicU64::new(0),
            files_failed: AtomicU64::new(0),
            download_bytes: AtomicU64::new(0),
            deleted_extra_bytes: AtomicU64::new(0),
            flags: SummaryFlags::default(),
            download_limit: AtomicU64::new(u64::MAX),
            download_stopped: AtomicBool::new(false),
        }
    }

    /// Plain-value copy, for handing a shared `Summary` back to the caller.
    pub fn snapshot(&self) -> Self {
        Self {
            files_total: self.files_total,
            files_skipped: AtomicU64::new(self.files_skipped.load(Ordering::Relaxed)),
            files_repaired: AtomicU64::new(self.files_repaired.load(Ordering::Relaxed)),
            files_failed: AtomicU64::new(self.files_failed.load(Ordering::Relaxed)),
            download_bytes: AtomicU64::new(self.download_bytes.load(Ordering::Relaxed)),
            deleted_extra_bytes: AtomicU64::new(self.deleted_extra_bytes.load(Ordering::Relaxed)),
            flags: self.flags,
            // A snapshot must not lose the ceiling or the stop flag: the caller
            // reads both *after* the tasks have been joined, which is the only
            // moment either can be meaningful.
            download_limit: AtomicU64::new(self.download_limit.load(Ordering::Relaxed)),
            download_stopped: AtomicBool::new(self.download_stopped.load(Ordering::Relaxed)),
        }
    }
}

/// The **conditional** `SUMMARY` keys (docs/04 §4.6).
///
/// Every one is emitted *only* when its flag is active — the same discipline
/// `girsync` used for `identical=N`: an unflagged run's output is byte-identical
/// to a run that never passes the flags at all. That is what keeps the machine
/// contract non-breaking when this surface grows.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SummaryFlags {
    /// Any resource ceiling active (`--max-files`, `--max-download-bytes`, …).
    pub limited: bool,
    /// Any path scope active (`--include` / `--exclude`).
    pub scoped: bool,
    /// `--max-download-bytes`'s estimate for the (filtered) plan.
    pub plan_download_bytes: Option<u64>,
    /// Bytes written into `*_tmp`, when `--max-write-bytes` is active.
    pub write_bytes: Option<u64>,
    /// What a **runtime** hard stop cut short (`files` / `bytes` /
    /// `delete`), vs a pre-flight refusal which wrote nothing at all.
    pub stopped: Option<&'static str>,
}

/// One conditional `SUMMARY` entry. Rendered as `key=value` on the `key=value`
/// plane and as a typed JSON member on the `--json-summary` plane, so the two
/// stay in agreement without a hand-maintained list of quoting rules.
#[derive(Debug, Clone, Copy)]
enum Extra {
    Num(&'static str, u64),
    Word(&'static str, &'static str),
}

impl Extra {
    /// Only the rendering paths are used in production; `key` exists so a test
    /// can assert the fixed order of the conditional tail.
    #[cfg(test)]
    fn key(&self) -> &'static str {
        match self {
            Extra::Num(k, _) | Extra::Word(k, _) => k,
        }
    }

    fn kv(&self) -> String {
        match self {
            Extra::Num(k, v) => format!("{k}={v}"),
            Extra::Word(k, v) => format!("{k}={v}"),
        }
    }

    fn json(&self) -> String {
        match self {
            Extra::Num(k, v) => format!("\"{k}\":{v}"),
            Extra::Word(k, v) => format!("\"{k}\":\"{v}\""),
        }
    }
}

/// The conditional tail of a `SUMMARY`, in a fixed order so two runs' lines
/// diff cleanly. Empty (and therefore invisible) on an unflagged run.
fn conditional_extras(sum: &Summary) -> Vec<Extra> {
    let f = &sum.flags;
    let mut out = Vec::new();
    if f.limited {
        out.push(Extra::Num("limited", 1));
    }
    if f.scoped {
        out.push(Extra::Num("scoped", 1));
        // A scoped run deliberately did not put the bytes in place, so it does
        // not advance config.ini (docs/04 §2.2). Emitted as a key rather than
        // left to be discovered by reading the file.
        out.push(Extra::Num("config_bumped", 0));
    }
    if f.limited || f.scoped {
        out.push(Extra::Num("plan_files", sum.files_total));
    }
    if let Some(n) = f.plan_download_bytes {
        out.push(Extra::Num("plan_download_bytes", n));
    }
    if let Some(n) = f.write_bytes {
        out.push(Extra::Num("write_bytes", n));
    }
    if let Some(s) = f.stopped {
        out.push(Extra::Word("stopped", s));
    }
    out
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
            cfg.exe_file_name,
            cfg.default_download_mode,
            pkg.branch,
            pkg.package_id,
            build.build_id
        )
    }
}

/// The final `SUMMARY` line, or the same fields as a JSON object with
/// `--json-summary`.
///
/// Trailing conditional keys (docs/04 §4.6: `limited`, `scoped`,
/// `config_bumped`, `plan_files`, …) are appended only when their flag is
/// active, so an unflagged run's line is byte-identical to today's.
pub fn format_summary_line(sum: &Summary, exit: i32, json: bool) -> String {
    let skipped = sum.files_skipped.load(Ordering::Relaxed);
    let repaired = sum.files_repaired.load(Ordering::Relaxed);
    let failed = sum.files_failed.load(Ordering::Relaxed);
    let dl = sum.download_bytes.load(Ordering::Relaxed);
    let del = sum.deleted_extra_bytes.load(Ordering::Relaxed);
    let extra = conditional_extras(sum);
    if json {
        let mut s = format!(
            "{{\"total\":{},\"skipped\":{},\"repaired\":{},\"failed\":{},\"download_bytes\":{},\"deleted_extra_bytes\":{},\"exit\":{}}}",
            sum.files_total, skipped, repaired, failed, dl, del, exit
        );
        if !extra.is_empty() {
            s.pop(); // the closing brace
            s.push(',');
            s.push_str(&extra.iter().map(Extra::json).collect::<Vec<_>>().join(","));
            s.push('}');
        }
        s
    } else {
        let tail: String = extra.iter().map(Extra::kv).collect::<Vec<_>>().join(" ");
        if tail.is_empty() {
            format!(
                "SUMMARY total={} skipped={} repaired={} failed={} download_bytes={} deleted_extra_bytes={} exit={exit}",
                sum.files_total, skipped, repaired, failed, dl, del
            )
        } else {
            format!(
                "SUMMARY total={} skipped={} repaired={} failed={} download_bytes={} deleted_extra_bytes={} exit={exit} {tail}",
                sum.files_total, skipped, repaired, failed, dl, del
            )
        }
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
            flags: SummaryFlags::default(),
            ..Summary::default()
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
        // An unflagged run gains no members at all — that is the whole
        // contract of the conditional keys.
        assert_eq!(v.as_object().unwrap().len(), 7, "{json}");
    }

    #[test]
    fn conditional_keys_appear_only_when_their_flag_is_active() {
        let mut s = Summary {
            files_total: 8,
            ..Summary::default()
        };
        s.flags.scoped = true;
        let line = format_summary_line(&s, 0, false);
        for k in ["scoped=1", "config_bumped=0", "plan_files=8"] {
            assert!(line.contains(k), "{k} missing from {line}");
        }
        assert!(!line.contains("limited"), "{line}");
        assert!(!line.contains("plan_download_bytes"), "{line}");
        assert!(!line.contains("write_bytes"), "{line}");
        assert!(!line.contains("stopped"), "{line}");
        // Fixed fields are untouched; only a tail is appended.
        assert!(line.starts_with("SUMMARY total=8 skipped=0 repaired=0 failed=0 download_bytes=0 deleted_extra_bytes=0 exit=0 "));

        // The same run as JSON: same keys, right types.
        let v: serde_json::Value = serde_json::from_str(&format_summary_line(&s, 0, true)).unwrap();
        assert_eq!(v["scoped"], 1);
        assert_eq!(v["config_bumped"], 0);
        assert_eq!(v["plan_files"], 8);
        assert_eq!(v.as_object().unwrap().len(), 10);
    }

    #[test]
    fn every_conditional_key_has_a_known_name_and_typed_rendering() {
        let mut s = Summary {
            files_total: 3,
            ..Summary::default()
        };
        s.flags = SummaryFlags {
            limited: true,
            scoped: true,
            plan_download_bytes: Some(4242),
            write_bytes: Some(77),
            stopped: Some("bytes"),
        };
        let line = format_summary_line(&s, 5, false);
        assert_eq!(
            line,
            "SUMMARY total=3 skipped=0 repaired=0 failed=0 download_bytes=0 deleted_extra_bytes=0 \
             exit=5 limited=1 scoped=1 config_bumped=0 plan_files=3 plan_download_bytes=4242 \
             write_bytes=77 stopped=bytes"
        );
        // `stopped` is a word, not a number: the JSON plane must say so.
        let v: serde_json::Value = serde_json::from_str(&format_summary_line(&s, 5, true)).unwrap();
        assert_eq!(v["stopped"], "bytes");
        assert_eq!(v["write_bytes"], 77);
        assert_eq!(v["plan_download_bytes"], 4242);
        // Key order is fixed, so two runs' lines diff cleanly.
        let keys: Vec<&str> = conditional_extras(&s).iter().map(Extra::key).collect();
        assert_eq!(
            keys,
            vec![
                "limited",
                "scoped",
                "config_bumped",
                "plan_files",
                "plan_download_bytes",
                "write_bytes",
                "stopped"
            ]
        );
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
        assert_eq!(snap.flags, SummaryFlags::default());
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
