//! NDJSON export/import for girsync's redb hash cache.
//!
//! `cache-json to-json <CACHE> [NDJSON]` writes the cache as one JSON object
//! per line; `cache-json from-json <NDJSON> <CACHE>` builds a new cache from
//! that stream. `-` means stdout for export and stdin for import, so the tool
//! composes with ordinary Unix-style pipelines. Only the current cache schema
//! is supported on either side: this is a text view of today's rows, not a
//! migration path for old ones.
//!
//! The library is split so both halves are testable without spawning the
//! binary: [`ndjson`] owns the line shapes and the digest spelling, and
//! [`convert`] owns the file/stream conversion and the publish protocol. The
//! binary in `main.rs` is only argument parsing, reading/writing the `-`
//! streams, and a summary line on stderr.
//!
//! **stdout is the data, never the narration.** Export writes the NDJSON
//! there (unless a file was named), so the summary goes to stderr. Import
//! writes a cache file and prints nothing to stdout at all.

use std::path::PathBuf;

pub mod convert;
pub mod ndjson;

pub use convert::{export_cache, import_cache};

// Deliberately no doc comment on `Args`: clap reads one as the command's
// `long_about` and would print it verbatim in `--help`.
#[derive(Debug, clap::Parser)]
#[command(
    name = "cache-json",
    about = "Convert a girsync redb cache to NDJSON and back (current schema only)"
)]
pub struct Args {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, clap::Subcommand)]
pub enum Command {
    /// Write a cache as NDJSON (`-` means stdout).
    ToJson {
        /// The redb cache file, read-only.
        #[arg(value_name = "CACHE")]
        cache: PathBuf,
        /// Destination NDJSON file, or `-` for stdout.
        #[arg(value_name = "NDJSON", default_value = "-")]
        ndjson: PathBuf,
    },
    /// Build a cache from NDJSON (`-` means stdin).
    FromJson {
        /// Source NDJSON file, or `-` for stdin.
        #[arg(value_name = "NDJSON")]
        ndjson: PathBuf,
        /// Destination redb cache file.
        #[arg(value_name = "CACHE")]
        cache: PathBuf,
        /// Replace an existing cache file.
        #[arg(long)]
        force: bool,
    },
}
