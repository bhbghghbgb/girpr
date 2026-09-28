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
    /// Concurrent **files**; guaranteed `>= 1`.
    pub jobs: usize,
    pub check_only: bool,
    pub dry_run: bool,
    pub purge_after: bool,
    pub purge_before: bool,
    pub json_summary: bool,
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
    /// gated on this (previously `--check-only` still swept temps and
    /// `--purge-before` still deleted — both fixed by gating on `readonly`, not
    /// just `dry_run`).
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
