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
//! - [`effective`] — cache + filters + case rules collapsed into one map per side
//! - [`diff`] — path-set diffing
//! - [`commands`] — `update`, `compare`, `compare-self`, `sync`
//! - [`filter`], [`hash`], [`util`], [`logging`] — supporting primitives

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
pub mod scan;
pub mod util;

pub use cli::{Cli, Cmd, CommonArgs, TrustArgs, TrustSide};
pub use commands::{cmd_compare, cmd_compare_self, cmd_sync, cmd_update, run};
pub use config::{
    CommonOpts, CompareOpts, CompareSelfOpts, LogCtx, ScanMode, SyncOpts, TrustOpts, UpdateOpts,
};
pub use effective::{EffRec, ScanStats, SideScan};
pub use logging::init_tracing;
