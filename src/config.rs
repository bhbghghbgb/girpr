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
use crate::repair::persistent::DispatchOverride;
use crate::repair::scope::Limits;
use crate::report::OutputFormat;
use crate::retry::BackoffPolicy;
use girpr_exit::define_exit_table;
use girpr_repair_plan::PathScope;
use girpr_repair_policy::RepairMode;

// The table is generated from one list by the shared exit-table mechanism:
// the leaf owns the enum/`ALL`/code/pin pattern, while the codes, meanings and
// helper methods here are this binary's own contract (gircap has its own).
define_exit_table! {
    /// The documented process exit codes.
    ///
    /// The numbers are the automation contract: a run's final `SUMMARY` carries
    /// one, and a fatal failure carries one through [`RunFailure`]. Named
    /// variants keep the choice at each return site explicit. Converting a
    /// variant to the process status happens at the boundary in `main`; the
    /// `SUMMARY` and the fatal log line call [`RunExit::code`] only to describe
    /// the run.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum RunExit {
        /// The run did everything it was asked to do.
        Success = 0,
        /// The request or the configuration is wrong.
        Usage = 1,
        /// Metadata, manifest, or network failure.
        Metadata = 2,
        /// A write or verification failure.
        Write = 3,
        /// `--check-only` found damage.
        CheckFailed = 4,
        /// A requested ceiling bound the run.
        Limit = 5,
    }
    ALL
    /// Every documented exit status, in numeric order.
    ///
    /// Generated from the same list as the variants, so the list and
    /// the enum cannot drift apart. Crate-internal: only the
    /// compile-time check and the unit tests read it. The binary
    /// target is a separate crate, so its test cannot see this and
    /// names the statuses itself; that duplication is deliberate,
    /// since removing it would mean publishing the list for one test.
    /// A new variant still fails the binary's wildcard-free match.
    pub(crate),
    CODE
    /// The numeric status the process returns.
    ///
    /// `SUMMARY` and the fatal log line take this as an `i32`; the process
    /// boundary takes [`RunExit::process_status`] instead.
    ;
}

impl RunExit {
    /// The byte a process exit status carries.
    ///
    /// Infallible: `#[repr(u8)]` makes every discriminant fit, so no runtime
    /// range check is needed at the boundary.
    pub const fn process_status(self) -> u8 {
        self as u8
    }
}

impl From<RunFailure> for RunExit {
    fn from(failure: RunFailure) -> Self {
        failure.exit_code
    }
}

/// Typed failure carrying the process exit code.
#[derive(Debug)]
pub struct RunFailure {
    pub exit_code: RunExit,
    pub source: anyhow::Error,
}

impl RunFailure {
    pub fn usage(e: impl Into<anyhow::Error>) -> Self {
        Self {
            exit_code: RunExit::Usage,
            source: e.into(),
        }
    }
    pub fn metadata(e: impl Into<anyhow::Error>) -> Self {
        Self {
            exit_code: RunExit::Metadata,
            source: e.into(),
        }
    }
    pub fn write(e: impl Into<anyhow::Error>) -> Self {
        Self {
            exit_code: RunExit::Write,
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
    /// `--repair-mode`: how each file is repaired. `Auto` when the flag was
    /// not passed.
    pub repair_mode: RepairMode,
    /// `true` when `--repair-mode` was passed. The `SUMMARY repair_mode=`
    /// key is emitted only when it was — the same discipline every other
    /// conditional key follows, so an unflagged run's line stays
    /// byte-identical.
    pub repair_mode_explicit: bool,
    /// Concurrent **files**; guaranteed `>= 1`. Chunks inside a file stay
    /// sequential. This is a file-task window, not the runtime's worker-thread
    /// count; `--jobs` is the primary spelling and `--io-threads` is a
    /// deprecated alias.
    pub jobs: usize,
    /// `--with-persistent`: the one master switch for the dispatcher →
    /// manifests → `Persistent/` tier. Off (the default) every persistent call
    /// site is a no-op and the run's published bytes and exit codes stay as
    /// before.
    pub with_persistent: bool,
    /// Test-only seam: a canned dispatcher result. Production never sets it;
    /// see [`DispatchOverride`]. Hidden from the CLI on purpose — this is not
    /// a user-facing way to bypass the real key/seed provider.
    #[doc(hidden)]
    pub persistent_dispatch_override: Option<DispatchOverride>,
    /// `--dispatch-key <FILE>`: the private key for a supplied dispatcher
    /// record. Both flags must be present or both absent, and both require
    /// `--with-persistent`. `None` in an unflagged run.
    pub dispatch_key_path: Option<PathBuf>,
    /// `--dispatch-record <FILE>`: a `dispatch/record.json` whose digest facts
    /// are verified by decoding its envelope with `--dispatch-key`. `None` in
    /// an unflagged run.
    pub dispatch_record_path: Option<PathBuf>,
    pub check_only: bool,
    /// `--scan`: read the whole install once, in the plan phase, so the
    /// plan's numbers are exact rather than upper bounds.
    pub scan: bool,
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
    /// One [`crate::Record`] per line either way.
    pub output: OutputFormat,
    /// Offline/test endpoint overrides. When `Some`, the HYP / Sophon
    /// APIs are fetched from these bases instead of the production CDNs
    /// (e.g. a local mock server in `tests/`). `None` = production, unless
    /// the `GIRPR_HYP_BASE` / `GIRPR_SOPHON_BASE` env vars are set (used by
    /// binary-level e2e runs; explicit `Some` wins over env).
    pub hyp_base_override: Option<String>,
    pub sophon_base_override: Option<String>,
    /// Retry wait policy: production uses exponential backoff with jitter;
    /// tests use [`BackoffPolicy::none`] so retries are instant.
    pub backoff: BackoffPolicy,
}

impl RunCtx {
    /// The RAM the in-place repair may stage the bytes it has to move
    /// inside a file.
    ///
    /// `None` = no budget was asked for. A budget is not a refusal: a file
    /// that cannot get its reservation is written in the other direction
    /// instead, which produces the same file with less reuse.
    pub fn ram_budget(&self) -> Option<u64> {
        self.limits.ram_bytes
    }

    /// `--dry-run` and `--check-only` must not write anything: `--dry-run`
    /// logs actions only, `--check-only` verifies only. Every delete/write in
    /// the pipeline is gated on this, which covers `--purge-before` as well as
    /// the phases `--check-only` never reaches.
    ///
    /// This is **not** the whole read-only story: `--print-plan` is read-only
    /// by construction (it returns before the execution phase), but it does
    /// reach the metadata phase, where the explicit-`--audio` scan path is
    /// **validated** under `!readonly() && !print_plan`. The scan file itself
    /// is not written there: since R-7 the write lives in the execution phase
    /// (`repair::mod`), which `--print-plan` never reaches. A new write site
    /// must ask `!readonly()` (and must be unreachable from the plan-only
    /// returns), not trust this method alone.
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

#[cfg(test)]
mod tests {
    use super::{RunExit, RunFailure};

    /// The numbers are the automation contract, so pin the integer each named
    /// variant converts to. [`RunExit::ALL`] is the crate's exhaustive list;
    /// the compile-time check ties its order to the discriminants.
    #[test]
    fn every_named_code_keeps_its_documented_number() {
        for (number, status) in RunExit::ALL.into_iter().enumerate() {
            assert_eq!(status.code(), number as i32, "{status:?}");
        }
    }

    #[test]
    fn every_failure_constructor_carries_its_documented_code() {
        let cases = [
            (
                RunFailure::usage(anyhow::anyhow!("bad request")),
                RunExit::Usage,
            ),
            (
                RunFailure::metadata(anyhow::anyhow!("bad metadata")),
                RunExit::Metadata,
            ),
            (
                RunFailure::write(anyhow::anyhow!("bad write")),
                RunExit::Write,
            ),
        ];
        for (failure, expected) in cases {
            assert_eq!(failure.exit_code, expected);
            assert_eq!(RunExit::from(failure), expected);
        }
    }
}
