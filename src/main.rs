//! Thin entry point: parse, install tracing, run the pipeline, print `SUMMARY`,
//! flush the log, return the exit code. All behavior lives in the library.

use std::process::ExitCode;

use clap::Parser;

use girpr::{
    Args, RunCtx, RunExit, flush_log, init_tracing, log_file_announcement, run as repair_run,
};
// The binary is a direct consumer of the report plane, not just of the facade's
// re-export: an edit to repair leaves this crate and its tests cached.
use girpr_report::{emit, summary_record};

fn main() -> ExitCode {
    let args = parse_args();
    // Logging first: everything after this point is diagnosable from the file.
    // Read off `args` before `into_ctx` consumes it — the flag layer is live
    // even for a run that is about to fail validation.
    let plan = init_tracing(
        args.log_level,
        args.log_file_level,
        args.log_file.as_deref(),
        args.no_log_file,
    );
    // `init_tracing` already announced the path into the file. But the caller who
    // passed `--log-file` (or `GIRPR_LOG_FILE`) asked for *this* file and never
    // saw the timestamped default, so acknowledge it outside the tracing layers:
    // a `tracing::info!` would be filtered out of stderr by the very
    // `--log-level` the caller chose, and an unfiltered run means a caller who
    // lost the log can still find it.
    if let Some(line) = log_file_announcement(&plan) {
        eprintln!("{line}");
    }

    let ctx = match args.into_ctx() {
        Ok(c) => c,
        Err(e) => {
            // Post-parse validation failures are usage errors
            // (`RunExit::Usage`, README / docs/02 §1).
            tracing::error!("usage error: {e:#}");
            eprintln!("ERROR {e:#}");
            flush_log();
            return exit_code(RunExit::Usage);
        }
    };

    // Repair file tasks do their synchronous disk I/O on the blocking pool,
    // so async workers are free for timers and network polling no matter how
    // many files are in flight. Size the runtime from the host alone; `--jobs`
    // bounds the file window, not the worker set.
    let available = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    let workers = worker_threads(available);
    tracing::info!(
        worker_threads = workers,
        jobs = ctx.jobs,
        "runtime sized independently of the file window"
    );

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(workers)
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            // A runtime that cannot start is an environment failure, not a
            // repair failure: `RunExit::Usage` like any other usage-level error.
            eprintln!("ERROR failed to start runtime: {e:#}");
            flush_log();
            return exit_code(RunExit::Usage);
        }
    };

    let code = runtime.block_on(run(ctx));
    flush_log();
    exit_code(code)
}

/// Map a documented exit status to what the process returns.
///
/// [`RunExit`] is the pipeline's only exit type; this is where it becomes the
/// process status. `#[repr(u8)]` makes the byte conversion infallible, so a
/// documented variant cannot wrap into an unrelated status.
fn exit_code(status: RunExit) -> ExitCode {
    ExitCode::from(status.process_status())
}

/// Worker threads for the async runtime.
///
/// A function of host parallelism only: repair file work runs on the blocking
/// pool, so the worker set never scales with the `--jobs` file window. Keep a
/// small floor and one spare worker for the run's own orchestration, timers
/// and network polling. The addition saturates so a pathological host count
/// cannot overflow the thread count.
fn worker_threads(available: usize) -> usize {
    available.max(2).saturating_add(1)
}

/// Run the pipeline and return the process exit code.
///
/// The caller builds the runtime first, so nothing here decides *how* the
/// pipeline is scheduled.
async fn run(ctx: RunCtx) -> RunExit {
    // `output` is logged with the rest of the run's shape because every data-plane
    // line in the file after this one is rendered in it. A log file with no
    // record of the plane is a log file whose `SUMMARY` line cannot be parsed
    // by the same reader that parses stdout.
    let output = ctx.output;
    tracing::info!(
        "girpr start game_path={} biz={} jobs={} purge_after={} purge_before={} check_only={} dry_run={} output={}",
        ctx.game_dir.display(),
        ctx.biz.as_str(),
        ctx.jobs,
        ctx.purge_after,
        ctx.purge_before,
        ctx.check_only,
        ctx.dry_run,
        output
    );

    match repair_run(ctx).await {
        Ok((summary, code)) => {
            // The run's last record. This is the only `SUMMARY` call site: a
            // phase never emits it, so every non-fatal return path ends with
            // exactly one. A fatal `Err` below prints `FATAL` and no `SUMMARY`.
            // stdout for parsing + log so a stderr-only capture still keeps it.
            emit(&summary_record(&summary, code.code()), output);
            code
        }
        Err(f) => {
            tracing::error!("fatal(exit={}): {:#}", f.exit_code.code(), f.source);
            eprintln!("FATAL {:#}", f.source);
            RunExit::from(f)
        }
    }
}

/// Parse the command line, mapping every argument-level failure to
/// [`RunExit::Usage`].
///
/// `clap`'s own `parse()` exits `2` on a bad flag, which would collide with
/// [`RunExit::Metadata`]; only `--help` / `--version` keep the conventional
/// `0`. The shared `girpr_cli::clap_error_code` decides that number from the
/// error kind and this binary's own usage status; clap prints the message (and
/// the usage block) either way. This runs before `init_tracing`, so a usage
/// error creates no log file and there is no non-blocking worker to drain:
/// `process::exit` skipping
/// destructors is harmless here. Every path that does install logging flushes
/// the worker through `flush_log` before returning (see below), so the explicit
/// seam covers all exits that have a log tail to lose.
fn parse_args() -> Args {
    match Args::try_parse() {
        Ok(a) => a,
        Err(e) => {
            let _ = e.print();
            std::process::exit(i32::from(girpr_cli::clap_error_code(
                e.kind(),
                RunExit::Usage.process_status(),
            )));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{exit_code, worker_threads};
    use girpr::RunExit;
    use std::process::ExitCode;

    #[test]
    fn worker_threads_arithmetic() {
        // (host parallelism, expected worker count). A small floor keeps at
        // least two workers on a single-CPU host; the spare one is for timers
        // and network polling. A pathological host count saturates, no overflow.
        for (available, expected) in [(1usize, 3usize), (2, 3), (8, 9), (usize::MAX, usize::MAX)] {
            assert_eq!(
                worker_threads(available),
                expected,
                "worker_threads({available})"
            );
        }
    }

    #[test]
    fn the_runtime_is_sized_by_the_host_not_the_file_window() {
        // The worker count is a function of host parallelism alone, with a
        // small fixed floor and one spare — never the `--jobs` file window.
        for (available, expected) in [(1usize, 3usize), (4, 5), (64, 65)] {
            assert_eq!(worker_threads(available), expected);
        }
        // Stable across calls: no hidden dependence on mutable state.
        assert_eq!(worker_threads(4), worker_threads(4));
    }

    /// The documented exit statuses, in numeric order.
    ///
    /// `RunExit`'s exhaustive list is crate-internal, so this binary target,
    /// unlike the crate's own unit tests, cannot read it and names the
    /// statuses itself. The wildcard-free `match` is generated from the same
    /// list: a new `RunExit` variant stops the build here until it is added,
    /// and the numeric pin in the test then holds it to its documented number.
    /// That makes the duplicate list deliberate — only a `pub` item crosses
    /// the target boundary, and widening the API for one test list is not
    /// worth it.
    ///
    /// The type is spelled `RunExit` because it is the only enum with this
    /// process-status guard. A second one would make the type a parameter
    /// (`$ty:ident`, used as `fn($ty)` and `$ty::$variant`) so the call site
    /// names the enum it pins.
    macro_rules! documented_exit_codes {
        ($($variant:ident),+ $(,)?) => {{
            let _: fn(RunExit) = |status| match status {
                $(RunExit::$variant => {}),+
            };
            [$(RunExit::$variant),+]
        }};
    }

    #[test]
    fn the_documented_exit_codes_reach_the_process_unchanged() {
        let codes = documented_exit_codes!(Success, Usage, Metadata, Write, CheckFailed, Limit,);
        for (number, status) in codes.into_iter().enumerate() {
            assert_eq!(
                exit_code(status),
                ExitCode::from(number as u8),
                "{status:?}"
            );
        }
    }
}
