//! `--no-trust-size` / `--no-trust-mtime`: switching the stat short circuit off.
//!
//! The short circuit is an optimisation, so it has to be switchable. These two flags
//! name the stat fields it consults, and **both together is "disable it"** — no third
//! spelling is needed to reach the whole thing off.
//!
//! What each one does is stated in the planner's tests; what is left here is what can
//! only be seen from outside:
//!
//! - **the counters.** A flag that widens the undecided set has to actually cost the
//!   reads, and the counts are the only honest observable. Every case states the
//!   `all-of`-style default as a control, because a test asserting only "the flag
//!   reads more" would pass against a planner that always read everything.
//! - **the verdict.** `--no-trust-mtime` can *change* one — a touched file with
//!   identical bytes comes back equal. That is the flag's whole point, so it is
//!   asserted end to end rather than left to the planner's arithmetic.
//! - **the cache.** This is what `--no-trust-size` is really for, and it is invisible
//!   in both counters and verdicts.
//! - **the flags themselves**, including `update`'s WARN, observed through a
//!   scoped subscriber (the same stack the binary installs, without a spawn).
//!
//! ## Layout
//!
//! This file holds the options builder and the fixture every case measures
//! against. The cases are grouped by what they can observe:
//!
//! | module | what it covers |
//! | --- | --- |
//! | [`counters`] | reads: a widened undecided set has to cost them |
//! | [`verdict`] | verdicts: the one flag whose answer can change |
//! | [`cache_repair`] | the stuck cache row, invisible in both of the above |
//! | [`other_commands`] | `sync` and `compare-self`, where the flags stop being an optimisation |
//! | [`flag_surface`] | `update`, and the flags as the CLI accepts them |
//! | [`no_fallback`] | what a pair is called when nothing is left to settle it |
//! | [`refusal`] | the combination refused at the flag layer |

// This target lives at `tests/stat_trust/main.rs`, so the shared fixtures one
// level up need an explicit path rather than the sibling `mod common;` every
// other test target gets.
#[path = "../common/mod.rs"]
mod common;

mod cache_repair;
mod counters;
mod flag_surface;
mod no_fallback;
mod other_commands;
mod refusal;
mod verdict;

use common::{TempDir, age, log, opts, recs_of, sync_mtime, temp_root, update_mem, wfile};
use girsync::cache::TestCacheBackend;
use girsync::planner::StatTrust;
use girsync::{CommonOpts, cmd_compare, cmd_compare_self, cmd_sync, cmd_update};

/// Run the CLI in-process under a scoped copy of the binary's tracing stack and
/// return the console bytes it rendered (plus the exit code). This replaces the
/// `bin`/`bin_mem` spawns for the one flag-surface case that used to read a WARN
/// off the child's stderr: the stack is the one `girsync::subscriber_for`
/// builds, with the console written into a buffer instead of a pipe.
///
/// `tracing` caches callsite interest process-globally, so a sibling test in
/// this process may emit the WARN under `NoSubscriber` and cache that callsite
/// as `Interest::never()`, leaving the scoped subscriber zero bytes. Two
/// measures make capture deterministic here (the rebuild is the same fix
/// `capture_log` in `phases.rs` and `capture_events` in `copy.rs` apply):
///
/// 1. Run once under `NoSubscriber` first, to force the WARN callsite to
///    *register* before the scoped subscriber is installed. Callsite
///    registration is a one-time event; a sibling reproducing it after our
///    rebuild would re-cache `never()` and drop the WARN again.
/// 2. Rebuild the interest cache as the first statement inside `with_default`,
///    re-registering that now-known callsite against the scoped subscriber.
///
/// Both live here, local to this fold, rather than in the shared
/// `common::run_args_logged` helper: they are process-global, so folding them
/// into that helper would race with another target's differently-filtered
/// scoped subscribers (output.rs's `logged()` tests) under plain `cargo test`.
// `set_var` is unsafe in edition 2024; nextest isolates each test process.
#[allow(unsafe_code)]
fn run_logged(args: &[String]) -> (i32, String) {
    use clap::Parser;
    use girsync::cli::Cli;

    let make_cli = || {
        let argv = std::iter::once(std::ffi::OsString::from("girsync"))
            .chain(args.iter().cloned().map(Into::into));
        Cli::try_parse_from(argv).expect("valid CLI args")
    };
    let cli = make_cli();
    let plan = girsync::plan(
        cli.log_level,
        cli.log_file_level,
        cli.log_file.as_deref(),
        cli.no_log_file,
    );
    // A spawned child got a fresh cache per process; keep the in-memory
    // registry from leaking across runs that share a path.
    unsafe { std::env::set_var("GIRSYNC_TEST_CACHE", "file") };
    // Force the WARN callsite to register before installing the scoped
    // subscriber. Callsite registration is a one-time, process-global event: if
    // a sibling thread performs it after our rebuild, it would cache the site
    // as `never()` against `NoSubscriber` and the WARN would be lost again.
    let _ = girsync::run_captured(make_cli(), &plan).expect("pre-register callsite");
    let console = common::SharedBuffer::default();
    let (subscriber, _guard) = girsync::subscriber_for(&plan, console.clone());
    let out = tracing::subscriber::with_default(subscriber, || {
        tracing::callsite::rebuild_interest_cache();
        girsync::run_captured(cli, &plan).expect("run_captured")
    });
    (out.code, console.text())
}

/// Restore a file's mtime to a value captured earlier.
///
/// Load-bearing wherever a *rewrite* is part of the fixture: rewriting moves the mtime,
/// and a pair whose mtime also differs is settled by the field this run still trusts —
/// so the flag under test would widen nothing and the fixture would appear broken.
fn restore_mtime(p: &std::path::Path, t: std::time::SystemTime) {
    std::fs::OpenOptions::new()
        .write(true)
        .open(p)
        .unwrap()
        .set_modified(t)
        .unwrap();
}

/// src and dst differing in length, with the mtime pinned so size is the only signal.
///
/// The shape both `--no-trust-size`'s cases and the "no digest to fall back on"
/// cases need: a pair whose *only* evidence of difference is size, so switching
/// that field off leaves the pair undecidable.
fn size_differs(t: &TempDir) -> (std::path::PathBuf, std::path::PathBuf) {
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    wfile(&src, "a.txt", b"short");
    wfile(
        &dst,
        "a.txt",
        b"a considerably longer body than the other side",
    );
    sync_mtime(&src.join("a.txt"), &dst.join("a.txt"));
    (src, dst)
}

/// `opts()` asking for `algos`, with stat trust set, served from an in-memory
/// cache (semantic suite: asserts counts/verdicts, not the cache file).
fn distrusting(algos: &[&str], stat: StatTrust) -> CommonOpts {
    CommonOpts {
        test_cache: TestCacheBackend::Memory,
        algos: algos.iter().map(|a| a.parse().unwrap()).collect(),
        stat,
        ..opts()
    }
}

/// Two folders, four files, one per interesting pair state.
///
/// ```text
///   equal.txt    identical bytes, mtime pinned   -> undecided, hashed either way
///   sized.txt    dst is longer, mtime pinned     -> free by size
///   mtime'd.txt  identical bytes, dst's mtime moved -> free by mtime
///   gone.txt     dst only                        -> EXTRA, free
/// ```
///
/// `equal.txt` is the control: it is already undecided, so it hashes in every
/// configuration and any change in the other three is attributable.
fn four_states(t: &TempDir) -> (std::path::PathBuf, std::path::PathBuf) {
    let src = t.subdir("src");
    let dst = t.subdir("dst");
    for rel in ["equal.txt", "sized.txt", "mtime'd.txt"] {
        let body = format!("body of {rel}");
        wfile(&src, rel, body.as_bytes());
        wfile(&dst, rel, body.as_bytes());
    }
    // `sized.txt` differs in length on dst, mtime pinned so size is the only signal.
    wfile(
        &dst,
        "sized.txt",
        b"a much longer body than src has for this path",
    );
    for rel in ["equal.txt", "sized.txt", "mtime'd.txt"] {
        sync_mtime(&src.join(rel), &dst.join(rel));
    }
    // `mtime'd.txt` then gets its mtime pushed forward: identical bytes, same size,
    // so mtime is the only thing that differs.
    age(&dst, "mtime'd.txt", 60);
    wfile(&dst, "gone.txt", b"dst only");
    (src, dst)
}

/// Files opened by a two-sided `compare`, summed over both sides.
///
/// Resolving a folder side **writes to its cache**, so this is not a pure function of
/// its inputs and a case must not measure two configurations against one fixture — the
/// first run backfills whatever the second would then find cached. Every case here
/// builds a fresh fixture per configuration. The counters come from the command's own
/// computation, so only the cache side effect is re-derived here, not the pipeline.
fn hashed(src: &std::path::Path, dst: &std::path::Path, common: &CommonOpts) -> usize {
    let opts = girsync::CompareOpts {
        src: src.to_path_buf(),
        dst: dst.to_path_buf(),
        trust: girsync::TrustOpts::default(),
        common: common.clone(),
    };
    let outcome = girsync::commands::compare_detailed(opts, &log())
        .expect("two folders cannot fail coverage");
    outcome.src_stats.hashed + outcome.dst_stats.hashed
}
