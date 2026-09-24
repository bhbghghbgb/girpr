use clap::Parser;
use girpr::{normalize_audio_lang, repair, Args};
use std::collections::HashSet;
use std::sync::atomic::Ordering;

#[tokio::main]
async fn main() {
    let args = Args::parse();
    let filter = format!(
        "girpr={},{}",
        args.log_level,
        std::env::var("RUST_LOG").unwrap_or_default()
    );
    tracing_subscriber::fmt()
        .with_env_filter(
            filter
                .parse::<tracing_subscriber::EnvFilter>()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();

    if args.io_threads == 0 {
        eprintln!("ERROR io-threads must be >= 1");
        std::process::exit(1);
    }
    let mut audio = HashSet::new();
    for a in &args.audio {
        match normalize_audio_lang(a) {
            Some(n) => {
                audio.insert(n);
            }
            None => {
                eprintln!("ERROR unknown audio lang '{}' (want zh-cn|en-us|ja-jp|ko-kr)", a);
                std::process::exit(1);
            }
        }
    }

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
