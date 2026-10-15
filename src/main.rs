//! Thin entry point: parse, install tracing, run the pipeline, print `SUMMARY`,
//! exit. All behavior lives in the library.

use clap::Parser;
use clap::error::ErrorKind;

use girpr::cli::Args;
use girpr::{logging, repair, report};

#[tokio::main]
async fn main() {
    let args = parse_args();
    // Logging first: everything after this point is diagnosable from the file.
    // Read off `args` before `into_ctx` consumes it — the flag layer is live
    // even for a run that is about to fail validation.
    let log_file = args.log_file.clone();
    let log_path = logging::init_tracing(&args.log_level, log_file.as_deref());
    // `init_tracing` already announced the path into the file. But the caller who
    // passed `--log-file` asked for *this* file and never saw the timestamped
    // default, so acknowledge it outside the tracing layers: a `tracing::info!`
    // would be filtered out of stderr by the very `--log-level` the caller chose,
    // and an unfiltered run means a caller who lost the log can still find it.
    if log_file.is_some() {
        eprintln!("log file: {}", log_path.display());
    }

    let output = args.output;
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
    // `output` is logged with the rest of the run's shape because every data-plane
    // line in the file after this one is rendered in it. A log file with no
    // record of the plane is a log file whose `SUMMARY` line cannot be parsed
    // by the same reader that parses stdout.
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
            std::process::exit(code);
        }
        Err(f) => {
            tracing::error!("fatal(exit={}): {:#}", f.exit_code, f.source);
            eprintln!("FATAL {:#}", f.source);
            std::process::exit(f.exit_code);
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
