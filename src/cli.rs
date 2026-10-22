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
use girpr_log::Level;

use crate::Biz;
use crate::config::{RepairMode, RunCtx};
use crate::logging::PROJECT;
use crate::repair::scope::{Limits, PathScope};
use crate::report::OutputFormat;

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

    /// Only act on paths matching this glob (repeatable). Matched against the
    /// `/`-separated path relative to `--game-path`, case-insensitively.
    /// NOTE: `*` also crosses `/`, so `*.blk` matches at any depth. Paths that
    /// do not match are treated as nonexistent: not repaired, not deleted, and
    /// `config.ini` is not bumped. With no `--include`, every path is admitted.
    #[arg(long = "include", value_name = "GLOB")]
    pub include: Vec<String>,

    /// Treat paths matching this glob as nonexistent (repeatable). Beats
    /// `--include`. e.g. `--exclude "*_tmp"` protects in-flight temp files.
    #[arg(long = "exclude", value_name = "GLOB")]
    pub exclude: Vec<String>,

    /// Refuse the run (exit 5) if the plan touches more than N files. `0` is a
    /// valid ceiling, not a usage error. Checked *after* `--include`, so it
    /// means "the files the scope selected".
    #[arg(long, value_name = "N")]
    pub max_files: Option<u64>,

    /// Refuse the run (exit 5) if the files-cleanup would delete more than N
    /// bytes. Counted once over both purge passes; out-of-scope paths are not
    /// candidates and so never count.
    #[arg(long, value_name = "N")]
    pub max_delete_bytes: Option<u64>,

    /// Refuse the run (exit 5) if the plan's estimated chunk downloads exceed N
    /// bytes, and hard-stop mid-flight if reuse falls back to more than the
    /// estimate predicted. The estimate counts only what the local chunk map
    /// cannot supply, so a ceiling set to the estimate still stops a run whose
    /// reuse path breaks. Does not bound metadata or manifest traffic.
    #[arg(long, value_name = "N")]
    pub max_download_bytes: Option<u64>,

    /// Hard-stop mid-flight (exit `5`, `SUMMARY stopped=--max-write-bytes`)
    /// once this many bytes have been written into `*_tmp`. Unlike
    /// `download_bytes`, this is the disk cost: every reuse-copy and every
    /// chunk write is counted, so a zero-reuse run is bounded too. Cannot be a
    /// pre-flight refusal — reuse success is decided per chunk at runtime.
    #[arg(long, value_name = "N")]
    pub max_write_bytes: Option<u64>,

    /// Hard-stop mid-flight (exit `5`,
    /// `SUMMARY stopped=--max-verified-download-bytes`) once this many
    /// **verified** download bytes have been made — the bytes of the files
    /// that were actually repaired, not the traffic the run caused. A budget
    /// on the *repair* rather than on the network: a retry that fetched the
    /// same chunk twice, and a chunk that failed its digest, cost nothing
    /// here. Cannot be a pre-flight refusal — the split between the two
    /// figures is only known at runtime.
    #[arg(long, value_name = "N")]
    pub max_verified_download_bytes: Option<u64>,

    /// Hard-stop mid-flight (exit `5`,
    /// `SUMMARY stopped=--max-verified-write-bytes`) once this many bytes
    /// have been written into the install **from sources that passed their
    /// gate**. The same split as `--max-write-bytes`, on the other half: the
    /// bytes that landed without passing a gate are spent and are what
    /// `--max-write-bytes` bounds; this bounds the bytes the repair was
    /// allowed to spend. Cannot be a pre-flight refusal — the split between
    /// the two figures is only known at runtime.
    #[arg(long, value_name = "N")]
    pub max_verified_write_bytes: Option<u64>,

    /// Cap the RAM the in-place repair may use to stage the bytes it has to
    /// move inside a file. A file that cannot get its reservation is written
    /// in the other direction instead of failing — the same file, with less
    /// reuse. `0` forces that path.
    #[arg(long, value_name = "N")]
    pub max_ram_bytes: Option<u64>,

    /// How each file is repaired.
    /// `auto` (the default) repairs in place when the target can be opened
    /// for writing, and writes it the `temp` way when it cannot; `inplace`
    /// forces the in-place path; `temp` assembles the repair in `{path}_tmp`
    /// and renames it over the target — **the target file is never modified**
    /// and the swap is atomic, at the cost of the whole file existing twice;
    /// `discard` deletes the target and writes it from scratch.
    #[arg(long, value_enum)]
    pub repair_mode: Option<RepairMode>,

    /// Read the whole install once, in the plan phase, and report the
    /// **exact** figures: the bytes the repair will really download, the
    /// chunks that are damaged, and the bytes the scan read. Costs a full
    /// read of every planned file, and makes a ceiling refusal slower (the
    /// estimate a ceiling is checked against is only exact after the scan).
    #[arg(long, default_value_t = false)]
    pub scan: bool,

    /// Only verify (size+md5) and report; writes nothing
    #[arg(long, default_value_t = false)]
    pub check_only: bool,

    /// Make the install match the live manifest without repairing anything:
    /// runs the files-cleanup and the post-phase, skips the repair loop. The
    /// inverse of `--check-only` ("is it intact?" → "make it match").
    /// `config.ini` is **not** bumped — a cleanup does not put the bytes in
    /// place, so claiming the newer version would desync the launcher.
    #[arg(long, default_value_t = false)]
    pub purge_only: bool,

    /// Stop the run as soon as one file exhausts its retries: no further files
    /// are started, and a file gets one attempt instead of five. Without it one
    /// failing file costs ~125 s of pure retry sleeping (5 file attempts ×
    /// linear backoff, each with its own 5× chunk retry), so a plan of a few
    /// thousand files can hang for hours before reaching its SUMMARY. Counters
    /// stay honest: the file that failed still counts as failed, exit `3`.
    #[arg(long, default_value_t = false)]
    pub fail_fast: bool,

    /// Print actions without writing anything
    #[arg(long, default_value_t = false)]
    pub dry_run: bool,

    /// Print the whole plan (`PLAN files=… est_download_bytes=… purge_files=…`)
    /// to stdout and exit 0 without repairing, purging or bumping anything.
    /// NOT an offline mode: the plan *is* the manifest, so this still fetches
    /// the API and the manifests, exactly as any other run does.
    #[arg(long, default_value_t = false)]
    pub print_plan: bool,

    /// Also write the per-file plan detail to this file as JSON
    /// `{repair:[{rel,size,chunks_total,est_download_bytes}],purge:[{rel,size}]}`.
    /// Implies nothing: use it with `--print-plan`, or on its own to dump the
    /// plan of a run that does the work.
    #[arg(long, value_name = "FILE")]
    pub plan_out: Option<PathBuf>,

    /// How stdout is written: one human `key=value` line per record, or one JSON
    /// object per line (NDJSON) carrying the same records in the same order. Only
    /// stdout is affected — diagnostics stay on stderr as `tracing` events either
    /// way — and `--log-file`, if given, carries the same rendered lines.
    #[arg(long, value_enum, default_value_t, global = true)]
    pub output: OutputFormat,

    /// Console verbosity: `error`/`warn`/`info`/`debug`/`trace`.
    ///
    /// The console is for the person watching the run, so it is `info` by
    /// default. `RUST_LOG` replaces it when it is set — one lever, not two
    /// that can disagree.
    #[arg(long, value_name = "LV", default_value = "info", value_parser = girpr_log::parse_level)]
    pub log_level: Level,

    /// Log file verbosity: `error`/`warn`/`info`/`debug`/`trace`.
    ///
    /// The file is for the person reading it afterwards, so it is one step
    /// louder than the console: `debug` by default. `GIRPR_LOG_FILE_LEVEL`
    /// says otherwise when this flag is absent. The two levels are separate
    /// because they answer separate questions, and a run that wants a quiet
    /// console and a detailed file should not have to choose.
    #[arg(long, value_name = "LV", value_parser = girpr_log::parse_level)]
    pub log_file_level: Option<Level>,

    /// Where the log file goes.
    ///
    /// By default it is `logs/girpr_<timestamp>.log` in a `logs/` directory
    /// beside the executable. `GIRPR_LOG_FILE` names one when this flag does
    /// not. Missing parent directories are created. A file that cannot be
    /// opened is a stderr warning, never a failure.
    #[arg(long, value_name = "FILE")]
    pub log_file: Option<PathBuf>,

    /// Point the HoYoPlay API at this base URL instead of the production CDN.
    /// Overrides the `GIRPR_HYP_BASE` env var. NOTE: this redirects
    /// `getGameConfigs`/`getGameBranches`/`getBuild`/`getGameDeprecatedFileConfigs`
    /// only — the manifest and chunk URLs come from `url_prefix` inside the
    /// *response bodies*, so a mock server must return bodies pointing at itself.
    #[arg(long, value_name = "URL")]
    pub hyp_base: Option<String>,

    /// Point the Sophon `getBuild` API at this base URL. Overrides
    /// `GIRPR_SOPHON_BASE`. Same caveat as `--hyp-base`: only `getBuild` is
    /// redirected; manifests and chunks follow `url_prefix` in the bodies.
    #[arg(long, value_name = "URL")]
    pub sophon_base: Option<String>,
}

impl Args {
    /// Validate the raw flags into a [`RunCtx`]. Every failure here is a usage
    /// error (exit 1), so the caller only has to pick the exit code.
    pub fn into_ctx(self) -> Result<RunCtx> {
        if self.io_threads == 0 {
            bail!("io-threads must be >= 1");
        }
        let (audio, _saw_none) = parse_audio_selection(&self.audio)?;
        // A bad glob is a usage error (exit 1) and says which one it was.
        let path_scope = PathScope::new(&self.include, &self.exclude)?;
        Ok(RunCtx {
            game_dir: self.game_path,
            biz: self.biz,
            audio,
            // No `--audio` at all -> autodetect (keep scan file, else en-us).
            // `--audio none` alone -> explicit game-only (empty set, overwrites
            // scan file).
            audio_explicit: !self.audio.is_empty(),
            path_scope,
            limits: Limits {
                files: self.max_files,
                delete_bytes: self.max_delete_bytes,
                download_bytes: self.max_download_bytes,
                write_bytes: self.max_write_bytes,
                verified_download_bytes: self.max_verified_download_bytes,
                verified_write_bytes: self.max_verified_write_bytes,
                ram_bytes: self.max_ram_bytes,
            },
            repair_mode: self.repair_mode.unwrap_or_default(),
            // The flag's *presence* is what the `SUMMARY repair_mode=` key
            // reports, so it is tracked next to the value: an unflagged run
            // must not grow a key.
            repair_mode_explicit: self.repair_mode.is_some(),
            jobs: self.io_threads,
            check_only: self.check_only,
            scan: self.scan,
            purge_only: self.purge_only,
            fail_fast: self.fail_fast,
            dry_run: self.dry_run,
            print_plan: self.print_plan,
            plan_out: self.plan_out,
            purge_after: self.purge_after,
            purge_before: self.purge_before,
            output: self.output,
            // Where the log file goes, when the caller named one. The flag
            // wins over `GIRPR_LOG_FILE`; `None` keeps the timestamped
            // default beside the executable, which nothing outside the process
            // can name reliably — the flag and the environment variable are
            // what make the file findable.
            log_file: self
                .log_file
                .or_else(|| girpr_log::env_options(PROJECT).log_file),
            // Endpoint overrides: explicit flag > GIRPR_*_BASE env > production, resolved
            // by `RunCtx::api_bases`. Omitted flags stay `None`, so the env seam
            // every offline test already uses is untouched.
            hyp_base_override: self.hyp_base,
            sophon_base_override: self.sophon_base,
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

    #[test]
    fn the_repair_mode_defaults_to_auto_and_is_not_reported_as_asked_for() {
        // The default is `auto`, and the *absence* of the flag is tracked
        // separately: the `SUMMARY repair_mode=` key is emitted only when
        // the flag was passed.
        let args = Args::parse_from(["girpr", "--game-path", ".", "--biz", "hk4e_global"]);
        let ctx = args.into_ctx().unwrap();
        assert_eq!(ctx.repair_mode, RepairMode::Auto);
        assert!(!ctx.repair_mode_explicit);
    }

    #[test]
    fn every_repair_mode_is_carried_and_marked_explicit() {
        for (flag, want) in [
            ("auto", RepairMode::Auto),
            ("inplace", RepairMode::Inplace),
            ("temp", RepairMode::Temp),
            ("discard", RepairMode::Discard),
        ] {
            let args = Args::parse_from([
                "girpr",
                "--game-path",
                ".",
                "--biz",
                "hk4e_global",
                "--repair-mode",
                flag,
            ]);
            let ctx = args.into_ctx().unwrap();
            assert_eq!(ctx.repair_mode, want, "{flag}");
            assert!(ctx.repair_mode_explicit, "{flag}");
            assert_eq!(ctx.repair_mode.as_str(), flag);
        }
    }

    #[test]
    fn an_unknown_repair_mode_is_rejected_by_the_parser() {
        assert!(
            Args::try_parse_from([
                "girpr",
                "--game-path",
                ".",
                "--biz",
                "hk4e_global",
                "--repair-mode",
                "sideways",
            ])
            .is_err(),
            "an unknown mode is an argument-level failure"
        );
    }

    #[test]
    fn the_ram_budget_is_none_unless_it_was_asked_for() {
        let args = Args::parse_from(["girpr", "--game-path", ".", "--biz", "hk4e_global"]);
        let ctx = args.into_ctx().unwrap();
        assert_eq!(ctx.ram_budget(), None);

        let args = Args::parse_from([
            "girpr",
            "--game-path",
            ".",
            "--biz",
            "hk4e_global",
            "--max-ram-bytes",
            "1048576",
        ]);
        let ctx = args.into_ctx().unwrap();
        assert_eq!(ctx.ram_budget(), Some(1_048_576));
        assert!(ctx.limits.is_active());
    }
}
