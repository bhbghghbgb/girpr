use clap::Parser;
use girpr::{is_audio_none, normalize_audio_lang, repair, Args};
use std::collections::HashSet;
use std::fs::File;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Mutex;
use tracing_subscriber::prelude::*;

fn open_log_file(path: &Path) -> std::io::Result<File> {
    let dir = path
        .parent()
        .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound))?;
    std::fs::create_dir_all(dir)?;
    std::fs::OpenOptions::new().create(true).append(true).open(path)
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    let rust_log = std::env::var("RUST_LOG").unwrap_or_default();
    let rust_log = rust_log.trim();
    let filter = if rust_log.is_empty() {
        format!("girpr={}", args.log_level)
    } else {
        format!("girpr={},{}", args.log_level, rust_log)
    };
    let stderr_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_filter(
            filter
                .parse::<tracing_subscriber::EnvFilter>()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        );

    let log_path = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| {
            std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
        })
        .join("logs")
        .join(format!(
            "girpr_{}.log",
            chrono::Local::now().format("%Y-%m-%d_%H-%M-%S%.3f")
        ));
    let file_writer: Mutex<Box<dyn std::io::Write + Send + Sync>> = match open_log_file(&log_path) {
        Ok(f) => Mutex::new(Box::new(f)),
        Err(e) => {
            eprintln!("WARN disabling file logging ({}): {e}", log_path.display());
            Mutex::new(Box::new(std::io::sink()))
        }
    };
    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(file_writer)
        .with_ansi(false)
        .with_filter(tracing_subscriber::filter::LevelFilter::TRACE);

    tracing_subscriber::registry()
        .with(stderr_layer)
        .with(file_layer)
        .init();

    tracing::info!("log file: {}", log_path.display());

    if args.io_threads == 0 {
        eprintln!("ERROR io-threads must be >= 1");
        std::process::exit(1);
    }
    let mut audio = HashSet::new();
    let mut saw_none = false;
    for a in &args.audio {
        if is_audio_none(a) {
            saw_none = true;
            continue;
        }
        match normalize_audio_lang(a) {
            Some(n) => {
                audio.insert(n);
            }
            None => {
                eprintln!(
                    "ERROR unknown audio lang '{}' (want zh-cn|en-us|ja-jp|ko-kr|none)",
                    a
                );
                std::process::exit(1);
            }
        }
    }
    if saw_none && !audio.is_empty() {
        eprintln!("ERROR --audio none cannot be mixed with language codes");
        std::process::exit(1);
    }
    // No `--audio` at all -> autodetect (keep scan file, else en-us).
    // `--audio none` alone -> explicit game-only (empty set, overwrites scan file).
    let audio_explicit = !args.audio.is_empty();

    tracing::info!(
        "girpr start game_path={} biz={} jobs={} purge_after={} purge_before={} check_only={} dry_run={}",
        args.game_path.display(),
        args.biz.as_str(),
        args.io_threads,
        args.purge_after,
        args.purge_before,
        args.check_only,
        args.dry_run
    );

    let ctx = repair::RepairCtx {
        game_dir: args.game_path.clone(),
        biz: args.biz,
        audio,
        audio_explicit,
        jobs: args.io_threads,
        check_only: args.check_only,
        dry_run: args.dry_run,
        purge_after: args.purge_after,
        purge_before: args.purge_before,
        json_summary: args.json_summary,
    };
    match repair::run(ctx).await {
        Ok((summary, code)) => {
            let dl = summary.download_bytes.load(Ordering::Relaxed);
            let del = summary.deleted_extra_bytes.load(Ordering::Relaxed);
            let skipped = summary.files_skipped.load(Ordering::Relaxed);
            let repaired = summary.files_repaired.load(Ordering::Relaxed);
            let failed = summary.files_failed.load(Ordering::Relaxed);
            if args.json_summary {
                let line = format!(
                    "{{\"total\":{},\"skipped\":{},\"repaired\":{},\"failed\":{},\"download_bytes\":{},\"deleted_extra_bytes\":{},\"exit\":{}}}",
                    summary.files_total,
                    skipped,
                    repaired,
                    failed,
                    dl,
                    del,
                    code
                );
                // stdout for parsing + log so a stderr-only capture still keeps it.
                println!("{line}");
                tracing::info!("{line}");
            } else {
                let line = format!(
                    "SUMMARY total={} skipped={} repaired={} failed={} download_bytes={} deleted_extra_bytes={} exit={}",
                    summary.files_total,
                    skipped,
                    repaired,
                    failed,
                    dl,
                    del,
                    code
                );
                // stdout for parsing + log so a stderr-only capture still keeps it.
                println!("{line}");
                tracing::info!("{line}");
            }
            std::process::exit(code);
        }
        Err(f) => {
            tracing::error!("fatal(exit={}): {:#}", f.exit_code, f.source);
            eprintln!("FATAL {:#}", f.source);
            std::process::exit(f.exit_code);
        }
    }
}
