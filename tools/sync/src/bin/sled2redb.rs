//! Convert a legacy sled cache directory into a redb cache file.
//!
//! Usage: `sled2redb <SLED_DIR> <REDB_PATH> [--force] [--output text|json]`
//! Exit `0` on success, `3` on failure (matches girsync's fatal code) — and a
//! usage error is a failure too, so this binary has one code for every way it
//! can fail.
//!
//! **The flags go on either side of the two paths.** `--force` and `--output`
//! are accepted before the paths and after them, and `--output=json` is the
//! same as `--output json`: the parser is `clap`'s, so its spelling is the
//! spelling, and no narrower set of forms sits behind it.
//!
//! **There is no `--version`.** The command declares no version, so the flag
//! is an unknown one and the run fails with `3` like any other usage error.
//! `#[command(version)]` on the argument struct is what would add it — and the
//! error path in `main` already treats a version request as an answer rather
//! than a failure, so the flag would exit `0` the moment it was declared.
//!
//! **The argument struct carries no doc comment, on purpose.** `clap` reads
//! one as the command's `long_about`, so `--help` prints it — verbatim, the
//! `**` markers and the `#[command(version)]` attribute included, because
//! `clap` does not render markdown. A note written for a source reader would
//! then tell a user about an attribute they cannot type. The notes above are
//! for source readers, so they live here.
//!
//! Reports through the same [`girsync::report`] writer as `girsync` itself, so
//! `--output json` means the same thing in both binaries and there is one
//! definition of a conversion record rather than a `println!` per tool.

use clap::Parser;
use girsync::report::{OutputFormat, Record};
use std::path::PathBuf;

#[derive(clap::Parser, Debug)]
#[command(
    name = "sled2redb",
    about = "Convert a legacy sled cache directory into a redb cache file"
)]
struct Args {
    /// Legacy sled cache directory.
    sled_dir: PathBuf,
    /// Destination redb file.
    redb_path: PathBuf,
    /// Replace an existing destination.
    #[arg(long)]
    force: bool,
    /// How the conversion record is rendered.
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    output: OutputFormat,
}

fn main() {
    let args = match Args::try_parse() {
        Ok(args) => args,
        Err(e) => {
            // `--help` is an answer, not a failure: clap prints it and exits 0.
            // A version request is the same kind of answer, and the command
            // declares no version today — so that arm is there for the day one
            // is declared, and it exits 0 then too.
            // Every other error is a usage error, and a usage error here is a
            // failure like any other, so it exits 3 rather than clap's 2 — this
            // binary has one code for every way it can fail.
            if matches!(
                e.kind(),
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
            ) {
                e.exit();
            }
            // The message is written to stderr, where clap puts a usage error.
            // A failed write is not worth a second failure: the run is already
            // on its way out with the code that says what happened.
            let _ = e.print();
            std::process::exit(3);
        }
    };
    // Tracing is installed up front so a conversion failure is a structured
    // `error!` on stderr like every other fatal in the crate, rather than a bare
    // line this binary formats itself. The log file is written when it is asked
    // for, exactly as it is for `girsync` itself.
    let _guard = girsync::init_tracing(girsync::logging::Level::Info, None, None, false);
    match girsync::convert::sled_to_redb(&args.sled_dir, &args.redb_path, args.force) {
        Ok(c) => Record::keyed("converted")
            .put("src", args.sled_dir.display().to_string())
            .put("dst", args.redb_path.display().to_string())
            .put("files", c.files)
            .put("dirs", c.dirs)
            .put("hashes", c.hashes)
            .emit(args.output),
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
