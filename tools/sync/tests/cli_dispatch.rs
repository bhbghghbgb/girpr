//! Full runs through `run(Cli)`, covering clap dispatch and log-option threading.

mod common;

use common::{TempRoot, rfile, wfile};
use girsync::cli::{Cli, Cmd, CommonArgs, TrustArgs};
use girsync::report::OutputFormat;
use girsync::run;

#[test]
fn run_cli_dispatch_update_compare_sync() {
    let t = TempRoot::new("cli");
    let src = t.mkdirs("src");
    let dst = t.mkdirs("dst");
    wfile(&src, "a.txt", b"aaa");
    wfile(&dst, "a.txt", b"bbb");

    let common = || CommonArgs {
        hash_all_of: common::md5arg(),
        hash_any_of: vec![],
        no_trust_size: false,
        no_trust_mtime: false,
        why: false,
        include: vec![],
        exclude: vec![],
        case_sensitive: true,
        max_depth: 10,
        ignore_cache: false,
        dry_run: false,
    };
    let mkcli = |cmd| Cli {
        log_level: "error".to_string(),
        log_file: None,
        output: OutputFormat::Text,
        cmd,
    };

    let code = run(mkcli(Cmd::Update {
        dir: src.clone(),
        common: common(),
    }))
    .unwrap();
    assert_eq!(code, 0);

    // The update above recorded src as it was. One file added to each side
    // afterwards, so `compare` and `compare-self` each have drift of their own
    // to find: b.txt is only on disk, c.txt is on disk but not in src's cache.
    wfile(&src, "c.txt", b"ccc");
    wfile(&dst, "b.txt", b"bbb");

    let code = run(mkcli(Cmd::CompareSelf {
        dir: src.clone(),
        no_trust_cached_hashes: false,
        common: common(),
    }))
    .unwrap();
    // Must run *before* the compare below: that one populates src's cache as
    // it scans, which is exactly what compare-self is here to avoid doing.
    assert_eq!(code, 4, "a file on disk that src's cache never recorded");

    let code = run(mkcli(Cmd::Compare {
        src: src.clone(),
        dst: dst.clone(),
        trust: TrustArgs::default(),
        common: common(),
    }))
    .unwrap();
    assert_eq!(code, 4);

    let code = run(mkcli(Cmd::Sync {
        src: src.clone(),
        dst: dst.clone(),
        missing_only: false,
        keep_extra: false,
        jobs: 1,
        trust: TrustArgs::default(),
        common: common(),
    }))
    .unwrap();
    assert_eq!(code, 0);
    assert_eq!(rfile(&dst, "a.txt"), b"aaa");

    let code = run(mkcli(Cmd::Compare {
        src: src.clone(),
        dst: dst.clone(),
        trust: TrustArgs::default(),
        common: common(),
    }))
    .unwrap();
    assert_eq!(code, 0);

    // `--output` is a global flag rather than a per-command one, so it reaches
    // every subcommand through the same `Cli` -> `LogCtx` path. Dispatching with
    // it set must not change any verdict; it only changes how the report renders.
    let json_cli = |cmd| Cli {
        output: OutputFormat::Json,
        ..mkcli(cmd)
    };
    wfile(&src, "later.txt", b"added after the first sync");
    assert_eq!(
        run(json_cli(Cmd::CompareSelf {
            dir: src.clone(),
            no_trust_cached_hashes: false,
            common: common(),
        }))
        .unwrap(),
        4,
        "a JSON run reaches the same verdict"
    );
    assert_eq!(
        run(json_cli(Cmd::Sync {
            src: src.clone(),
            dst: dst.clone(),
            missing_only: false,
            keep_extra: false,
            jobs: 1,
            trust: TrustArgs::default(),
            common: common(),
        }))
        .unwrap(),
        0
    );
    assert_eq!(rfile(&dst, "later.txt"), b"added after the first sync");

    let _ = t.root();
}
