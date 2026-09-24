use std::path::PathBuf;

use clap::Parser;

pub mod biz;
pub use biz::Biz;

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

    /// Emit final summary as JSON on stdout
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

pub mod hyp;
pub mod repair;
pub mod sophon;
pub mod util;
