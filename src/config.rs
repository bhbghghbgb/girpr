//! Validated per-run configuration, and the typed failure that carries the
//! process exit code.
//!
//! The CLI hands over raw strings ([`crate::cli::Args`]); [`RunCtx`] is the
//! checked form every layer below works with — audio already normalized to
//! canonical codes, `jobs >= 1` already enforced. Nothing downstream re-parses
//! user input.
//!
//! [`RunFailure`] is the single error type [`crate::repair::run`] returns, so
//! the exit-code contract in the README / docs/02 (§1) is enforced at the raise
//! site instead of a blanket `2`.

use std::collections::HashSet;
use std::path::PathBuf;

use crate::Biz;
use crate::repair::scope::{Limits, PathScope};
use crate::report::OutputFormat;

/// How a repair puts the bytes on disk.
///
/// The default is [`RepairMode::Auto`], which repairs **in place** wherever
/// it can: the target is written where it lies, so a repair costs the growth
/// of the file rather than a second copy of it (docs/07 §2). The other modes
/// exist because the in-place contract is deliberately weaker than the
/// temp-then-promote one (docs/07 §5), and a user who wants the old
/// guarantee has to be able to ask for it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
#[value(rename_all = "kebab-case")]
pub enum RepairMode {
    /// In place when the target can be opened read+write; the temp path when
    /// it cannot. The default.
    #[default]
    Auto,
    /// Always in place: the target is written where it lies. Falls back to
    /// the temp path only on an I/O error, and says so when it does.
    Inplace,
    /// The shipped behaviour: write `{path}_tmp`, then rename it over the
    /// target. The original is never modified, and the extra space is one
    /// whole file per file in flight.
    Temp,
    /// Delete the target first, then write it fresh. Costs no extra space
    /// and no network reuse — the two are the same thing, and the mode is
    /// only useful when the user knows the file has nothing worth keeping.
    Discard,
}

impl RepairMode {
    /// The value as it appears in `--repair-mode` and in the `SUMMARY` key.
    pub fn as_str(&self) -> &'static str {
        match self {
            RepairMode::Auto => "auto",
            RepairMode::Inplace => "inplace",
            RepairMode::Temp => "temp",
            RepairMode::Discard => "discard",
        }
    }
}

impl std::fmt::Display for RepairMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The default staging budget: how much RAM the in-place repair may use to
/// hold the bytes that have to move inside a file.
///
/// It is a **budget**, not a hint: a file whose payload does not fit in what
/// is left of it does not fail — it falls to the ordered tier, which writes
/// the same file without staging anything (docs/07 §4.3). The default is
/// small on purpose: the common repair (a corrupt file) stages **nothing**,
/// and the case that stages the most is the one where the file is being
/// rewritten wholesale anyway.
pub const DEFAULT_RAM_BUDGET: u64 = 64 * 1024 * 1024;

/// Typed failure carrying the process exit code.
/// `1` usage/config, `2` metadata/network, `3` write/verify.
#[derive(Debug)]
pub struct RunFailure {
    pub exit_code: i32,
    pub source: anyhow::Error,
}

impl RunFailure {
    pub fn usage(e: impl Into<anyhow::Error>) -> Self {
        Self {
            exit_code: 1,
            source: e.into(),
        }
    }
    pub fn metadata(e: impl Into<anyhow::Error>) -> Self {
        Self {
            exit_code: 2,
            source: e.into(),
        }
    }
    pub fn write(e: impl Into<anyhow::Error>) -> Self {
        Self {
            exit_code: 3,
            source: e.into(),
        }
    }
}

impl std::fmt::Display for RunFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:#}", self.source)
    }
}
impl std::error::Error for RunFailure {}

/// Everything one `girpr` run needs, already validated.
#[derive(Debug)]
pub struct RunCtx {
    pub game_dir: PathBuf,
    pub biz: Biz,
    /// Effective audio selection, already normalized to canonical codes
    /// (`zh-cn` / `en-us` / `ja-jp` / `ko-kr`); empty means game-only.
    pub audio: HashSet<String>,
    /// `true` when `--audio` was passed (including `--audio none` = explicit
    /// game-only empty set). `false` (flag omitted) means autodetect: keep the
    /// scan file, else default `en-us` for automation.
    pub audio_explicit: bool,
    /// Compiled `--include` / `--exclude` globs. The patterns are
    /// compiled once, in [`crate::cli::Args::into_ctx`], so a `RunCtx` that
    /// exists at all has a valid scope — a bad glob was already exit `1`. An
    /// *open* scope admits every path, which makes every `is_active()` gate
    /// downstream a no-op, so an unflagged run is unaffected.
    pub path_scope: PathScope,
    /// Manually requested resource ceilings. All `None` in an
    /// unflagged run, which is what makes every refusal a no-op and keeps
    /// `SUMMARY limited=1` absent.
    pub limits: Limits,
    /// How the repair writes: in place, through a `_tmp`, or by discarding
    /// the target first. `Auto` (the default) picks per file.
    pub repair_mode: RepairMode,
    /// The effective staging budget for the run, in bytes: how much RAM the
    /// in-place repair may use to hold the bytes that have to move inside a
    /// file. `--max-ram-bytes` sets it; otherwise it is
    /// [`DEFAULT_RAM_BUDGET`].
    ///
    /// It is a *budget*, not a limit that refuses a run: a file that cannot
    /// get its reservation falls to the ordered tier and is repaired anyway.
    pub ram_budget: u64,
    /// Concurrent **files**; guaranteed `>= 1`.
    pub jobs: usize,
    pub check_only: bool,
    /// `--purge-only`: `check_only` inverted. Runs the files-cleanup and the
    /// post-phase, skips the repair loop.
    pub purge_only: bool,
    /// `--fail-fast`: stop the run once one file exhausts its retries.
    pub fail_fast: bool,
    pub dry_run: bool,
    /// `--print-plan`: report the plan and stop. Read-only by construction —
    /// the pipeline returns before `create_dir_all` — and **not** the same thing
    /// as `check_only`, which answers "what is wrong" rather than "what would
    /// you do".
    pub print_plan: bool,
    /// `--plan-out <FILE>`: where to write the per-file plan JSON.
    pub plan_out: Option<PathBuf>,
    pub purge_after: bool,
    pub purge_before: bool,
    /// How the data plane is rendered: `key=value` lines (the default) or NDJSON.
    /// One [`crate::report::Record`] per line either way — see [`crate::report`].
    pub output: OutputFormat,
    /// `--log-file <FILE>`: the requested destination of the always-`TRACE`
    /// trace log. `None` means the timestamped default under `<exe-dir>/logs/`
    /// ([`crate::logging::log_file_path`]). Carried on the context rather than
    /// applied here because the file layer is installed before any validation
    /// runs — a usage error is meant to be diagnosable from the log too.
    pub log_file: Option<PathBuf>,
    /// Offline/test endpoint overrides. When `Some`, the HoYoPlay / Sophon
    /// APIs are fetched from these bases instead of the production CDNs
    /// (e.g. a local mock server in `tests/`). `None` = production, unless
    /// the `GIRPR_HYP_BASE` / `GIRPR_SOPHON_BASE` env vars are set (used by
    /// binary-level e2e runs; explicit `Some` wins over env).
    pub hyp_base_override: Option<String>,
    pub sophon_base_override: Option<String>,
}

impl RunCtx {
    /// Read-only modes must not write anything: `--dry-run` logs actions only,
    /// `--check-only` verifies only. Every delete/write in the pipeline is
    /// gated on this, which covers `--purge-before` as well as the phases
    /// `--check-only` never reaches.
    pub fn readonly(&self) -> bool {
        self.dry_run || self.check_only
    }

    /// Resolve the effective API bases: explicit override > env var >
    /// production.
    pub fn api_bases(&self) -> (Option<String>, Option<String>) {
        let env = |k: &str| std::env::var(k).ok().filter(|s| !s.is_empty());
        (
            self.hyp_base_override
                .clone()
                .or_else(|| env("GIRPR_HYP_BASE")),
            self.sophon_base_override
                .clone()
                .or_else(|| env("GIRPR_SOPHON_BASE")),
        )
    }
}
