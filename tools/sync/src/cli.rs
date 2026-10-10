//! Command-line surface.
//!
//! The three subcommands share every flag except their positional inputs and a
//! handful of mode switches, so the shared ones live in [`CommonArgs`] and are
//! flattened into each variant. `run` converts them into [`crate::config`]
//! structs once, which is where the strings get validated.

use clap::{Args, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

use crate::report::OutputFormat;

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
    /// How stdout reports are written: text (one human line per record) or json
    /// (one JSON object per line, same records and same order).
    ///
    /// Only stdout is affected. Diagnostics go to stderr as `tracing` events
    /// either way, so `--output json` yields a stream of JSON records on stdout
    /// with no human chatter mixed in — unless `--log-file` is given, which adds
    /// a third, always-trace JSON log beside it.
    #[arg(long, value_enum, default_value_t = OutputFormat::Text, global = true)]
    pub output: OutputFormat,
    #[command(subcommand)]
    pub cmd: Cmd,
}

/// Flags shared by every subcommand: hashing, glob filters, case mode, walk
/// depth, cache reset, and the dry run.
#[derive(Args, Debug, Clone)]
pub struct CommonArgs {
    /// Hash algorithm; repeatable (md5, sha256). "none" disables hashing entirely.
    ///
    /// **Every** algorithm named here must be available on both sides of a pair that
    /// size+mtime cannot settle. A folder backfills one it lacks; a record has no
    /// filesystem to read from, so a record that lacks one is an error naming the
    /// remedy, rather than the pair quietly falling back to stat.
    ///
    /// `--hash-any-of` asks for only one of several, and the run picks it per pair.
    #[arg(
        long = "hash-all-of",
        default_values_t = vec!["md5".to_string()],
        conflicts_with = "hash_any_of"
    )]
    pub hash_all_of: Vec<String>,
    /// As `--hash-all-of`, but only **one** of the named algorithms is required.
    ///
    /// The run picks which, per pair: cheapest first — one both sides already hold
    /// costs nothing, one costs a read, two cost two — and within a tier, the order
    /// given here. So `--hash-any-of sha256 md5` prefers sha256.
    ///
    /// Use it when a side holds only some of what you would otherwise demand, which
    /// is the common case for a record written by an older run: any-of then reads
    /// the one the record has instead of refusing. Every algorithm it picks is
    /// still obtainable on **both** sides, so a pair settled this way is settled by
    /// a digest both sides hold — which is the difference between this and the old
    /// behaviour of skipping whatever was missing.
    ///
    /// `none` is not accepted here: with nothing requested there is no "any of" to
    /// choose, and `--hash-all-of none` is the stat-only audit.
    #[arg(long = "hash-any-of", conflicts_with = "hash_all_of")]
    pub hash_any_of: Vec<String>,
    /// Do not let a **size** difference settle a pair: treat those pairs as needing a
    /// digest.
    ///
    /// A size difference already implies different content, so this cannot change a
    /// verdict — the digest comparison will always disagree. What it buys is the read,
    /// and that repairs the cache: a size-changed row arrives with its digests dropped
    /// (a stale row's are all suspect), so under the default it stays stat-only forever
    /// and can never become comparable again.
    ///
    /// Use `--no-trust-mtime` instead when a file's mtime moved but its bytes may not
    /// have — that one *can* change a verdict, because mtime alone is not evidence.
    #[arg(long, default_value_t = false)]
    pub no_trust_size: bool,
    /// Do not let an **mtime** difference settle a pair: treat those pairs as needing a
    /// digest.
    ///
    /// This is the flag that can change a verdict. A file whose mtime moved but whose
    /// bytes did not — a `touch`, an extractor rewriting identical bytes — is reported
    /// `CHANGED` by default; with this flag it is compared on content and may come back
    /// equal.
    ///
    /// The cost is a read of every mtime-changed pair on both sides.
    #[arg(long, default_value_t = false)]
    pub no_trust_mtime: bool,
    /// Say **why** each `CHANGED` record is `CHANGED`: add `why=<tag>` to it.
    ///
    /// A `CHANGED` line tells you *that* the two sides disagree; this tells you which
    /// check decided it. The tag is a short machine-readable string a parser can switch
    /// on — `stat-size`, `stat-mtime`, `stat-size+stat-mtime`, `digest-differs:<algos>`,
    /// `digest-matches:<algos>`, `stat-match`, `unverifiable:<fields>`, `dir-present`
    /// — split on the first `:` if there is detail after it.
    ///
    /// One tag per **decision**, never per check. The diff is a short circuit, so the
    /// tag names whatever settled the pair: a stat difference needs no digest at all, and
    /// mentioning the digest that was skipped would be reporting an effort rather than a
    /// reason.
    ///
    /// Only `CHANGED` gets one. `MISSING`, `EXTRA`, `TYPE-CONFLICT` and `CASE-MISMATCH`
    /// are self-explanatory — the bucket name is the reason — and a tag there would be
    /// an explanation nobody asked for.
    ///
    /// **Default off**, and an unflagged run is byte-identical to a run without this
    /// flag. It changes nothing but the text: not the exit code, not the counts, not the
    /// work done.
    #[arg(long, default_value_t = false)]
    pub why: bool,
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
    /// Write nothing at all: no cache created, no cache updated, no backup, and
    /// for a command that moves files, no filesystem change.
    ///
    /// Every command accepts this, including the ones that never touch a file
    /// tree. `compare` does write — a folder side updates its own cache as it
    /// resolves — so without this flag there is no way to audit two folders and
    /// leave both caches byte-identical. `compare-self` never writes and ignores
    /// it with a warning.
    ///
    /// The guarantee is that the run *answers the same question*, not that it
    /// does less work: it still stats the tree, still reads cached digests, and
    /// still hashes whatever stat alone cannot settle. Only the writes are gone.
    #[arg(long, default_value_t = false)]
    pub dry_run: bool,
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
    /// Build/refresh the record for a folder (always hashes per --hash-all-of, prunes missing).
    Update {
        #[arg(long)]
        dir: PathBuf,
        #[command(flatten)]
        common: CommonArgs,
    },
    /// Compare two sides (each: folder root or girpr-cache* record dir).
    ///
    /// Never modifies the two trees, but a folder side does write its cache as it
    /// resolves. Pass `--dry-run` to leave both caches exactly as they were.
    Compare {
        #[arg(long)]
        src: PathBuf,
        #[arg(long)]
        dst: PathBuf,
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
    /// The folder side is scanned with **no cache to consult**, which is what
    /// makes this an audit. A file whose size+mtime still match is rehashed, so
    /// content that changed while preserving both is reported rather than taken
    /// at its recorded digest.
    CompareSelf {
        /// The folder to audit. Its `girpr-cache` is the record side.
        #[arg(long)]
        dir: PathBuf,
        /// Accepted and ignored. The disk side has no cache to distrust, so an
        /// undecided pair is always rehashed and content drift that preserved size
        /// and mtime is already reported.
        #[arg(long, default_value_t = false)]
        no_trust_cached_hashes: bool,
        // `--dry-run` arrives here via `CommonArgs` and is accepted but has no
        // effect: this command opens its record read-only and scans the folder
        // against an in-memory cache, so it already writes nothing.
        // `cmd_compare_self` warns rather than erroring, so a script passing it
        // everywhere does not break here.
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
        // `--dry-run` arrives via `CommonArgs`. This command threads it into the
        // rename and apply phases too, so it means no cache writes *and* no
        // filesystem change.
        /// Parallel copy workers.
        #[arg(long, default_value_t = 4)]
        jobs: usize,
        #[command(flatten)]
        trust: TrustArgs,
        #[command(flatten)]
        common: CommonArgs,
    },
}
