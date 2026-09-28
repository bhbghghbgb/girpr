//! Thin entry point: parse, install tracing, run the pipeline, print `SUMMARY`,
//! exit. All behavior lives in the library.

use clap::error::ErrorKind;
use clap::Parser;

use girpr::cli::Args;
use girpr::{logging, report, repair};

#[tokio::main]
async fn main() {
    let args = parse_args();
    // Logging first: everything after this point is diagnosable from the file.
    logging::init_tracing(&args.log_level);

    let json_summary = args.json_summary;
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
    tracing::info!(
        "girpr start game_path={} biz={} jobs={} purge_after={} purge_before={} check_only={} dry_run={}",
        ctx.game_dir.display(),
        ctx.biz.as_str(),
        ctx.jobs,
        ctx.purge_after,
        ctx.purge_before,
        ctx.check_only,
        ctx.dry_run
    );

    match repair::run(ctx).await {
        Ok((summary, code)) => {
            // stdout for parsing + log so a stderr-only capture still keeps it.
            report::emit(&report::format_summary_line(&summary, code, json_summary));
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
