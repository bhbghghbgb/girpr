use clap::Parser;
use girsync::{Cli, init_tracing, run};
use tracing::error;

fn main() {
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
            std::process::exit(3);
        }
    };
    let code = match run(cli) {
        Ok(code) => code,
        Err(e) => {
            error!(error = format!("{:#}", e), "fatal");
            3
        }
    };
    std::process::exit(code);
}
