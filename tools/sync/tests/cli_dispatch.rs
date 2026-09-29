//! Full runs through `run(Cli)`, covering clap dispatch and log-option threading.

mod common;

use common::{TempRoot, rfile, wfile};
use girsync::cli::{Cli, Cmd, CommonArgs, TrustArgs};
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
        dry_run: false,
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

    let _ = t.root();
}
