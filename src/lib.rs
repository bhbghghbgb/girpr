use std::path::PathBuf;

use clap::{Parser, ValueEnum};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Biz {
    #[value(name = "hk4e_cn")]
    Hk4eCn,
    #[value(name = "hk4e_global")]
    Hk4eGlobal,
    #[value(name = "hk4e_bilibili")]
    Hk4eBilibili,
}

impl Biz {
    pub fn as_str(&self) -> &'static str {
        match self {
            Biz::Hk4eCn => "hk4e_cn",
            Biz::Hk4eGlobal => "hk4e_global",
            Biz::Hk4eBilibili => "hk4e_bilibili",
        }
    }
    /// (host, launcher_id, game_id)
    pub fn endpoints(&self) -> (&'static str, &'static str, &'static str) {
        match self {
            Biz::Hk4eCn => ("mihoyo", "jGHBHlcOq1", "1Z8W5NHUQb"),
            Biz::Hk4eGlobal => ("hoyoverse", "VYTpXlbWo8", "gopR6Cufr3"),
            Biz::Hk4eBilibili => ("mihoyo", "umfgRO5gh5", "T2S0Gz4Dr2"),
        }
    }
    /// (channel, sub_channel, cps) written to config.ini
    pub fn channel_tuple(&self) -> (&'static str, &'static str, &'static str) {
        match self {
            Biz::Hk4eCn => ("1", "1", "hyp_mihoyo"),
            Biz::Hk4eGlobal => ("1", "0", "hyp_hoyoverse"),
            Biz::Hk4eBilibili => ("14", "0", "hyp_mihoyo"),
        }
    }
}

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

    /// Delete files not in the live manifest after patching (Collapse parity)
    #[arg(long, default_value_t = false)]
    pub purge_extra: bool,

    /// Also purge extra files BEFORE patching (frees space for the repair itself)
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
