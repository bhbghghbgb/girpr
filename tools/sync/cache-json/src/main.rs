//! `cache-json`: convert a girsync redb cache to NDJSON and back.
//!
//! Usage:
//! ```text
//! cache-json to-json <CACHE> [NDJSON]              # NDJSON defaults to stdout
//! cache-json from-json <NDJSON|-> <CACHE> [--force] # `-` means stdin
//! ```
//!
//! Exit `0` on success, `3` on failure — including a usage error. There is no
//! `--version`: the command declares none, so the flag is a usage failure like
//! any other.
//!
//! The NDJSON goes to stdout by default, so the summary goes to stderr and
//! stdout always stays parseable. All conversion logic lives in the library;
//! this file only opens the two endpoints and dispatches.

use anyhow::{Context, Result};
use clap::Parser;
use girsync_cache_json::{Args, Command, export_cache, import_cache};
use std::fs::File;
use std::io::{self, BufReader, BufWriter};
use std::path::Path;

fn main() {
    let args = match Args::try_parse() {
        Ok(args) => args,
        Err(e) => {
            // `--help` is an answer, not a failure: clap prints it and exits 0.
            // Every other error is a usage error, and this binary has one code
            // for every way it can fail, so it exits 3 rather than clap's 2.
            let code = girpr_cli::clap_error_code(e.kind(), 3);
            if code == 0 {
                e.exit();
            }
            let _ = e.print();
            std::process::exit(i32::from(code));
        }
    };
    if let Err(e) = run(args.command) {
        eprintln!("error: {e:#}");
        std::process::exit(3);
    }
}

fn run(command: Command) -> Result<()> {
    match command {
        Command::ToJson { cache, ndjson } => {
            if ndjson.as_path() == Path::new("-") {
                let stdout = io::stdout();
                let stats = export_cache(&cache, BufWriter::new(stdout.lock()))?;
                eprintln!(
                    "exported {} rows ({} files, {} dirs) from {} to stdout",
                    stats.entries,
                    stats.files,
                    stats.dirs,
                    cache.display()
                );
            } else {
                let file = File::create(&ndjson)
                    .with_context(|| format!("create NDJSON file {}", ndjson.display()))?;
                let stats = export_cache(&cache, BufWriter::new(file))?;
                eprintln!(
                    "exported {} rows ({} files, {} dirs) from {} to {}",
                    stats.entries,
                    stats.files,
                    stats.dirs,
                    cache.display(),
                    ndjson.display()
                );
            }
        }
        Command::FromJson {
            ndjson,
            cache,
            force,
        } => {
            let stats = if ndjson.as_path() == Path::new("-") {
                let stdin = io::stdin();
                import_cache(BufReader::new(stdin.lock()), &cache, force)?
            } else {
                let file = File::open(&ndjson)
                    .with_context(|| format!("open NDJSON file {}", ndjson.display()))?;
                import_cache(BufReader::new(file), &cache, force)?
            };
            eprintln!(
                "imported {} rows ({} files, {} dirs) into {}",
                stats.entries,
                stats.files,
                stats.dirs,
                cache.display()
            );
        }
    }
    Ok(())
}
