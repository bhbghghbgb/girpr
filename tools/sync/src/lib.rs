//! girsync — dev-only one-way mirror src(old) -> dst(working dir) with a redb hash cache.
//!
//! Layout: `<folder>/girpr-cache` is a redb file.
//! Backups: `<folder>/girpr-cache-backup-<ts>` and `<folder>/girpr-cache-old-<ts>`
//! (file copies; kept all by default, bounded by `--backup-retain N`). A copy
//! is written under `<folder>/girpr-cache-partial-<ts>` and published under its
//! own name once it is complete, so a name in a copy list never holds half a
//! copy. A published copy carries a record of its own beside it —
//! `<folder>/girpr-cache-hold-backup-<ts>` or
//! `<folder>/girpr-cache-hold-old-<ts>` — which says the copy is in use by
//! the run that published it.
//! Record-only inputs: any path whose final component
//! starts with `girpr-cache`.
//!
//! Module map:
//! - [`cli`] / [`config`] — argument surface and validated per-run options
//! - [`cache`] — redb schema, open/rebuild, backup snapshots
//! - [`scan`] — live filesystem walk
//! - [`effective`] — phase A (stat + cache) and phase C (hash) for one side
//! - [`planner`] — [`HashPlan`]: the only place that decides what must be hashed
//! - [`rw`] — [`RwLimits`]/[`RwRuntime`]: how many file reads and writes may be in
//!   flight, and whether the two sides draw on one counter or two
//! - [`diff`] — path-set diffing
//! - [`commands`] — `update`, `compare`, `compare-self`, `sync`
//! - [`filter`], [`link`], [`hardlink`], [`util`], [`logging`] - supporting primitives
//! - [`hash`] — directory module whose `mod.rs` is a facade; see [`hash`] for
//!   the submodule map
//! - [`report`] — the stdout data plane: [`report::Record`] rendered as text
//!   or JSON by `--output`
//!
//! `cache`, `config`, `diff`, `effective`, `hash`, `planner`, `report` and `rw`
//! are each a directory module whose `mod.rs` is a facade; the leaves own one
//! concern apiece.
//!
//! Two channels, deliberately separate. **stdout** is the answer: differences,
//! plan actions and summaries, as [`report::Record`]s in the `--output` format.
//! **stderr** is the narration: `tracing` events, filtered by `--log-level` and
//! optionally also written to a JSON log file. Nothing is printed raw.
//!
//! The log file's *policy* — where it goes, which level each sink gets, which
//! environment variable answers for which flag — is the workspace's and lives
//! in the `girpr-log` crate, shared with the repair binary and with `gircap`.

// The cache is now an upstream crate rather than a module of this lib: the
// heavy `redb`/`postcard`/`tempfile` tree and the cache's byte codec stay
// cached when a planner, command or other subsystem below changes. The alias
// keeps every `crate::cache::X` call site and the public `girsync::cache::X`
// surface unchanged.
// The acyclic leaves live upstream in `girpr-sync-leaves` and are re-exported
// under their old module names, so no `crate::X` call site moves. girsync
// depends on that crate directly (it is not reached through another module), so
// an edit to the planner, commands or config above leaves all four leaves and
// their rw-native dependencies cached. The crate is also consumed directly by
// its own integration tests, without this facade.
pub use girpr_sync_cache as cache;
pub use girpr_sync_leaves::{filter, hardlink, link, rw};
pub mod cli;
pub mod commands;
pub mod config;
pub mod diff;
pub mod effective;
pub mod hash;
pub mod logging;
pub mod planner;
pub mod report;
pub mod scan;
pub mod util;

// The log policy crate, re-exported so that a caller of [`run`] can name the
// type of the plan it takes without adding a dependency of its own: the
// policy is part of this crate's public signature, so it is part of this
// crate's public surface.
pub use girpr_log;

// The glob crate, re-exported for the same reason: [`Filter::includes`] and
// [`Filter::excludes`] hand back `glob::Pattern`, so the type is part of this
// crate's public surface, and a caller that wants to hold a compiled pattern
// needs no dependency of its own. A caller that wants to build one rather than
// read it reaches [`glob::Pattern::new`] here too. It comes through
// `girpr-filter`, the single place that pins the glob version both filters
// compile against.
pub use girpr_filter::glob;

pub use cli::{Cli, Cmd, CommonArgs, RwArgs, TrustArgs, TrustSide};
pub use commands::{
    CompareOutcome, RunOutcome, cmd_compare, cmd_compare_self, cmd_sync, cmd_update,
    compare_detailed, compare_self_detailed, run, run_captured, run_captured_with_hooks,
};
pub use config::{
    BackupPolicy, CommonOpts, CompareOpts, CompareSelfOpts, LogCtx, ScanMode, SyncOpts, TestHooks,
    TrustOpts, UpdateOpts,
};
pub use effective::{EffRec, ScanStats, SideCapability, SideEntry, SideScan};
// The filter's type and the crate's own way to build its patterns, re-exported
// so a caller that builds a [`CommonOpts`] by hand names one module path: the
// options are at the crate root, so the type of one of their fields has to be
// reachable there too.
//
// The crate's own way is not the only way - [`glob::Pattern::new`] builds a
// pattern too - but it is the one that names the offending glob.
pub use filter::{Filter, compile_patterns};
pub use girpr_log::Level;
pub use logging::{LogGuard, init_tracing, plan, subscriber_for};
pub use planner::HashPlan;
pub use report::{OutputFormat, Record, Report, ReportSink, verdict, visit_verdict};
pub use rw::{RwLimits, RwPermit, RwRuntime, RwSide};
