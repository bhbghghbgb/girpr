use clap::Parser;
use girsync::{Cli, init_tracing, run};
use tracing::error;

fn main() {
    let cli = Cli::parse();
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
            error!(error = format!("{:#}", e), "FATAL");
            eprintln!("FATAL {:#}", e);
            3
        }
    };
    std::process::exit(code);
}
