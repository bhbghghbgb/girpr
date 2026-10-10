//! girsync — dev-only one-way mirror src(old) -> dst(working dir) with a redb hash cache.
//!
//! Layout: `<folder>/girpr-cache` is a redb file.
//! Backups: `<folder>/girpr-cache-backup-<ts>` and `<folder>/girpr-cache-old-<ts>`
//! (file copies, keep-all). Record-only inputs: any path whose final component
//! starts with `girpr-cache`.
//!
//! Module map:
//! - [`cli`] / [`config`] — argument surface and validated per-run options
//! - [`cache`] — redb schema, open/rebuild, backup snapshots
//! - [`convert`] — one-shot sled -> redb converter
//! - [`scan`] — live filesystem walk
//! - [`effective`] — phase A (stat + cache) and phase C (hash) for one side
//! - [`planner`] — [`HashPlan`]: the only place that decides what must be hashed
//! - [`rw`] — [`RwLimits`]/[`RwRuntime`]: how many file reads and writes may be in
//!   flight, and whether the two sides draw on one counter or two
//! - [`diff`] — path-set diffing
//! - [`commands`] — `update`, `compare`, `compare-self`, `sync`
//! - [`filter`], [`hash`], [`util`], [`logging`] — supporting primitives
//! - [`report`] — the stdout data plane: [`report::Record`] rendered as text
//!   or JSON by `--output`
//!
//! Two channels, deliberately separate. **stdout** is the answer: differences,
//! plan actions and summaries, as [`report::Record`]s in the `--output` format.
//! **stderr** is the narration: `tracing` events, filtered by `--log-level` and
//! optionally also written to a JSON `--log-file`. Nothing is printed raw.

pub mod cache;
pub mod cli;
pub mod commands;
pub mod config;
pub mod convert;
pub mod diff;
pub mod effective;
pub mod filter;
pub mod hash;
pub mod logging;
pub mod planner;
pub mod report;
pub mod rw;
pub mod scan;
pub mod util;

pub use cli::{Cli, Cmd, CommonArgs, TrustArgs, TrustSide};
pub use commands::{cmd_compare, cmd_compare_self, cmd_sync, cmd_update, run};
pub use config::{
    CommonOpts, CompareOpts, CompareSelfOpts, LogCtx, ScanMode, SyncOpts, TrustOpts, UpdateOpts,
};
pub use effective::{EffRec, ScanStats, SideCapability, SideEntry, SideScan};
pub use logging::init_tracing;
pub use planner::HashPlan;
pub use report::{OutputFormat, Record, Report, verdict};
pub use rw::{RwLimits, RwPermit, RwRuntime, RwSide};
