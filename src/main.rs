//! Thin entry point: parse, install tracing, run the pipeline, print `SUMMARY`,
//! exit. All behavior lives in the library.

use clap::Parser;
use clap::error::ErrorKind;

use girpr::cli::Args;
use girpr::config::RunCtx;
use girpr::{logging, repair, report};

fn main() {
    let args = parse_args();
    // Logging first: everything after this point is diagnosable from the file.
    // Read off `args` before `into_ctx` consumes it — the flag layer is live
    // even for a run that is about to fail validation.
    let plan = logging::init_tracing(
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
    if let Some(path) = plan.named_file() {
        eprintln!("log file: {}", path.display());
    }

    let ctx = match args.into_ctx() {
        Ok(c) => c,
        Err(e) => {
            // Post-parse validation failures are usage errors, exit 1
            // (README / docs/02 §1).
            tracing::error!("usage error: {e:#}");
            eprintln!("ERROR {e:#}");
            std::process::exit(1);
        }
    };

    // The file tasks inside the run do blocking disk I/O on the async
    // runtime's workers. One worker beyond the file window keeps timers and
    // network polling able to run while every file task is waiting on the
    // disk, so raising `--io-threads` does not starve the runtime itself.
    let available = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    let workers = worker_threads(available, ctx.jobs);
    tracing::info!(
        worker_threads = workers,
        jobs = ctx.jobs,
        "runtime sized for requested file window"
    );

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(workers)
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            // A runtime that cannot start is an environment failure, not a
            // repair failure: exit 1 like any other usage-level error.
            eprintln!("ERROR failed to start runtime: {e:#}");
            std::process::exit(1);
        }
    };

    std::process::exit(runtime.block_on(run(ctx)));
}

/// Worker threads for a run that admits `jobs` files at once.
///
/// At least the host's parallelism, and always one more than the file window
/// so the runtime keeps a worker for timers and network progress. The addition
/// saturates: an absurd `jobs` must not overflow the thread count.
fn worker_threads(available: usize, jobs: usize) -> usize {
    available.max(jobs.saturating_add(1))
}

/// Run the pipeline and return the process exit code.
///
/// The caller builds the runtime first, so nothing here decides *how* the
/// pipeline is scheduled.
async fn run(ctx: RunCtx) -> i32 {
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

    match repair::run(ctx).await {
        Ok((summary, code)) => {
            // stdout for parsing + log so a stderr-only capture still keeps it.
            report::emit(&report::summary_record(&summary, code), output);
            code
        }
        Err(f) => {
            tracing::error!("fatal(exit={}): {:#}", f.exit_code, f.source);
            eprintln!("FATAL {:#}", f.source);
            f.exit_code
        }
    }
}

/// Parse the command line, mapping every argument-level failure to exit `1`.
///
/// `clap`'s own `parse()` exits `2` on a bad flag, which would collide with the
/// documented "2 = metadata/network" code; only `--help` / `--version` keep the
/// conventional `0`. clap prints the message (and the usage block) either way.
/// This runs before `init_tracing`, so a usage error creates no log file.
fn parse_args() -> Args {
    match Args::try_parse() {
        Ok(a) => a,
        Err(e) => {
            let _ = e.print();
            std::process::exit(match e.kind() {
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => 0,
                _ => 1,
            });
        }
    }
}
