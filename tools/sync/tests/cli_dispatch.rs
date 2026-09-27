//! Full runs through `run(Cli)`, covering clap dispatch and log-option threading.

mod common;

use common::{rfile, wfile, TempRoot};
use girsync::cli::{Cli, Cmd, CommonArgs};
use girsync::run;

#[test]
fn run_cli_dispatch_update_compare_sync() {
    let t = TempRoot::new("cli");
    let src = t.mkdirs("src");
    let dst = t.mkdirs("dst");
    wfile(&src, "a.txt", b"aaa");
    wfile(&dst, "a.txt", b"bbb");

    let common = || CommonArgs {
        hash: common::md5arg(),
        include: vec![],
        exclude: vec![],
        case_sensitive: true,
        max_depth: 10,
        ignore_cache: false,
    };
    let mkcli = |cmd| Cli {
        log_level: "error".to_string(),
        log_file: None,
        cmd,
    };

    let code = run(mkcli(Cmd::Update {
        dir: src.clone(),
        common: common(),
    }))
    .unwrap();
    assert_eq!(code, 0);

    let code = run(mkcli(Cmd::Compare {
        src: src.clone(),
        dst: dst.clone(),
        no_fast: false,
        common: common(),
    }))
    .unwrap();
    assert_eq!(code, 4);

    let code = run(mkcli(Cmd::Sync {
        src: src.clone(),
        dst: dst.clone(),
        no_fast: false,
        missing_only: false,
        keep_extra: false,
        dry_run: false,
        jobs: 1,
        common: common(),
    }))
    .unwrap();
    assert_eq!(code, 0);
    assert_eq!(rfile(&dst, "a.txt"), b"aaa");

    let code = run(mkcli(Cmd::Compare {
        src: src.clone(),
        dst: dst.clone(),
        no_fast: false,
        common: common(),
    }))
    .unwrap();
    assert_eq!(code, 0);

    let _ = t.root();
}
