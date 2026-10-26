//! Full runs through `run(Cli)`, covering clap dispatch and log-option threading.

mod common;

use common::{rfile, temp_root, wfile};
use girsync::cli::{Cli, Cmd, CommonArgs, RwArgs, TrustArgs};
use girsync::logging::Level;
use girsync::report::OutputFormat;
use girsync::run;

#[test]
fn run_cli_dispatch_update_compare_sync() {
    // The whole dispatch is in-process, so one shared memory registry stands in
    // for the files a sequence of commands would leave behind. Each test is its
    // own process under nextest, so the switch cannot leak to another test.
    unsafe { std::env::set_var("GIRSYNC_TEST_CACHE", "memory") };
    let t = temp_root("cli");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    wfile(&src, "a.txt", b"aaa");
    wfile(&dst, "a.txt", b"bbb");

    let common = || CommonArgs {
        hash_all_of: common::md5arg()
            .iter()
            .map(|a| a.as_str().to_string())
            .collect(),
        hash_any_of: vec![],
        no_trust_size: false,
        no_trust_mtime: false,
        why: false,
        show_identical: false,
        include: vec![],
        exclude: vec![],
        case_sensitive: true,
        max_depth: 10,
        ignore_cache: false,
        dry_run: false,
        rw: RwArgs::default(),
    };
    let mkcli = |cmd| Cli {
        log_level: Level::Error,
        log_file_level: None,
        log_file: None,
        no_log_file: false,
        output: OutputFormat::Text,
        cmd,
    };
    // The log policy the runs below are dispatched under. Resolved once, the
    // way `main` does it, so the spans name the file that is really being
    // written.
    let plan = girsync::plan(Level::Error, None, None, false);

    let code = run(
        mkcli(Cmd::Update {
            dir: src.clone(),
            common: common(),
        }),
        &plan,
    )
    .unwrap();
    assert_eq!(code, 0);

    // The update above recorded src as it was. One file added to each side
    // afterwards, so `compare` and `compare-self` each have drift of their own
    // to find: b.txt is only on disk, c.txt is on disk but not in src's cache.
    wfile(&src, "c.txt", b"ccc");
    wfile(&dst, "b.txt", b"bbb");

    let code = run(
        mkcli(Cmd::CompareSelf {
            dir: src.clone(),
            no_trust_cached_hashes: false,
            common: common(),
        }),
        &plan,
    )
    .unwrap();
    // Must run *before* the compare below: that one populates src's cache as
    // it scans, which is exactly what compare-self is here to avoid doing.
    assert_eq!(code, 4, "a file on disk that src's cache never recorded");

    let code = run(
        mkcli(Cmd::Compare {
            src: src.clone(),
            dst: dst.clone(),
            trust: TrustArgs::default(),
            common: common(),
        }),
        &plan,
    )
    .unwrap();
    assert_eq!(code, 4);

    let code = run(
        mkcli(Cmd::Sync {
            src: src.clone(),
            dst: dst.clone(),
            missing_only: false,
            keep_extra: false,
            trust: TrustArgs::default(),
            common: common(),
        }),
        &plan,
    )
    .unwrap();
    assert_eq!(code, 0);
    assert_eq!(rfile(&dst, "a.txt"), b"aaa");

    let code = run(
        mkcli(Cmd::Compare {
            src: src.clone(),
            dst: dst.clone(),
            trust: TrustArgs::default(),
            common: common(),
        }),
        &plan,
    )
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
        run(
            json_cli(Cmd::CompareSelf {
                dir: src.clone(),
                no_trust_cached_hashes: false,
                common: common(),
            }),
            &plan
        )
        .unwrap(),
        4,
        "a JSON run reaches the same verdict"
    );
    assert_eq!(
        run(
            json_cli(Cmd::Sync {
                src: src.clone(),
                dst: dst.clone(),
                missing_only: false,
                keep_extra: false,
                trust: TrustArgs::default(),
                common: common(),
            }),
            &plan
        )
        .unwrap(),
        0
    );
    assert_eq!(rfile(&dst, "later.txt"), b"added after the first sync");
}

/// `--hash-all-of none` with a distrusted stat field is refused by the CLI: with
/// no digest requested there is nothing to settle a pair by, so a distrusted
/// stat would have nowhere to go and the run bails with exit 3 instead of
/// reporting a changed file equal. Pinned through the real dispatch, not just
/// the config unit level.
#[test]
fn run_cli_refuses_hash_all_of_none_with_distrusted_stat() {
    unsafe { std::env::set_var("GIRSYNC_TEST_CACHE", "memory") };
    let t = temp_root("refuse");
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    wfile(&src, "a.txt", b"aaa");

    let refusing = CommonArgs {
        hash_all_of: vec!["none".to_string()],
        hash_any_of: vec![],
        no_trust_size: true,
        no_trust_mtime: false,
        why: false,
        show_identical: false,
        include: vec![],
        exclude: vec![],
        case_sensitive: true,
        max_depth: 10,
        ignore_cache: false,
        dry_run: false,
        rw: RwArgs::default(),
    };

    let plan = girsync::plan(Level::Error, None, None, false);
    let err = run(
        Cli {
            log_level: Level::Error,
            log_file_level: None,
            log_file: None,
            no_log_file: false,
            output: OutputFormat::Text,
            cmd: Cmd::Compare {
                src: src.clone(),
                dst: dst.clone(),
                trust: TrustArgs::default(),
                common: refusing,
            },
        },
        &plan,
    );
    assert!(err.is_err(), "the combination must be refused: {err:?}");
    assert!(
        format!("{err:?}").contains("no-trust-size"),
        "the error names the distrusted flag: {err:?}"
    );
}
