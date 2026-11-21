//! Offline mock API for the root integration tests — thin facade glue.
//!
//! The mechanics live once in two libraries. `girpr-test-fixtures` is the
//! facade-free mock, fixture builders, digest and lock cleanup; a product edit
//! leaves it cached. `girpr-test-support` is the small facade-coupled glue that
//! builds a `RunCtx` and drives the pipeline. nextest runs one process per test
//! and skips doctests, so inlining either library here with `mod common;` would
//! compile it into every integration binary; only this shim is inlined now.
//!
//! `run_binary` stays here because `CARGO_BIN_EXE_girpr` is bound only into an
//! integration-test target, not into a library dependency.

// Each binary uses a different part of this shared surface, so most of it is
// dead code or an unused re-export in any one of them.
#![allow(dead_code, unused_imports)]

// The shared fixture/mock surface, compiled once in the library.
pub use girpr_test_fixtures::{
    BranchFailure, ChunkSpec, ExtraManifestSpec, FileSpec, FixtureChunk, FixtureFile,
    FixtureLatestManifest, FixtureManifest, FixtureSpec, GameFixture, LATEST_TAG, LOCAL_TAG, Lies,
    MockOpts, MockServer, PLAN_SCAN_BYTES_KEY, UNSERVED_TAG, absent_game_dir, build_fixture,
    build_fixture_spec, build_fixture_with_lies, build_moved_fixture, field, lay_out, logs_of,
    read_game_file, temp_dir, write_game_file,
};

// The facade-coupled glue, also compiled once in its library.
pub use girpr_test_support::{
    ctx_for, ctx_for_biz, load_counters, run, scope, stamp_ago, stamp_ahead,
};

mod run_lock;

// A given binary uses only part of this surface, so the rest are unused here.
#[allow(unused_imports)]
pub use run_lock::{
    args_take_lock, game_path_arg, lock_key_path, mode_takes_lock, remove_run_lock,
    remove_run_lock_for_args,
};

mod digest;

pub use digest::md5hex;

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
