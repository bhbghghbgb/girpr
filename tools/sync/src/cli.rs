//! Command-line surface.
//!
//! The three subcommands share every flag except their positional inputs and a
//! handful of mode switches, so the shared ones live in [`CommonArgs`] and are
//! flattened into each variant. `run` converts them into [`crate::config`]
//! structs once, which is where the strings get validated.

use clap::{Args, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "girsync",
    about = "Dev-only one-way folder mirror with hash cache"
)]
pub struct Cli {
    /// Console log level: trace|debug|info|warn|error (file log, if enabled, always captures trace+).
    #[arg(long, default_value = "info", global = true)]
    pub log_level: String,
    /// Optional log file path. File always records at trace level regardless of --log-level.
    #[arg(long, global = true)]
    pub log_file: Option<PathBuf>,
    #[command(subcommand)]
    pub cmd: Cmd,
}

/// Flags shared by every subcommand: hashing, glob filters, case mode, walk
/// depth, and cache reset.
#[derive(Args, Debug, Clone)]
pub struct CommonArgs {
    /// Hash algorithm; repeatable (md5, sha256). "none" disables hashing entirely.
    #[arg(long = "hash", default_values_t = vec!["md5".to_string()])]
    pub hash: Vec<String>,
    /// Only sync paths matching this glob; repeatable.
    #[arg(long = "include")]
    pub include: Vec<String>,
    /// Skip paths matching this glob; repeatable, wins over --include.
    #[arg(long = "exclude")]
    pub exclude: Vec<String>,
    /// Case-sensitive path handling. Default false = insensitive with rename/abort rules.
    #[arg(long, default_value_t = false)]
    pub case_sensitive: bool,
    /// Maximum directory depth to walk (applies to the whole tree).
    #[arg(long, default_value_t = 10)]
    pub max_depth: usize,
    /// Back up and rebuild the cache instead of trusting it.
    #[arg(long, default_value_t = false)]
    pub ignore_cache: bool,
}

/// One side of a two-sided run, as named on the command line.
#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrustSide {
    /// The `--src` side.
    Src,
    /// The `--dst` side.
    Dst,
}

/// Cache-trust overrides, shared by the two-sided subcommands.
///
/// `update` is absent on purpose: it is defined as a full repopulate, so it
/// never trusts cached digests and has nothing to override.
#[derive(Args, Debug, Clone, Default)]
pub struct TrustArgs {
    /// Rehash this side even when size+mtime match a cached digest. Repeatable;
    /// pass `src`, `dst`, or both. Off by default.
    #[arg(long = "no-trust-cached-hashes", value_name = "SIDE")]
    pub no_trust_cached_hashes: Vec<TrustSide>,
}

#[derive(Subcommand, Debug)]
pub enum Cmd {
    /// Build/refresh the record for a folder (always hashes per --hash, prunes missing).
    Update {
        #[arg(long)]
        dir: PathBuf,
        #[command(flatten)]
        common: CommonArgs,
    },
    /// Compare two sides (each: folder root or girpr-cache* record dir).
    Compare {
        #[arg(long)]
        src: PathBuf,
        #[arg(long)]
        dst: PathBuf,
        /// Write nothing at all — including the caches.
        ///
        /// `compare` never modifies the two trees, but a folder side does update
        /// its cache as it resolves, so without this flag there is no way to
        /// audit two folders and leave both caches untouched. The report is
        /// identical either way.
        #[arg(long, default_value_t = false)]
        dry_run: bool,
        #[command(flatten)]
        trust: TrustArgs,
        #[command(flatten)]
        common: CommonArgs,
    },
    /// Compare a folder against its own cache, writing nothing.
    ///
    /// The record side is `--dir`'s own `girpr-cache`, so this is `compare`
    /// with one argument: report what the cache has drifted from, without
    /// repairing the cache in the process.
    ///
    /// By default a file whose size+mtime still match the cache is taken at its
    /// recorded digest, so this reports *stat* drift. Add
    /// `--no-trust-cached-hashes` to rehash everything and report content
    /// drift too.
    CompareSelf {
        /// The folder to audit. Its `girpr-cache` is the record side.
        #[arg(long)]
        dir: PathBuf,
        /// Rehash every file even when size+mtime match a cached digest.
        #[arg(long, default_value_t = false)]
        no_trust_cached_hashes: bool,
        #[command(flatten)]
        common: CommonArgs,
    },
    /// Mirror src folder -> dst folder.
    Sync {
        #[arg(long)]
        src: PathBuf,
        #[arg(long)]
        dst: PathBuf,
        /// Only copy src-only (missing) files; skip content updates.
        #[arg(long, default_value_t = false)]
        missing_only: bool,
        /// Keep dst-only files (default deletes them).
        #[arg(long, default_value_t = false)]
        keep_extra: bool,
        /// Print the plan without touching the filesystem or cache.
        #[arg(long, default_value_t = false)]
        dry_run: bool,
        /// Parallel copy workers.
        #[arg(long, default_value_t = 4)]
        jobs: usize,
        #[command(flatten)]
        trust: TrustArgs,
        #[command(flatten)]
        common: CommonArgs,
    },
}
