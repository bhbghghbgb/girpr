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
//! The argument struct and the conversion record live in the library, not
//! here, so a test can parse a spelling and render the record in-process.
//! Why the struct carries no doc comment is documented there.
//!
//! Reports through the same [`girsync::report`] writer as `girsync` itself, so
//! `--output json` means the same thing in both binaries and there is one
//! definition of a conversion record rather than a `println!` per tool.

use clap::Parser;
use sled2redb::Args;

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
            // binary has one code for every way it can fail. The shared leaf
            // decides only the number; this binary still calls `e.exit()` for
            // the answers and prints + exits with its own literal 3 otherwise.
            let code = girpr_cli::clap_error_code(e.kind(), 3);
            if code == 0 {
                e.exit();
            }
            // The message is written to stderr, where clap puts a usage error.
            // A failed write is not worth a second failure: the run is already
            // on its way out with the code that says what happened.
            let _ = e.print();
            std::process::exit(i32::from(code));
        }
    };
    // Tracing is installed up front so a conversion failure is a structured
    // `error!` on stderr like every other fatal in the crate, rather than a bare
    // line this binary formats itself. The log file is written when it is asked
    // for, exactly as it is for `girsync` itself.
    let _guard = girsync::init_tracing(girsync::logging::Level::Info, None, None, false);
    match sled2redb::convert::sled_to_redb(&args.sled_dir, &args.redb_path, args.force) {
        Ok(c) => {
            sled2redb::conversion_record(&c, &args.sled_dir, &args.redb_path).emit(args.output)
        }
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
