//! Argument surface: the raw clap `Args` plus the input normalization that runs
//! before anything else (audio codes, `jobs >= 1`).
//!
//! [`Args`] is deliberately dumb: it describes the flags and nothing more. The
//! checked form is [`crate::config::RunCtx`], built in [`Args::into_ctx`] so the
//! validation order is one readable function and the error text reaches stderr
//! unchanged.

use std::collections::HashSet;
use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::Parser;

use crate::Biz;
use crate::config::RunCtx;

#[derive(Parser, Debug)]
#[command(name = "girpr", about = "Genshin Impact low-disk repair patcher")]
pub struct Args {
    /// Game install directory (contains config.ini, *_Data, ...)
    #[arg(long)]
    pub game_path: PathBuf,

    /// Game biz / channel
    #[arg(long, value_enum)]
    pub biz: Biz,

    /// Audio languages to keep (repeatable). If omitted, keeps current selection
    /// (falls back to `en-us` if the scan file is missing). Use `--audio none`
    /// for game-only (no audio); it overwrites the scan file with an empty list.
    /// `none` cannot be mixed with language codes.
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

impl Args {
    /// Validate the raw flags into a [`RunCtx`]. Every failure here is a usage
    /// error (exit 1), so the caller only has to pick the exit code.
    pub fn into_ctx(self) -> Result<RunCtx> {
        if self.io_threads == 0 {
            bail!("io-threads must be >= 1");
        }
        let (audio, _saw_none) = parse_audio_selection(&self.audio)?;
        Ok(RunCtx {
            game_dir: self.game_path,
            biz: self.biz,
            audio,
            // No `--audio` at all -> autodetect (keep scan file, else en-us).
            // `--audio none` alone -> explicit game-only (empty set, overwrites
            // scan file).
            audio_explicit: !self.audio.is_empty(),
            jobs: self.io_threads,
            check_only: self.check_only,
            dry_run: self.dry_run,
            purge_after: self.purge_after,
            purge_before: self.purge_before,
            json_summary: self.json_summary,
            // Production run: no overrides (env GIRPR_HYP_BASE/GIRPR_SOPHON_BASE
            // are still honored by `RunCtx::api_bases` for binary-level e2e).
            hyp_base_override: None,
            sophon_base_override: None,
        })
    }
}

/// Split the repeatable `--audio` values into a canonical language set.
///
/// Returns `(set, saw_none)`. `none` is the game-only sentinel and is handled
/// separately from [`normalize_audio_lang`] so it is never confused with an
/// unknown language code. The second element only feeds logging; the
/// game-only/empty-set decision is already visible in `set.is_empty()`.
pub fn parse_audio_selection(values: &[String]) -> Result<(HashSet<String>, bool)> {
    let mut audio = HashSet::new();
    let mut saw_none = false;
    for a in values {
        if is_audio_none(a) {
            saw_none = true;
            continue;
        }
        match normalize_audio_lang(a) {
            Some(n) => {
                audio.insert(n);
            }
            None => bail!("unknown audio lang '{a}' (want zh-cn|en-us|ja-jp|ko-kr|none)"),
        }
    }
    if saw_none && !audio.is_empty() {
        bail!("--audio none cannot be mixed with language codes");
    }
    Ok((audio, saw_none))
}

/// `true` for the game-only sentinel (`--audio none`, case-insensitive).
pub fn is_audio_none(s: &str) -> bool {
    s.eq_ignore_ascii_case("none")
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

    fn sel(values: &[&str]) -> Result<(HashSet<String>, bool)> {
        let owned: Vec<String> = values.iter().map(|s| s.to_string()).collect();
        parse_audio_selection(&owned)
    }

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

    #[test]
    fn audio_none_sentinel_is_case_insensitive_and_not_a_lang() {
        assert!(is_audio_none("none"));
        assert!(is_audio_none("NONE"));
        assert!(!is_audio_none("en-us"));
        assert_eq!(normalize_audio_lang("none"), None);
    }

    #[test]
    fn selection_is_deduped_and_none_is_game_only() {
        let (set, saw_none) = sel(&["zh-cn", "chinese", "ja-jp"]).unwrap();
        assert!(!saw_none);
        assert_eq!(set.len(), 2);
        assert!(set.contains("zh-cn") && set.contains("ja-jp"));

        let (set, saw_none) = sel(&["none"]).unwrap();
        assert!(saw_none);
        assert!(set.is_empty());
    }

    #[test]
    fn none_cannot_be_mixed_and_unknown_is_rejected() {
        assert!(sel(&["en-us", "none"]).is_err());
        assert!(sel(&["fr-fr"]).is_err());
    }

    #[test]
    fn zero_threads_is_a_usage_error() {
        let args = Args::parse_from([
            "girpr",
            "--game-path",
            ".",
            "--biz",
            "hk4e_global",
            "--io-threads",
            "0",
        ]);
        let err = args.into_ctx().unwrap_err().to_string();
        assert!(err.contains("io-threads must be >= 1"), "{err}");
    }

    #[test]
    fn omitted_audio_is_not_explicit_and_defaults_jobs_to_4() {
        let args = Args::parse_from(["girpr", "--game-path", ".", "--biz", "hk4e_global"]);
        let ctx = args.into_ctx().unwrap();
        assert!(!ctx.audio_explicit);
        assert!(ctx.audio.is_empty());
        assert_eq!(ctx.jobs, 4);
    }
}
