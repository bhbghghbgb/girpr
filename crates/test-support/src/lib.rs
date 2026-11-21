//! The facade-coupled glue the root integration tests share.
//!
//! The bulk of the old `tests/common/mod.rs` — the offline mock, the fixture
//! builders, the digest spelling and the lock cleanup — now lives once in the
//! facade-free `girpr-test-fixtures` library. What remained on the test side
//! was a small set of helpers that must name facade types: they construct a
//! [`RunCtx`] from the private config types, drive the real pipeline, and read
//! the counters off a [`Summary`]. They are public here, reached through the
//! thin `tests/common/mod.rs` shim, so nextest's one-process-per-test model
//! compiles them once instead of once per integration binary.
//!
//! The binary runner deliberately stays in the shim: `CARGO_BIN_EXE_girpr` is
//! only bound into an integration-test target, not into a library dependency.
//!
//! This crate depends on the facade, so a product edit recompiles it; that is
//! unavoidable for glue that names facade types. The win is compilation volume:
//! eleven binaries no longer each compile a copy of these helpers.

// Test glue asserts with `expect`; these are not production paths.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashSet;

use girpr::test_support::{BackoffPolicy, Limits, PathScope, RepairMode};
use girpr::{Biz, OutputFormat, RunCtx, RunExit, RunFailure, Summary};

// Host-relative fixture timestamps. The implementation lives behind the
// facade's test seam so unit and integration tests share one definition; this
// re-export is the integration side's way in.
pub use girpr::test_support::{stamp_ago, stamp_ahead};

/// Compile a path scope for a test, the way `Args::into_ctx` does.
pub fn scope(include: &[&str], exclude: &[&str]) -> PathScope {
    let inc: Vec<String> = include.iter().map(|s| s.to_string()).collect();
    let exc: Vec<String> = exclude.iter().map(|s| s.to_string()).collect();
    PathScope::new(&inc, &exc).expect("test globs must compile")
}

/// A default run context against the mock's endpoints.
pub fn ctx_for(
    game_dir: std::path::PathBuf,
    mock: &girpr_test_fixtures::MockServer,
    jobs: usize,
) -> RunCtx {
    ctx_for_biz(game_dir, mock, jobs, Biz::Hk4eGlobal)
}

/// A run context for a specific launcher against the mock's endpoints.
pub fn ctx_for_biz(
    game_dir: std::path::PathBuf,
    mock: &girpr_test_fixtures::MockServer,
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

/// The five counters a test asserts on, read atomically from a [`Summary`].
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
/// shared temp directory. A read-only run takes no lock, so cleanup is skipped
/// for it rather than acquiring a lock just to find the file to delete.
pub async fn run(ctx: RunCtx) -> Result<(Summary, RunExit), RunFailure> {
    let game_dir = ctx.game_dir.clone();
    let takes_lock =
        girpr_test_fixtures::mode_takes_lock(ctx.readonly(), ctx.check_only, ctx.print_plan);
    let result = girpr::run(ctx).await;
    if takes_lock {
        girpr_test_fixtures::remove_run_lock(&game_dir);
    }
    result
}
