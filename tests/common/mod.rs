//! Offline mock API for the root integration tests — thin facade glue.
//!
//! The mock server, fixture builders, digest and lock-cleanup mechanics live
//! once in the `girpr-test-fixtures` library. nextest runs one process per test
//! and skips doctests, so a helper inlined here with `mod common;` would be
//! compiled into every integration binary; only this shim is inlined now. The
//! few items below are the ones that must stay on the test side: they build a
//! `RunCtx` from facade-private config types, call the pipeline, or need the
//! `CARGO_BIN_EXE_girpr` binding Cargo gives only to integration targets.

// Each binary uses a different part of this shared surface, so most of it is
// dead code or an unused re-export in any one of them.
#![allow(dead_code, unused_imports)]

use std::collections::HashSet;

use girpr::test_support::{BackoffPolicy, Limits, PathScope, RepairMode};
use girpr::{Biz, OutputFormat, RunCtx, RunExit, RunFailure, Summary};

// The shared fixture/mock surface, compiled once in the library.
pub use girpr_test_fixtures::{
    BranchFailure, ChunkSpec, ExtraManifestSpec, FileSpec, FixtureChunk, FixtureFile,
    FixtureLatestManifest, FixtureManifest, FixtureSpec, GameFixture, LATEST_TAG, LOCAL_TAG, Lies,
    MockOpts, MockServer, PLAN_SCAN_BYTES_KEY, UNSERVED_TAG, absent_game_dir, build_fixture,
    build_fixture_spec, build_fixture_with_lies, build_moved_fixture, field, lay_out, logs_of,
    read_game_file, temp_dir, write_game_file,
};

// Host-relative fixture timestamps. The implementation lives behind the
// library crate's test seam so the unit tests and the integration tests share
// one definition; this re-export is the integration side's way in.
pub use girpr::test_support::{stamp_ago, stamp_ahead};

mod run_lock;

// Each test binary uses one of the two helpers, so the other is unused here.
#[allow(unused_imports)]
pub use run_lock::{remove_run_lock, remove_run_lock_for_args};

mod digest;

pub use digest::md5hex;

// ---------------------------------------------------------------------------
// facade glue (stays on the test side)
// ---------------------------------------------------------------------------

/// Compile a path scope for a test, the way `Args::into_ctx` does.
pub fn scope(include: &[&str], exclude: &[&str]) -> PathScope {
    let inc: Vec<String> = include.iter().map(|s| s.to_string()).collect();
    let exc: Vec<String> = exclude.iter().map(|s| s.to_string()).collect();
    PathScope::new(&inc, &exc).expect("test globs must compile")
}

pub fn ctx_for(game_dir: std::path::PathBuf, mock: &MockServer, jobs: usize) -> RunCtx {
    ctx_for_biz(game_dir, mock, jobs, Biz::Hk4eGlobal)
}

pub fn ctx_for_biz(
    game_dir: std::path::PathBuf,
    mock: &MockServer,
    jobs: usize,
    biz: Biz,
) -> RunCtx {
    RunCtx {
        game_dir,
        biz,
        audio: HashSet::new(),
        audio_explicit: false,
        path_scope: PathScope::default(),
        limits: Limits::default(),
        repair_mode: RepairMode::Auto,
        repair_mode_explicit: false,
        jobs,
        check_only: false,
        scan: false,
        purge_only: false,
        fail_fast: false,
        dry_run: false,
        print_plan: false,
        plan_out: None,
        purge_after: false,
        purge_before: false,
        output: OutputFormat::Text,
        hyp_base_override: Some(mock.hyp_base.clone()),
        sophon_base_override: Some(mock.sophon_base.clone()),
        // Tests skip retry backoff: a failed fetch is retried immediately.
        backoff: BackoffPolicy::none(),
    }
}

pub fn load_counters(s: &Summary) -> (u64, u64, u64, u64, u64) {
    (
        s.files_skipped.load(std::sync::atomic::Ordering::Relaxed),
        s.files_repaired.load(std::sync::atomic::Ordering::Relaxed),
        s.files_failed.load(std::sync::atomic::Ordering::Relaxed),
        s.download.settled(),
        s.deleted_extra_bytes
            .load(std::sync::atomic::Ordering::Relaxed),
    )
}

/// Run the library's entry point, then remove the lock file it leaves in the
/// shared temp directory. See [`remove_run_lock`].
///
/// A read-only run takes no lock, so cleanup is skipped for it rather than
/// acquiring a lock just to find and delete the file that acquisition created.
pub async fn run(ctx: RunCtx) -> Result<(Summary, RunExit), RunFailure> {
    let game_dir = ctx.game_dir.clone();
    let takes_lock = run_lock::mode_takes_lock(ctx.readonly(), ctx.check_only, ctx.print_plan);
    let result = girpr::run(ctx).await;
    if takes_lock {
        run_lock::remove_run_lock(&game_dir);
    }
    result
}

/// Run the shipped binary. The endpoint overrides are the only seam a
/// *binary-level* test can use, and this is where that seam is exercised: the
/// library tests inject `RunCtx::hyp_base_override` directly,
/// which proves nothing about whether a flag reaches `HypClient`.
///
/// Async on purpose: a blocking `Command` inside `#[tokio::test]` would occupy
/// the single runtime thread the mock server needs to answer on.
pub async fn run_binary(args: &[&str], env: &[(&str, &str)]) -> (i32, String, String) {
    let mut cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_girpr"));
    cmd.args(args);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().await.expect("run girpr");
    remove_run_lock_for_args(args);
    (
        out.status.code().expect("process must exit normally"),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}
