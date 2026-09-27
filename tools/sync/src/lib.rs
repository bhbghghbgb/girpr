//! girsync — dev-only one-way mirror src(old) -> dst(working dir) with a sled hash cache.
//!
//! Layout: `<folder>/girpr-cache` is a sled DB directory.
//! Backups: `<folder>/girpr-cache-backup-<ts>` and `<folder>/girpr-cache-old-<ts>`
//! (dir copies, keep-all). Record-only inputs: any path whose final component
//! starts with `girpr-cache`.
//!
//! Module map:
//! - [`cli`] / [`config`] — argument surface and validated per-run options
//! - [`cache`] — sled schema, open/rebuild, backup snapshots
//! - [`scan`] — live filesystem walk
//! - [`effective`] — cache + filters + case rules collapsed into one map per side
//! - [`diff`] — path-set diffing
//! - [`commands`] — `update`, `compare`, `sync`
//! - [`filter`], [`hash`], [`util`], [`logging`] — supporting primitives

pub mod cache;
pub mod cli;
pub mod commands;
pub mod config;
pub mod diff;
pub mod effective;
pub mod filter;
pub mod hash;
pub mod logging;
pub mod scan;
pub mod util;

pub use cli::{Cli, Cmd, CommonArgs};
pub use commands::{cmd_compare, cmd_sync, cmd_update, run};
pub use config::{CommonOpts, CompareOpts, LogCtx, ScanMode, SyncOpts, UpdateOpts};
pub use logging::init_tracing;
