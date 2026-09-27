//! girpr — Genshin Impact low-disk repair patcher (library root).
//!
//! Module map:
//! - [`biz`] — biz/channel/launcher/game-id mapping
//! - [`hyp`] — HoYoPlay client (`getGameConfigs`/`getGameBranches`/`getBuild`/`getDeprecated`)
//! - [`sophon`] — chunk-manifest protobuf, fetch+verify+parse, manifest filtering
//! - [`plan`] — work list (`RepairPlan`/`PlannedFile`, `build_plan`)
//! - [`repair`] — pipeline orchestration + per-file repair, files-cleanup, check-only
//! - [`report`] — `Summary` counters and the `REPORT`/`PROGRESS`/`SUMMARY` line formats
//! - [`error`] — `RunFailure`, the exit-code-carrying error type
//! - [`util`] — MD5 helpers, path normalization, `config.ini` read/write

use std::path::PathBuf;

use clap::Parser;

pub mod biz;
pub mod error;
pub mod hyp;
pub mod plan;
pub mod repair;
pub mod report;
pub mod sophon;
pub mod util;

pub use biz::Biz;
pub use error::RunFailure;

#[derive(Parser, Debug)]
#[command(name = "girpr", about = "Genshin Impact low-disk repair patcher")]
pub struct Args {
    /// Game install directory (contains config.ini, *_Data, ...)
    #[arg(long)]
    pub game_path: PathBuf,

    /// Game biz / channel
    #[arg(long, value_enum)]
    pub biz: Biz,

    /// Audio languages to keep (repeatable). If omitted, keeps current selection.
    #[arg(long = "audio")]
    pub audio: Vec<String>,

    /// Max concurrent FILES (chunks inside a file are sequential -> HDD friendly).
    /// SSD: 4-8, HDD: 1-2.
    #[arg(long, default_value_t = 4)]
    pub io_threads: usize,

    /// Delete files not in the live manifest after patching (Collapse files-cleanup parity).
    /// Same cleanup as `--purge-before`, only the timing differs.
    #[arg(long, default_value_t = false)]
    pub purge_after: bool,

    /// Same files-cleanup as `--purge-after`, but run before patching
    /// (frees space for the repair itself).
    #[arg(long, default_value_t = false)]
    pub purge_before: bool,

    /// Only verify (size+md5) and report; writes nothing
    #[arg(long, default_value_t = false)]
    pub check_only: bool,

    /// Print actions without writing anything
    #[arg(long, default_value_t = false)]
    pub dry_run: bool,

    /// Emit the begin REPORT and final SUMMARY stdout lines as JSON
    #[arg(long, default_value_t = false)]
    pub json_summary: bool,

    /// Log level
    #[arg(long, default_value = "info")]
    pub log_level: String,
}

/// Normalize user audio input ("English(US)", "en_us", ...) to sophon matching_field.
pub fn normalize_audio_lang(s: &str) -> Option<String> {
    match s.to_lowercase().replace('_', "-").as_str() {
        "zh-cn" | "chinese" => Some("zh-cn".to_string()),
        "en-us" | "english" | "english(us)" => Some("en-us".to_string()),
        "ja-jp" | "japanese" => Some("ja-jp".to_string()),
        "ko-kr" | "korean" => Some("ko-kr".to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_lang_normalizes_aliases_and_rejects_unknown() {
        assert_eq!(normalize_audio_lang("zh-cn"), Some("zh-cn".to_string()));
        assert_eq!(normalize_audio_lang("Chinese"), Some("zh-cn".to_string()));
        assert_eq!(normalize_audio_lang("chinese"), Some("zh-cn".to_string()));
        assert_eq!(normalize_audio_lang("en-us"), Some("en-us".to_string()));
        assert_eq!(normalize_audio_lang("en_us"), Some("en-us".to_string()));
        assert_eq!(normalize_audio_lang("English"), Some("en-us".to_string()));
        assert_eq!(
            normalize_audio_lang("English(US)"),
            Some("en-us".to_string())
        );
        assert_eq!(normalize_audio_lang("ja-jp"), Some("ja-jp".to_string()));
        assert_eq!(normalize_audio_lang("Japanese"), Some("ja-jp".to_string()));
        assert_eq!(normalize_audio_lang("ko-kr"), Some("ko-kr".to_string()));
        assert_eq!(normalize_audio_lang("Korean"), Some("ko-kr".to_string()));
        assert_eq!(normalize_audio_lang("fr-fr"), None);
        assert_eq!(normalize_audio_lang(""), None);
    }
}
