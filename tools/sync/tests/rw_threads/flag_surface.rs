//! The flag surface: what the `--rw-threads*` family parses into.
//!
//! The resolution table, the mutual exclusions, and the reach of the family across
//! every subcommand — checked against `clap` directly rather than through a run, so a
//! combination the parser refuses is distinguished from one a command ignores.
//!
//! `limits_of` is in the parent: the refusal cases below and the end-to-end cases
//! both resolve a flag set the same way.

use super::*;

fn rw_args(threads: Option<usize>, src: Option<usize>, dst: Option<usize>, dual: bool) -> RwArgs {
    RwArgs {
        rw_threads: threads,
        rw_threads_src: src,
        rw_threads_dst: dst,
        rw_dual_drive: dual,
    }
}

/// **The flag table, every row.** The resolution is one `match` in `config.rs`, and
/// it is the whole of what a user can ask for — so it is stated here as the table
/// the doc comments promise, not as a restatement of the code.
#[test]
fn rw_flags_resolve_to_the_documented_limits() {
    let rows: &[(&str, RwArgs, RwLimits)] = &[
        (
            "no flags",
            rw_args(None, None, None, false),
            RwLimits::Shared(nz(1)),
        ),
        (
            "--rw-threads 4",
            rw_args(Some(4), None, None, false),
            RwLimits::Shared(nz(4)),
        ),
        (
            "--rw-dual-drive",
            rw_args(None, None, None, true),
            RwLimits::Split {
                src: nz(1),
                dst: nz(1),
            },
        ),
        (
            "--rw-threads-src 3",
            rw_args(None, Some(3), None, false),
            RwLimits::Split {
                src: nz(3),
                dst: nz(1),
            },
        ),
        (
            "--rw-threads-dst 5",
            rw_args(None, None, Some(5), false),
            RwLimits::Split {
                src: nz(1),
                dst: nz(5),
            },
        ),
        (
            "--rw-threads-src 3 --rw-threads-dst 5",
            rw_args(None, Some(3), Some(5), false),
            RwLimits::Split {
                src: nz(3),
                dst: nz(5),
            },
        ),
    ];
    for (flags, args, want) in rows {
        assert_eq!(&limits_of(args.clone()), want, "{flags}");
    }
}

/// **The unflagged run is exactly `Shared(1)`.**
///
/// Worth its own case because it is the *default*, and the default is what every
/// script that never mentioned concurrency gets — one read at a time, which is
/// the conservative choice for the single-disk case. A default is the thing most
/// likely to drift unnoticed, so it is pinned rather than assumed.
#[test]
fn an_unflagged_run_is_exactly_one_shared_operation() {
    let l = RwLimits::default();
    assert_eq!(l, RwLimits::Shared(nz(1)));
    assert_eq!(l.pool_size(), 1, "one worker, one permit");
}

/// **`--rw-dual-drive` is not `--rw-threads 1`.**
///
/// The claim the flag exists to make, and the one most likely to be quietly broken by
/// someone "simplifying" it into the shared case. Stated as the single observable
/// difference: under `dual`, a permit held on one side does not block the other;
/// under `--rw-threads 1` it does, because there is only one counter.
#[test]
fn dual_drive_is_not_the_same_request_as_one_shared_thread() {
    let dual = RwRuntime::new(limits_of(rw_args(None, None, None, true))).unwrap();
    let one = RwRuntime::new(limits_of(rw_args(Some(1), None, None, false))).unwrap();
    assert_ne!(
        dual.limits(),
        one.limits(),
        "the two flags mean different things"
    );

    for (name, rw, other_side_free) in [("dual", &dual, true), ("shared(1)", &one, false)] {
        let src = rw.acquire(RwSide::Src);
        let dst_free = rw.limiter().try_acquire(RwSide::Dst).is_some();
        drop(src);
        assert_eq!(
            dst_free,
            other_side_free,
            "{name}: holding src must {} the dst permit",
            if other_side_free {
                "leave free"
            } else {
                "exhaust"
            }
        );
    }
}

/// **`--rw-threads N` with N == 0 is refused, naming its own flag.**
///
/// A runtime error rather than a parse error, deliberately: the value arrives as a
/// `usize`, so `0` is well-formed and only the resolved `NonZeroUsize` can turn it
/// down. A clap range validator would have made it exit `2`, which reads as "you
/// typed it wrong" rather than "you asked for none of something". `--jobs 0` behaved
/// this way and this keeps it.
#[test]
fn rw_threads_rejects_a_zero_count() {
    for (flag, args) in [
        ("--rw-threads", rw_args(Some(0), None, None, false)),
        ("--rw-threads-src", rw_args(None, Some(0), None, false)),
        ("--rw-threads-dst", rw_args(None, None, Some(0), false)),
    ] {
        let err =
            RwLimits::try_from(args).expect_err("zero reads or writes in flight is not a run");
        assert!(
            format!("{err:#}").contains(flag),
            "{flag} = 0 must name {flag} in its error, got: {err:#}"
        );
    }
}

/// **A conflicting combination is refused by clap, before anything runs.**
///
/// In-process and no binary spawn, so this is cheap enough to state exhaustively:
/// four flags, six conflicting pairs.
#[test]
fn rw_flags_are_mutually_exclusive() {
    // Every conflicting *pair*, each stated once. Enumerated rather than sampled
    // because the table is small and a missing entry would be a hole in the surface:
    // `--rw-threads` conflicts with all three, and `--rw-dual-drive` with both
    // per-side flags, and the two per-side flags are deliberately NOT in conflict
    // with each other.
    let conflicting = [
        vec!["--rw-threads", "2", "--rw-threads-src", "2"],
        vec!["--rw-threads", "2", "--rw-threads-dst", "2"],
        vec!["--rw-threads", "2", "--rw-dual-drive"],
        vec!["--rw-dual-drive", "--rw-threads-src", "2"],
        vec!["--rw-dual-drive", "--rw-threads-dst", "2"],
    ];
    for args in conflicting {
        let argv: Vec<String> = ["girsync", "sync", "--src", "a", "--dst", "b"]
            .iter()
            .map(|s| s.to_string())
            .chain(args.iter().map(|s| s.to_string()))
            .collect();
        Cli::try_parse_from(&argv).expect_err(&format!("{args:?} must be refused"));
    }
    // The pair that is *not* a conflict, because it is the explicit form of the
    // whole feature: naming both sides is how you say "these are independent".
    Cli::try_parse_from([
        "girsync",
        "sync",
        "--src",
        "a",
        "--dst",
        "b",
        "--rw-threads-src",
        "2",
        "--rw-threads-dst",
        "3",
    ])
    .expect("both sides together is legal");
}

/// **The flags reach every subcommand, not just `sync`.**
///
/// `--jobs` lived on the `Sync` variant alone, which meant `update` — which rehashes
/// every file — had no way to be given a budget at all. That is the scope widening
/// this commit makes deliberately, and it is worth one test so nobody narrows the
/// surface back by accident.
#[test]
fn the_rw_flags_reach_every_subcommand() {
    for cmd in [
        vec!["update", "--dir", "a"],
        vec!["compare", "--src", "a", "--dst", "b"],
        vec!["compare-self", "--dir", "a"],
        vec!["sync", "--src", "a", "--dst", "b"],
    ] {
        let argv: Vec<String> = ["girsync"]
            .iter()
            .map(|s| s.to_string())
            .chain(cmd.iter().map(|s| s.to_string()))
            .chain(["--rw-threads", "2"].iter().map(|s| s.to_string()))
            .collect();
        assert!(
            Cli::try_parse_from(&argv).is_ok(),
            "{cmd:?} should accept --rw-threads"
        );
    }
}

/// **`--jobs` is gone, loudly.**
///
/// The one half of the removal that is not a feature. A script still passing `--jobs
/// 4` now fails at clap with exit 2 and an "unexpected argument" message that names
/// neither the replacement nor the reason — which is why this is a deliberate choice
/// rather than an oversight, and why the Gotchas entry exists. The crate's own
/// precedent is `--hash` -> `--hash-all-of`, rejected rather than aliased so a script
/// fails loudly instead of quietly getting the new default.
#[test]
fn jobs_is_rejected_rather_than_aliased() {
    let err = Cli::try_parse_from(["girsync", "sync", "--src", "a", "--dst", "b", "--jobs", "4"])
        .expect_err("--jobs must not parse");
    let msg = err.to_string();
    assert!(msg.contains("--jobs"), "the message names the flag: {msg}");
}
