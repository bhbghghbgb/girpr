use clap::Parser;
use girsync::{Cli, init_tracing, run};
use tracing::error;

/// Exit codes are 0, 3 and 4 — see `commands::run` and `report_diff`.
///
/// `ExitCode` rather than `process::exit`, because the latter **skips
/// destructors** and the log writer is a `tracing_appender` non-blocking queue
/// whose `WorkerGuard` flushes on drop. An explicit exit therefore loses the tail
/// of `--log-file`, and the lines most likely to be in that tail are the `fatal`
/// event written a statement earlier, which is the one line the file exists to
/// keep.
fn main() -> std::process::ExitCode {
    let cli = Cli::parse();
    // Two failure paths, and they are deliberately different. A failure to
    // install the subscriber happens *before* there is one, so there is nothing
    // structured to write to and the message goes straight to stderr. Once
    // installed, the error is an `error!` event with the error in a field: the
    // console renders it at every `--log-level` (error clears all of them) and a
    // `--log-file` keeps it as JSON. Printing it a second time by hand would put
    // the same fatal on stderr twice, once structured and once not.
    let _log_guard = match init_tracing(&cli.log_level, cli.log_file.as_deref()) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("FATAL {:#}", e);
            return std::process::ExitCode::from(3);
        }
    };
    let code = match run(cli) {
        Ok(code) => code,
        Err(e) => {
            error!(error = format!("{:#}", e), "fatal");
            3
        }
    };
    // `_log_guard` drops here, flushing the file before the process ends.
    std::process::ExitCode::from(u8::try_from(code).unwrap_or(1))
}
