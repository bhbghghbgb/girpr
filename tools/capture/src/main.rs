//! Thin entry point: parse, install tracing, dispatch, print `SUMMARY`, exit.
//!
//! All behaviour lives in the library. What is here is the part that cannot be:
//! clap's exit codes are remapped so a flag typo is `1` rather than `2`, logging
//! is installed before any work so a failure is diagnosable afterwards, and the
//! process ends with the one code the run earned.

use gircap::cli::{Cmd, parse_from, resolve_common};
use gircap::config::{capture_plan, materialize_plan, verify_plan};
use gircap::exit::ExitCode;
use gircap::{exit, logging, run};

#[tokio::main]
async fn main() {
    // The shared shim prints clap's message and maps the failure with gircap's
    // own usage code. `exit_with` stays here: it is the process boundary that
    // flushes stdout, and only this program owns it.
    let cli = girpr_cli::parse_or_report_error(
        parse_from(std::env::args_os()),
        ExitCode::Usage.code() as u8,
        |code| {
            exit::exit_with(if code == 0 {
                ExitCode::Ok
            } else {
                ExitCode::Usage
            })
        },
    );

    // Logging first: everything after this point is diagnosable from the file.
    let (event, common) = match &cli.cmd {
        Cmd::Capture(a) => ("capture", &a.common),
        Cmd::Verify(a) => ("verify", &a.common),
        Cmd::Materialize(a) => ("materialize", &a.common),
        Cmd::List(a) => ("list", &a.common),
    };
    let common = resolve_common(common);
    let mut log_guard = logging::init(
        common.level,
        common.file_level,
        common.log_file.as_deref(),
        common.no_log_file,
    );
    // The path is echoed on stderr when the caller named it — by flag or by
    // `GIRCAP_LOG_FILE` — so a wrapper that captured only stdout can still
    // find the log. The default path is announced inside the file instead.
    if let Some(line) = girpr_cli::log_file_announcement(log_guard.plan()) {
        eprintln!("{line}");
    }

    // Resolving the plan is where flag validation happens, so a rejected request
    // is reported here rather than inside each command — one place, three calls.
    let outcome = match &cli.cmd {
        Cmd::Capture(a) => {
            let argv = gircap::cli::recorded_argv(std::env::args_os());
            match capture_plan(a, argv) {
                Ok(plan) => run::capture(plan).await,
                Err(e) => Err(run::usage("capture", e.message())),
            }
        }
        Cmd::Verify(a) => match verify_plan(a) {
            Ok(plan) => run::verify(plan).await,
            Err(e) => Err(run::usage("verify", e.message())),
        },
        Cmd::Materialize(a) => match materialize_plan(a) {
            Ok(plan) => run::materialize(plan).await,
            Err(e) => Err(run::usage("materialize", e.message())),
        },
        Cmd::List(a) => run::list(a.root.clone(), common.output).await,
    };

    let (summary, code) = match outcome {
        Ok(outcome) => outcome,
        Err(mut f) => {
            // A run that failed *after* measuring still has numbers worth
            // printing, and they were built before the failure was known. They
            // go out first, so stdout reads chronologically and the failure
            // summary is still last — which is what a harness tailing the line
            // for `exit_code` expects.
            if let Some(pre) = f.preceded_by.take() {
                pre.emit(common.output);
            }
            (f.summary(event), f.code)
        }
    };
    summary.emit(common.output);

    // Emitting a record also logs it. Flush only now, after both a failure's own
    // numbers (if any) and the terminal SUMMARY have been logged, so the file
    // ends with the run's verdict. `exit_with` terminates the process, so this
    // must stay ahead of it.
    log_guard.finish();

    exit::exit_with(code);
}
