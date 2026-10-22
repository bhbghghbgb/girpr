//! The clap surface, verbatim.
//!
//! This file parses and nothing else. Validation that needs more than one flag —
//! mutual exclusions, defaults resolved against the API, budgets against
//! selections — lives in [`crate::config`], so that the same rule is applied to a
//! command line and to a plan assembled by a test.
//!
//! Every argument-level failure exits `1`, never clap's default. The tool's own
//! table reserves `2` for network failure, and a flag typo must not be
//! indistinguishable from a CDN outage.

use std::path::PathBuf;

use clap::Args as ClapArgs;
use clap::Parser;
use clap::Subcommand;
use clap::ValueEnum;

use crate::exit::ExitCode;
use crate::logging::Level;
use crate::report::OutputFormat;

/// Sentinel accepted by `--new-tag` meaning "whatever `getGameBranches` says is
/// current". It is a flag value rather than a lookup performed by clap, so the
/// resolution happens against a live API answer and is recorded in the artifact.
pub const NEW_TAG_LATEST: &str = "latest";

/// Dev-only capture tool for `girpr` testing.
#[derive(Debug, Parser)]
#[command(
    name = "gircap",
    version,
    about = "Capture live HoYoPlay/Sophon artifacts as a self-contained offline fixture bundle",
    long_about = "gircap fetches from the live HoYoPlay and Sophon APIs and writes a self-contained \
artifact bundle: a real game_dir fixture holding one real file per test-matrix variant, every API \
response and manifest behind it, every chunk blob needed to materialize the old file, the current \
file and the patch between them, and ground truth for the result.\n\nIt is a capture tool. It never \
patches, never repairs, and never invokes the tool under test.",
    disable_help_subcommand = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub cmd: Cmd,
}

/// The four commands.
///
/// The variants are uneven by nature — `capture` has twenty flags and `list` has
/// one — and the enum is built exactly once per process, so the size difference
/// is not worth a `Box` that would only move the bytes.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Subcommand)]
pub enum Cmd {
    /// Fetch from the live APIs and write an artifact bundle.
    Capture(CaptureArgs),
    /// Re-run the artifact's own invariants from its stored bytes. No network.
    Verify(VerifyArgs),
    /// Rebuild the old and new files from `chunks/`. No network.
    Materialize(MaterializeArgs),
    /// Index every capture under a root directory.
    List(ListArgs),
}

/// Flags every command takes.
///
/// Flattened into all four rather than declared once per command, so a script
/// can pass `--log-level debug --output json` to anything and be right.
#[derive(Debug, Clone, ClapArgs)]
pub struct CommonArgs {
    /// stderr verbosity: `error`/`warn`/`info`/`debug`/`trace`.
    ///
    /// `RUST_LOG` *adds* to it: it can turn up other crates
    /// (`RUST_LOG=reqwest=debug`), but it cannot change this program's own
    /// level, because a flag is what the person running the tool typed.
    #[arg(long, value_name = "LV", default_value = "info", global = true, value_parser = girpr_log::parse_level)]
    pub log_level: Level,

    /// Log file verbosity: `error`/`warn`/`info`/`debug`/`trace`.
    ///
    /// Defaults to `debug`; `GIRCAP_LOG_FILE_LEVEL` says otherwise when this
    /// flag is absent. The file is one step louder than the console by
    /// default, because the file is read afterwards and the console is read
    /// while it happens.
    #[arg(long, value_name = "LV", global = true, value_parser = girpr_log::parse_level)]
    pub log_file_level: Option<Level>,

    /// Where the log file goes.
    ///
    /// By default it is `logs/gircap_<timestamp>.log` in a `logs/` directory
    /// beside the executable. `GIRCAP_LOG_FILE` names one when this flag does
    /// not. The file is always written, as JSON lines, and appended to.
    #[arg(long, value_name = "FILE", global = true)]
    pub log_file: Option<PathBuf>,

    /// How the single stdout summary is rendered.
    #[arg(long, value_name = "FORMAT", default_value = "text", global = true)]
    pub output: String,
}

/// `--gzip-threshold`'s default, in bytes.
///
/// A named constant rather than a literal at the flag and at the assertion that
/// reads it back: the number a capture records must be the number the flag
/// defaults to, and those are two different places in two different files.
pub const DEFAULT_GZIP_THRESHOLD: u64 = 1_048_576;

/// The `--audio` default: one launchable-install language, which is also a
/// deliberate widening of the selection pool.
///
/// A capture that selected no audio would have one fewer manifest to score the
/// matrix against, and every audio class would rest on nothing but the `game`
/// field's own candidates.
pub use crate::config::DEFAULT_AUDIO;

/// `gircap capture`.
#[derive(Debug, Clone, ClapArgs)]
pub struct CaptureArgs {
    /// Which region/server to capture.
    #[arg(long, value_enum, default_value_t = Biz::Hk4eGlobal)]
    pub biz: Biz,

    /// Artifact root. The bundle lands in `<out>/<biz>/<old>__<new>/<id>/`.
    #[arg(long, value_name = "DIR", default_value = "artifacts")]
    pub out: PathBuf,

    /// Version to patch *to*. `latest` = whatever the branch reports as current.
    #[arg(long, value_name = "TAG")]
    pub new_tag: Option<String>,

    /// Version the fixture is materialized at. Defaults to `diff_tags[0]`.
    #[arg(long, value_name = "TAG")]
    pub old_tag: Option<String>,

    /// How many files to admit per matrix class. Floored at 1: asking for zero
    /// is not a request for an empty bundle, it is a request to capture nothing
    /// while still charging admission for it. Use `--files` to restrict what is
    /// fetched, or `--max-download-bytes` to cap the cost.
    #[arg(long, value_name = "N", default_value_t = 1)]
    pub per_class: u32,

    /// Hard ceiling on the summed cost of admitted files. Unset = unlimited.
    #[arg(long, value_name = "BYTES")]
    pub max_download_bytes: Option<u64>,

    /// Fail with exit 4 unless these matrix classes are satisfied. Default: all.
    #[arg(long = "require-class", value_name = "ID")]
    pub require_class: Vec<String>,

    /// Capture these exact relative paths, bypassing matrix selection. Still
    /// scored and still recorded.
    #[arg(long = "files", value_name = "REL_PATH")]
    pub files: Vec<String>,

    /// Audio languages. `none` writes an empty scan file. Each code also adds a
    /// manifest to the selection pool.
    #[arg(long = "audio", value_name = "CODE", default_value = DEFAULT_AUDIO)]
    pub audio: Vec<String>,

    /// How much of each manifest to keep. `paths-only` skips the full decode dump.
    #[arg(long, value_enum, default_value_t = ManifestDetail::Full)]
    pub manifest_detail: ManifestDetail,

    /// Documents larger than this are gzipped, and the decision is recorded.
    #[arg(long, value_name = "BYTES", default_value_t = DEFAULT_GZIP_THRESHOLD)]
    pub gzip_threshold: u64,

    /// Extra tags to probe with `getBuild` (JSON metadata only, no blob).
    #[arg(long = "probe-tags", value_name = "TAG")]
    pub probe_tags: Vec<String>,

    /// Capture metadata only. Breaks assertions A7-A10 by design.
    #[arg(long = "no-chunk-blobs", default_value_t = false)]
    pub no_chunk_blobs: bool,

    /// Concurrent chunk downloads.
    #[arg(long, value_name = "N", default_value_t = 4)]
    pub jobs: usize,

    /// Attempts per request, including the first.
    #[arg(long, value_name = "N", default_value_t = 3)]
    pub retries: u32,

    /// Per-request timeout, seconds.
    #[arg(long, value_name = "S", default_value_t = 60)]
    pub timeout: u64,

    /// Ceiling on one response body, in bytes. A body larger than this is
    /// refused rather than buffered: an origin that keeps sending is bounded by
    /// this and not by the machine's memory. Raise it if a legitimate manifest
    /// archive ever approaches the default.
    #[arg(long, value_name = "BYTES", default_value_t = crate::http::DEFAULT_MAX_RESPONSE_BYTES)]
    pub max_response_bytes: u64,

    /// Override the HoYoPlay base. Testing seam: points every endpoint at a
    /// local server. Defaults to the production base for `--biz`.
    #[arg(long, value_name = "URL")]
    pub hyp_base: Option<String>,

    /// Override the Sophon `getBuild` base. Testing seam; see `--hyp-base`.
    #[arg(long, value_name = "URL")]
    pub sophon_base: Option<String>,

    #[command(flatten)]
    pub common: CommonArgs,
}

/// `gircap verify`.
#[derive(Debug, Clone, ClapArgs)]
pub struct VerifyArgs {
    /// The bundle to re-check.
    #[arg(long, value_name = "DIR")]
    pub artifact: PathBuf,

    #[command(flatten)]
    pub common: CommonArgs,
}

/// `gircap materialize`.
#[derive(Debug, Clone, ClapArgs)]
pub struct MaterializeArgs {
    /// The bundle to rebuild from.
    #[arg(long, value_name = "DIR")]
    pub artifact: PathBuf,

    /// Where to write the rebuilt trees. Defaults to writing beside the bundle.
    #[arg(long, value_name = "DIR")]
    pub out: Option<PathBuf>,

    #[command(flatten)]
    pub common: CommonArgs,
}

/// `gircap list`.
#[derive(Debug, Clone, ClapArgs)]
pub struct ListArgs {
    /// Root to index. Every `<biz>/<old>__<new>/<id>/` under it is listed.
    #[arg(long, value_name = "DIR")]
    pub root: PathBuf,

    #[command(flatten)]
    pub common: CommonArgs,
}

/// The three regions `gircap` knows how to address.
///
/// The launcher and game ids behind each are resolved in [`crate::biz`], never
/// written here, so the flag set stays the only place a region is named.
///
/// The `value` spellings are the *API* ids (`hk4e_global`), not clap's derived
/// kebab-case (`hk4e-global`). They have to match the ids the APIs return,
/// because a bundle's documents name the region by API id and a reader joining a
/// `--biz` flag to a `by_field` row must not have to learn a second spelling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, ValueEnum)]
pub enum Biz {
    /// Mainland China (`hk4e_cn`).
    #[value(name = "hk4e_cn")]
    Hk4eCn,
    /// International (`hk4e_global`).
    #[value(name = "hk4e_global")]
    Hk4eGlobal,
    /// Bilibili (`hk4e_bilibili`).
    #[value(name = "hk4e_bilibili")]
    Hk4eBilibili,
}

impl Biz {
    /// The id as the APIs spell it.
    pub const fn as_str(self) -> &'static str {
        match self {
            Biz::Hk4eCn => "hk4e_cn",
            Biz::Hk4eGlobal => "hk4e_global",
            Biz::Hk4eBilibili => "hk4e_bilibili",
        }
    }
}

impl std::fmt::Display for Biz {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// How much of a decoded manifest to keep on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, ValueEnum)]
pub enum ManifestDetail {
    /// The whole protobuf, verbatim, so no consumer re-downloads to inspect it.
    #[default]
    Full,
    /// Only the per-path sidecar and the counters.
    PathsOnly,
}

/// A usage error, carrying the code it maps to.
///
/// Every constructor here is `ExitCode::Usage`. The type exists so that a
/// validation failure cannot be reported under a different code by accident:
/// there is exactly one way to make one.
#[derive(Debug)]
pub struct UsageError {
    message: String,
}

impl UsageError {
    /// Build one.
    pub fn new(message: impl Into<String>) -> Self {
        UsageError {
            message: message.into(),
        }
    }

    /// The message, without any prefix.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// The exit code a usage error always maps to.
    pub const fn exit_code(&self) -> ExitCode {
        ExitCode::Usage
    }
}

impl std::fmt::Display for UsageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for UsageError {}

/// Shorthand for this module's own parse step.
pub type Result<T> = std::result::Result<T, UsageError>;

/// Resolve `--log-level` and `--output` into their typed forms.
///
/// `--log-level` is typed by `clap` itself — a bad level is an argument-level
/// failure, and it fails where every other bad argument fails. `--output` is
/// resolved here, once, so no command re-parses it and no command can disagree
/// about what `default_value` meant.
pub fn resolve_common(common: &CommonArgs) -> Result<CommonResolved> {
    Ok(CommonResolved {
        level: common.log_level,
        file_level: common.log_file_level,
        output: common
            .output
            .parse::<OutputFormat>()
            .map_err(UsageError::new)?,
        log_file: common.log_file.clone(),
    })
}

/// The typed form of [`CommonArgs`].
#[derive(Debug, Clone)]
pub struct CommonResolved {
    /// stderr verbosity.
    pub level: Level,
    /// The log file's verbosity, when the flag named one.
    pub file_level: Option<Level>,
    /// stdout rendering.
    pub output: OutputFormat,
    /// Where the JSON-lines log goes, if the caller named one.
    pub log_file: Option<PathBuf>,
}

/// Parse an argv into a [`Cli`], mapping every argument-level failure to exit
/// `1` and leaving `--help` / `--version` at the conventional `0`.
///
/// clap's own `parse()` exits `2` on a bad flag, which would collide with this
/// tool's "network or API failure" code. Both functions stay here so there is one
/// place that decides it.
pub fn parse_from<I, T>(argv: I) -> std::result::Result<Cli, clap::Error>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    Cli::try_parse_from(argv)
}

/// Exit with `code` on a clap error, having printed clap's own message.
///
/// Only `--help` and `--version` are `0`. Everything else — including a
/// *missing* subcommand, which clap renders as a help screen — is a usage error
/// and exits `1`, because a harness that treats "printed help" as "ran
/// successfully" cannot tell a help request from a command line with a typo
/// dropped off the end of it.
pub fn exit_on_clap_error(e: clap::Error) -> ! {
    let code = clap_error_code(e.kind());
    let _ = e.print();
    crate::exit::exit_with(code)
}

/// The exit code a clap error maps to, without exiting.
///
/// Exposed so the process-level contract can be asserted against one function
/// rather than against a spawned binary in every case; `tests/cli_contract.rs`
/// spawns the binary anyway, to prove the shim actually calls this.
pub fn clap_error_code(kind: clap::error::ErrorKind) -> ExitCode {
    match kind {
        clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion => {
            ExitCode::Ok
        }
        _ => ExitCode::Usage,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory as _;

    #[test]
    fn the_cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    fn parse(args: &[&str]) -> Cli {
        let mut argv = vec!["gircap"];
        argv.extend_from_slice(args);
        parse_from(argv).unwrap_or_else(|e| panic!("{args:?}: {e}"))
    }

    fn parse_err(args: &[&str]) -> clap::Error {
        let mut argv = vec!["gircap"];
        argv.extend_from_slice(args);
        match parse_from(argv) {
            Ok(_) => panic!("{args:?} unexpectedly parsed"),
            Err(e) => e,
        }
    }

    #[test]
    fn capture_defaults_match_the_documented_ones() {
        let cli = parse(&["capture"]);
        let Cmd::Capture(a) = cli.cmd else {
            panic!("expected capture")
        };
        assert_eq!(a.biz, Biz::Hk4eGlobal);
        assert_eq!(a.out, PathBuf::from("artifacts"));
        assert_eq!(a.new_tag, None);
        assert_eq!(a.old_tag, None);
        assert_eq!(a.per_class, 1);
        assert_eq!(a.max_download_bytes, None);
        assert_eq!(a.require_class, Vec::<String>::new());
        assert_eq!(a.audio, vec!["en-us".to_string()]);
        assert_eq!(a.manifest_detail, ManifestDetail::Full);
        assert_eq!(a.gzip_threshold, 1_048_576);
        assert!(!a.no_chunk_blobs);
        assert_eq!(a.jobs, 4);
        assert_eq!(a.retries, 3);
        assert_eq!(a.timeout, 60);
    }

    #[test]
    fn every_flag_on_the_spec_surface_parses() {
        let cli = parse(&[
            "capture",
            "--biz",
            "hk4e_cn",
            "--out",
            r"E:\a b\artifacts",
            "--new-tag",
            "latest",
            "--old-tag",
            "6.7.0",
            "--per-class",
            "3",
            "--max-download-bytes",
            "104857600",
            "--require-class",
            "identical",
            "--require-class",
            "added",
            "--files",
            "GenshinImpact_Data/Plugins/HoYoChannel.dll",
            "--audio",
            "none",
            "--manifest-detail",
            "paths-only",
            "--gzip-threshold",
            "0",
            "--probe-tags",
            "6.6.0",
            "--no-chunk-blobs",
            "--jobs",
            "8",
            "--retries",
            "5",
            "--timeout",
            "120",
            "--log-level",
            "debug",
            "--log-file",
            "run.log",
            "--output",
            "json",
        ]);
        let Cmd::Capture(a) = cli.cmd else {
            panic!("expected capture")
        };
        assert_eq!(a.biz, Biz::Hk4eCn);
        assert_eq!(a.out, PathBuf::from(r"E:\a b\artifacts"));
        assert_eq!(a.new_tag.as_deref(), Some(NEW_TAG_LATEST));
        assert_eq!(a.old_tag.as_deref(), Some("6.7.0"));
        assert_eq!(a.per_class, 3);
        assert_eq!(a.max_download_bytes, Some(104_857_600));
        assert_eq!(a.require_class, vec!["identical", "added"]);
        assert_eq!(a.files.len(), 1);
        assert_eq!(a.audio, vec!["none".to_string()]);
        assert_eq!(a.manifest_detail, ManifestDetail::PathsOnly);
        assert_eq!(a.probe_tags, vec!["6.6.0"]);
        assert!(a.no_chunk_blobs);
        assert_eq!(a.jobs, 8);
        assert_eq!(a.retries, 5);
        assert_eq!(a.timeout, 120);
        assert_eq!(a.common.log_level, Level::Debug);
        assert_eq!(a.common.log_file_level, None);
        assert_eq!(a.common.log_file, Some(PathBuf::from("run.log")));
        assert_eq!(a.common.output, "json");
    }

    #[test]
    fn the_other_three_commands_take_the_shared_flags() {
        let Cmd::Verify(v) = parse(&["verify", "--artifact", "a", "--output", "json"]).cmd else {
            panic!("expected verify")
        };
        assert_eq!(v.artifact, PathBuf::from("a"));
        assert_eq!(v.common.output, "json");

        let Cmd::Materialize(m) = parse(&[
            "materialize",
            "--artifact",
            "a",
            "--out",
            "b",
            "--log-level",
            "trace",
        ])
        .cmd
        else {
            panic!("expected materialize")
        };
        assert_eq!(m.artifact, PathBuf::from("a"));
        assert_eq!(m.out, Some(PathBuf::from("b")));
        assert_eq!(m.common.log_level, Level::Trace);

        let Cmd::List(l) = parse(&["list", "--root", "artifacts"]).cmd else {
            panic!("expected list")
        };
        assert_eq!(l.root, PathBuf::from("artifacts"));
    }

    #[test]
    fn repeatable_flags_accumulate_rather_than_overwrite() {
        let Cmd::Capture(a) = parse(&[
            "capture", "--files", "a", "--files", "b", "--audio", "zh-cn", "--audio", "ja-jp",
        ])
        .cmd
        else {
            panic!("expected capture")
        };
        assert_eq!(a.files, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(a.audio, vec!["zh-cn".to_string(), "ja-jp".to_string()]);
    }

    /// `--audio` has a default *and* is repeatable, so the "did the user ask?"
    /// question cannot be answered by the flag being present. It is answered by
    /// the value being exactly the default, which is the one spelling no one
    /// would type. Pinned so the resolution in `config` keeps working.
    #[test]
    fn an_unpassed_audio_flag_leaves_exactly_the_default() {
        let Cmd::Capture(a) = parse(&["capture"]).cmd else {
            panic!("expected capture")
        };
        assert_eq!(a.audio, vec![crate::config::DEFAULT_AUDIO]);
        let Cmd::Capture(b) = parse(&["capture", "--audio", "en-us"]).cmd else {
            panic!("expected capture")
        };
        assert_eq!(
            b.audio, a.audio,
            "explicit default and absent are indistinguishable"
        );
    }

    #[test]
    fn an_unknown_flag_and_an_unknown_subcommand_both_fail() {
        assert_eq!(
            parse_err(&["capture", "--nope"]).kind(),
            clap::error::ErrorKind::UnknownArgument
        );
        assert_eq!(
            parse_err(&["frobnicate"]).kind(),
            clap::error::ErrorKind::InvalidSubcommand
        );
    }

    #[test]
    fn a_bad_enum_value_is_rejected_with_the_valid_set() {
        let e = parse_err(&["capture", "--biz", "hk4e_turkey"]);
        assert_eq!(e.kind(), clap::error::ErrorKind::InvalidValue);
        let text = e.to_string();
        assert!(text.contains("hk4e_global"), "{text}");
    }

    #[test]
    fn help_and_version_are_the_only_zero_exits() {
        assert_eq!(
            parse_err(&["--help"]).kind(),
            clap::error::ErrorKind::DisplayHelp
        );
        assert_eq!(
            parse_err(&["capture", "--help"]).kind(),
            clap::error::ErrorKind::DisplayHelp
        );
        assert_eq!(
            parse_err(&["--version"]).kind(),
            clap::error::ErrorKind::DisplayVersion
        );
    }

    /// clap renders a *missing subcommand* as a help screen, which is why the
    /// remapping is easy to get wrong by accident. It is still a usage error:
    /// the caller asked for nothing and got nothing done.
    #[test]
    fn a_missing_subcommand_is_a_usage_error_not_a_help_request() {
        let kind = parse_err(&[]).kind();
        assert_eq!(clap_error_code(kind), ExitCode::Usage, "{kind:?}");
        assert_ne!(kind, clap::error::ErrorKind::DisplayHelp);
    }

    #[test]
    fn only_help_and_version_map_to_zero() {
        assert_eq!(
            clap_error_code(clap::error::ErrorKind::DisplayHelp),
            ExitCode::Ok
        );
        assert_eq!(
            clap_error_code(clap::error::ErrorKind::DisplayVersion),
            ExitCode::Ok
        );
        for kind in [
            clap::error::ErrorKind::UnknownArgument,
            clap::error::ErrorKind::InvalidSubcommand,
            clap::error::ErrorKind::InvalidValue,
            clap::error::ErrorKind::MissingRequiredArgument,
            clap::error::ErrorKind::ValueValidation,
        ] {
            assert_eq!(clap_error_code(kind), ExitCode::Usage, "{kind:?}");
        }
    }

    #[test]
    fn a_missing_required_argument_fails_rather_than_defaulting() {
        assert_eq!(
            parse_err(&["verify"]).kind(),
            clap::error::ErrorKind::MissingRequiredArgument
        );
    }

    #[test]
    fn common_flags_resolve_to_typed_forms_or_a_usage_error() {
        let ok = resolve_common(&CommonArgs {
            log_level: Level::Debug,
            log_file_level: Some(Level::Trace),
            log_file: Some("x.log".into()),
            output: "json".into(),
        })
        .unwrap();
        assert_eq!(ok.level, Level::Debug);
        assert_eq!(ok.file_level, Some(Level::Trace));
        assert_eq!(ok.output, OutputFormat::Json);
        assert_eq!(ok.log_file, Some(PathBuf::from("x.log")));

        let bad_output = resolve_common(&CommonArgs {
            log_level: Level::Info,
            log_file_level: None,
            log_file: None,
            output: "yaml".into(),
        })
        .unwrap_err();
        assert_eq!(bad_output.exit_code(), ExitCode::Usage);
    }

    /// A bad level is an *argument* failure, so it is rejected by the parser
    /// rather than by the command that happens to read it.
    #[test]
    fn a_bad_log_level_is_rejected_by_the_parser() {
        let e = parse_err(&["capture", "--log-level", "loud"]);
        assert_eq!(e.kind(), clap::error::ErrorKind::ValueValidation);
        let text = e.to_string();
        assert!(text.contains("loud"), "{text}");
        assert!(text.contains("error, warn, info, debug, trace"), "{text}");
    }

    /// The file's level has no clap default, so that
    /// `GIRCAP_LOG_FILE_LEVEL` can answer for a flag that was never given.
    /// The default is `debug`, and it is the *policy* crate's default.
    #[test]
    fn the_file_level_is_unset_until_something_says_otherwise() {
        let Cmd::Capture(a) = parse(&["capture"]).cmd else {
            panic!("expected capture")
        };
        assert_eq!(a.common.log_level, Level::Info);
        assert_eq!(a.common.log_file_level, None);
        let Cmd::Capture(b) = parse(&["capture", "--log-file-level", "trace"]).cmd else {
            panic!("expected capture")
        };
        assert_eq!(b.common.log_file_level, Some(Level::Trace));
    }

    #[test]
    fn biz_spellings_are_the_api_ids() {
        assert_eq!(Biz::Hk4eCn.as_str(), "hk4e_cn");
        assert_eq!(Biz::Hk4eGlobal.to_string(), "hk4e_global");
        assert_eq!(Biz::Hk4eBilibili.as_str(), "hk4e_bilibili");
    }
}
