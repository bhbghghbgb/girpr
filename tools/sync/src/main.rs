use clap::Parser;
use girsync::{Cli, init_tracing, run};
use tracing::error;

/// Exit codes are 0, 3 and 4 — see `commands::run` and `report_diff`.
///
/// `ExitCode` rather than `process::exit`, because the latter **skips
/// destructors** and the log writer is a `tracing_appender` non-blocking queue
/// whose `WorkerGuard` flushes on drop. An explicit exit therefore loses the tail
/// of the log file, and the lines most likely to be in that tail are the `fatal`
/// event written a statement earlier, which is the one line the file exists to
/// keep.
fn main() -> std::process::ExitCode {
    let cli = Cli::parse();
    // The log file is written when it is asked for, and a file that cannot be
    // opened is a warning on stderr rather than a failure: a run must not end
    // differently because of where it was asked to log. Once the subscriber is
    // installed, an error is an `error!` event with the error in a field: the
    // console renders it at every `--log-level` (error clears all of them) and
    // the log file keeps it as JSON. Printing it a second time by hand would put
    // the same fatal on stderr twice, once structured and once not.
    let log_guard = init_tracing(
        cli.log_level,
        cli.log_file_level,
        cli.log_file.as_deref(),
        cli.no_log_file,
    );
    // The path is echoed on stderr when the caller named it — by flag or by
    // `GIRSYNC_LOG_FILE` — so a wrapper that captured only stdout can still
    // find the log. The default path is announced inside the file instead.
    if let Some(path) = log_guard.plan().named_file() {
        eprintln!("log file: {}", path.display());
    }
    let code = match run(cli, log_guard.plan()) {
        Ok(code) => code,
        Err(e) => {
            error!(error = format!("{:#}", e), "fatal");
            3
        }
    };
    // `log_guard` drops here, flushing the file before the process ends.
    std::process::ExitCode::from(u8::try_from(code).unwrap_or(1))
}
