//! Convert a legacy sled cache directory into a redb cache file.
//!
//! Usage: `sled2redb <sled-dir> <redb-file> [--force] [--output text|json]`
//! Exit `0` on success, `3` on failure (matches girsync's fatal code).
//!
//! Reports through the same [`girsync::report`] writer as `girsync` itself, so
//! `--output json` means the same thing in both binaries and there is one
//! definition of a conversion record rather than a `println!` per tool.

use girsync::report::{OutputFormat, Record};
use std::path::PathBuf;

fn usage() -> ! {
    eprintln!("usage: sled2redb <sled-dir> <redb-file> [--force] [--output text|json]");
    std::process::exit(3);
}

fn parse_output(s: &str) -> OutputFormat {
    match s {
        "text" => OutputFormat::Text,
        "json" => OutputFormat::Json,
        other => {
            eprintln!("invalid --output '{other}' (expected text|json)");
            usage()
        }
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let sled_dir: PathBuf = match args.next() {
        Some(a) if !a.starts_with("--") => a.into(),
        _ => usage(),
    };
    let redb_path: PathBuf = match args.next() {
        Some(a) if !a.starts_with("--") => a.into(),
        _ => usage(),
    };
    let mut force = false;
    let mut output = OutputFormat::default();
    let mut rest = args;
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--force" => force = true,
            "--output" => match rest.next() {
                Some(v) => output = parse_output(&v),
                None => usage(),
            },
            _ => usage(),
        }
    }
    // Tracing is installed up front so a conversion failure is a structured
    // `error!` on stderr like every other fatal in the crate, rather than a bare
    // line this binary formats itself.
    let _guard = match girsync::init_tracing("info", None) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("FATAL {:#}", e);
            std::process::exit(3);
        }
    };
    match girsync::convert::sled_to_redb(&sled_dir, &redb_path, force) {
        Ok(c) => Record::keyed("converted")
            .put("src", sled_dir.display().to_string())
            .put("dst", redb_path.display().to_string())
            .put("files", c.files)
            .put("dirs", c.dirs)
            .put("hashes", c.hashes)
            .emit(output),
        Err(e) => {
            // `target: "girsync"` on purpose. `init_tracing` filters the console and
            // the log file to the `girsync` target so third-party crates stay quiet,
            // but this binary's *crate* is named `sled2redb` — without the explicit
            // target the fatal would be filtered out of both and the run would fail
            // silently. It is girsync's event, so it says so.
            tracing::error!(target: "girsync", error = format!("{:#}", e), "fatal");
            std::process::exit(3);
        }
    }
}
