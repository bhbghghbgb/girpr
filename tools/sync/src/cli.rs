//! Command-line surface.
//!
//! The three subcommands share every flag except their positional inputs and a
//! handful of mode switches, so the shared ones live in [`CommonArgs`] and are
//! flattened into each variant. `run` converts them into [`crate::config`]
//! structs once, which is where the strings get validated.

use clap::{Args, Parser, Subcommand};
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
        /// Rehash every file instead of trusting cached hashes on size+mtime hits.
        #[arg(long, default_value_t = false)]
        no_fast: bool,
        #[command(flatten)]
        common: CommonArgs,
    },
    /// Mirror src folder -> dst folder.
    Sync {
        #[arg(long)]
        src: PathBuf,
        #[arg(long)]
        dst: PathBuf,
        /// Rehash every file instead of trusting cached hashes on size+mtime hits.
        #[arg(long, default_value_t = false)]
        no_fast: bool,
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
        common: CommonArgs,
    },
}
