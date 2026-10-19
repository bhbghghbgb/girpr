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
