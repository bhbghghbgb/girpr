//! Thin entry point: parse, install tracing, dispatch, print `SUMMARY`, exit.
//!
//! All behaviour lives in the library. What is here is the part that cannot be:
//! clap's exit codes are remapped so a flag typo is `1` rather than `2`, logging
//! is installed before any work so a failure is diagnosable afterwards, and the
//! process ends with the one code the run earned.

use gircap::cli::{Cmd, exit_on_clap_error, parse_from, resolve_common};
use gircap::config::{capture_plan, materialize_plan, verify_plan};
use gircap::report::OutputFormat;
use gircap::{exit, logging, run};

#[tokio::main]
async fn main() {
    let cli = match parse_from(std::env::args_os()) {
        Ok(c) => c,
        Err(e) => exit_on_clap_error(e),
    };

    // Logging first: everything after this point is diagnosable from the file.
    // A usage error below is reported through `tracing` *and* stderr, because the
    // message is the only thing the caller has.
    let (event, common) = match &cli.cmd {
        Cmd::Capture(a) => ("capture", a.common.clone()),
        Cmd::Verify(a) => ("verify", a.common.clone()),
        Cmd::Materialize(a) => ("materialize", a.common.clone()),
        Cmd::List(a) => ("list", a.common.clone()),
    };
    let common = match resolve_common(&common) {
        Ok(c) => c,
        Err(e) => {
            // The earliest possible failure, and therefore the only one that
            // cannot honour `--output`: the flag itself is what failed to
            // resolve. Emitting the text summary anyway is what keeps the
            // promise that every run prints exactly one, which is what lets a
            // harness read stdout without special-casing a crash.
            tracing::error!(error = %e, "usage error");
            run::failure_summary(event, e.exit_code()).emit(OutputFormat::Text);
            exit::exit_with(e.exit_code())
        }
    };
    let mut log_guard = logging::init(common.level, common.log_file.as_deref());

    let outcome = match &cli.cmd {
        Cmd::Capture(a) => match capture_plan(a, std::env::args().collect()) {
            Ok(plan) => run::capture(plan).await,
            Err(e) => {
                tracing::error!(error = %e, "usage error");
                Err(run::Failure::usage(&e))
            }
        },
        Cmd::Verify(a) => match verify_plan(a) {
            Ok(plan) => run::verify(plan).await,
            Err(e) => {
                tracing::error!(error = %e, "usage error");
                Err(run::Failure::usage(&e))
            }
        },
        Cmd::Materialize(a) => match materialize_plan(a) {
            Ok(plan) => run::materialize(plan).await,
            Err(e) => {
                tracing::error!(error = %e, "usage error");
                Err(run::Failure::usage(&e))
            }
        },
        Cmd::List(a) => run::list(a.root.clone(), common.output).await,
    };

    // Flush the JSON-lines log before the summary prints, so a reader tailing the
    // file sees the run's own narration and not a prefix of it.
    log_guard.finish();

    let (summary, code) = match outcome {
        Ok(outcome) => outcome,
        Err(f) => (f.summary(event), f.code),
    };
    summary.emit(common.output);

    exit::exit_with(code);
}