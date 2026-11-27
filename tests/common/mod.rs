//! Offline mock API for the root integration tests — thin facade glue.
//!
//! The mechanics live once in two libraries. `girpr-test-fixtures` is the
//! facade-free mock, fixture builders, digest and lock cleanup; a product edit
//! leaves it cached. `girpr-test-support` is the small facade-coupled glue that
//! builds a `RunCtx` and drives the pipeline. nextest runs one process per test
//! and skips doctests, so inlining either library here with `mod common;` would
//! compile it into every integration binary; only this shim is inlined now.
//!
//! `run_binary*` stays here because `CARGO_BIN_EXE_girpr` is bound only into an
//! integration-test target, not into a library dependency.

// Each binary uses a different part of this shared surface, so most of it is
// dead code or an unused re-export in any one of them.
#![allow(dead_code, unused_imports)]

// The shared fixture/mock surface, compiled once in the library.
pub use girpr_test_fixtures::{
    BranchFailure, ChunkSpec, ExtraManifestSpec, FileSpec, FixtureChunk, FixtureFile,
    FixtureLatestManifest, FixtureManifest, FixtureSpec, GameFixture, LATEST_TAG, LOCAL_TAG, Lies,
    MockOpts, MockServer, PLAN_SCAN_BYTES_KEY, UNSERVED_TAG, absent_game_dir, build_fixture,
    build_fixture_spec, build_fixture_with_lies, build_moved_fixture, field, field_token, lay_out,
    logs_of, read_game_file, read_json, str_field, temp_dir, write_game_file, write_json,
    zstd_bytes,
};

// Console captures are compared as plain text. The escape grammar belongs to
// the tempdir test-support leaf; re-exporting it here keeps every call site's
// `common::strip_ansi(..)` spelling unchanged.
pub use girpr_tempdir::test_support::{set_mtime, strip_ansi};

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

/// How the child's stdout is handled by [`run_binary_with`]. Every policy
/// still collects stderr and cleans up the run lock; only the stdout seam
/// differs, which is what the closed-consumer case is testing.
pub enum StdoutPolicy {
    /// `Command::output`: collect stdout and stderr into the returned strings.
    Captured,
    /// Pipe stdout and drop the reading end immediately, so the run's first
    /// data-plane write meets a closed consumer. The returned stdout is empty.
    Closed,
}

/// Spawn the shipped binary with the caller's argv/env and the chosen stdout
/// seam, then clean up the run lock. The endpoint overrides are the only seam a
/// *binary-level* test can use, and this is where that seam is exercised: the
/// library tests inject `RunCtx::hyp_base_override` directly, which proves
/// nothing about whether a flag reaches `HypClient`.
///
/// Async on purpose: a blocking `Command` inside `#[tokio::test]` would occupy
/// the single runtime thread the mock server needs to answer on. `A` is generic
/// so both the `&[&str]` call sites and the ones that build `Vec<String>` argv
/// can share the one spawn/lock-cleanup body.
pub async fn run_binary_with<A: AsRef<str> + AsRef<std::ffi::OsStr>>(
    args: &[A],
    env: &[(&str, &str)],
    stdout_policy: StdoutPolicy,
) -> (i32, String, String) {
    let mut cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_girpr"));
    cmd.args(args);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let (status, stdout, stderr) = match stdout_policy {
        StdoutPolicy::Captured => {
            let out = cmd.output().await.expect("run girpr");
            (out.status, out.stdout, out.stderr)
        }
        StdoutPolicy::Closed => {
            cmd.stdout(std::process::Stdio::piped());
            cmd.stderr(std::process::Stdio::piped());
            let mut child = cmd.spawn().expect("spawn girpr");
            drop(child.stdout.take());
            let out = child.wait_with_output().await.expect("wait for girpr");
            // The reading end was taken above, so there are no stdout bytes to
            // collect; `wait_with_output` still gathers stderr for the caller.
            (out.status, Vec::new(), out.stderr)
        }
    };
    remove_run_lock_for_args(args);
    (
        status.code().expect("process must exit normally"),
        String::from_utf8_lossy(&stdout).into_owned(),
        String::from_utf8_lossy(&stderr).into_owned(),
    )
}

/// The usual binary run: both streams captured.
pub async fn run_binary(args: &[&str], env: &[(&str, &str)]) -> (i32, String, String) {
    run_binary_with(args, env, StdoutPolicy::Captured).await
}

/// The closed-stdout case, keeping its `(code, stderr)` shape for the one call
/// site that asserts a run against a consumer that went away.
pub async fn run_binary_closed_stdout(args: &[&str], env: &[(&str, &str)]) -> (i32, String) {
    let (code, _, stderr) = run_binary_with(args, env, StdoutPolicy::Closed).await;
    (code, stderr)
}
