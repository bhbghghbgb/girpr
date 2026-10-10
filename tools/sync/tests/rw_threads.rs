//! The rw limiter: one gate over every file read and write, with a per-side mode.
//!
//! The limiter is wired into phase C and the two sides of a pair now resolve
//! concurrently, but no flag reaches any of it yet — a command builds itself a
//! `serial()` runtime. That leaves the invariant unpinned, and this is the worst
//! moment to leave it: the structural change and the CLI break land in different
//! commits, and the second is the loud one. So the limiter is exercised here
//! through its public API, at every shape the CLI is about to offer.
//!
//! What is asserted throughout is the crate's standing rule: the limiter changes
//! **scheduling** and nothing else. Every setting must resolve the same trees to
//! the same effective maps — digests included — and report the same `ScanStats`,
//! compared whole rather than field by field, so a counter added later cannot
//! diverge between settings without this noticing.
//!
//! What is *not* here, and why: a `sync` driven at `split(1,1)` through the CLI.
//! That needs the flags this change has not shipped yet, so it arrives with them.
//! What is here instead is the same property stated directly — that concurrent
//! two-permit acquisition terminates — which does not need a flag to be true.

mod common;

use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::Barrier;
use std::sync::atomic::{AtomicUsize, Ordering};

use clap::Parser;
use common::{Spec, TempRoot, pair, recs_of, serial_rw};
use girsync::cache::CACHE_PREFIX;
use girsync::cli::{Cli, RwArgs};
use girsync::config::{CommonOpts, ScanMode};
use girsync::effective::{
    EffRec, OpenSide, ScanStats, classify, ensure_distinct_sides, open_side, resolve_side,
    resolve_sides_concurrently,
};
use girsync::planner::{SideRequest, plan_pairs};
use girsync::rw::{RwLimits, RwRuntime, RwSide};

/// Resolve a flag combination the way the CLI does, without going through clap.
fn limits_of(a: RwArgs) -> RwLimits {
    RwLimits::try_from(a).unwrap_or_else(|e| panic!("{e:#}"))
}

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
/// script that never mentioned concurrency gets. It is also the largest behavioural
/// change in the flag's history: `--jobs` defaulted to 4, so an unflagged `sync` now
/// runs one copy at a time rather than four. That is the conservative choice — a
/// single disk is the common case — but it is a change, so it is pinned rather than
/// left to drift.
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
    let conflicting = [
        vec!["--rw-threads", "2", "--rw-dual-drive"],
        vec!["--rw-threads", "2", "--rw-threads-src", "2"],
        vec!["--rw-threads", "2", "--rw-threads-dst", "2"],
        vec!["--rw-dual-drive", "--rw-threads-src", "2"],
        vec!["--rw-dual-drive", "--rw-threads-dst", "2"],
        vec!["--rw-threads-src", "2", "--rw-dual-drive"],
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

fn nz(n: usize) -> NonZeroUsize {
    NonZeroUsize::new(n).unwrap()
}

/// One resolved side: the map it produced and the counters that describe how.
type Resolved = (HashMap<String, EffRec>, ScanStats);

/// Every shape the CLI resolves to, plus what a command builds when the user
/// names nothing.
fn settings() -> Vec<(&'static str, RwLimits)> {
    vec![
        ("serial", RwLimits::Shared(nz(1))),
        ("shared(2)", RwLimits::Shared(nz(2))),
        ("shared(4)", RwLimits::Shared(nz(4))),
        (
            "split(1,1)",
            RwLimits::Split {
                src: nz(1),
                dst: nz(1),
            },
        ),
        (
            "split(4,4)",
            RwLimits::Split {
                src: nz(4),
                dst: nz(4),
            },
        ),
        (
            "split(4,1)",
            RwLimits::Split {
                src: nz(4),
                dst: nz(1),
            },
        ),
    ]
}

/// Padding content for the bulk of the fixture. A `const` rather than a repeat
/// expression so the slices in the spec are genuinely `'static`, which the
/// fixture's `Spec` type requires.
const PAD: [u8; 512] = [b'x'; 512];

/// A tree with every kind of work in the plan: stat-equal pairs that only a
/// digest can settle, a pair settled by size, a src-only and a dst-only path, a
/// directory and an empty file.
///
/// Deliberately **more files than one resolve window**, so the windowed loop runs
/// several times. A fixture that fit in a single window would pass even if the
/// window boundary dropped a file, duplicated one, or lost the write order
/// between windows.
fn fixture(t: &TempRoot) -> (PathBuf, PathBuf) {
    let mut spec: Vec<Spec> = vec![
        ("changed.txt", Some(b"src version"), Some(b"dst")),
        ("sized.txt", Some(b"src is longer"), Some(b"s")),
        ("src_only.txt", Some(b"only here"), None),
        ("dst_only.txt", None, Some(b"only there")),
        ("sub/nested.txt", Some(b"nested"), Some(b"nest")),
        ("emptydir", Some(&[]), None),
    ];
    // 96 files. With the smallest window the limiter computes (32) that is three
    // windows, so the boundary is exercised three times rather than trivially
    // satisfied once.
    for i in 0..96usize {
        let rel: &'static str = Box::leak(format!("pad{i:02}.bin").into_boxed_str());
        spec.push((rel, Some(&PAD[..]), Some(&PAD[..])));
    }
    pair(t, &spec, &["md5"], false)
}

/// Remove both caches so the next resolution is a cold run over the *same* bytes.
///
/// The alternative — build a fresh pair per setting — would make the whole test
/// vacuous: `wfile` stamps each file at write time, so two "identical" fixtures
/// have different mtimes, and `EffRec` carries `mtime_ns`. The maps would then
/// differ for a reason that has nothing to do with the limiter, and a loose
/// assertion would wave it through. One pair, re-resolved cold, is the only shape
/// in which map equality means what it says.
fn drop_caches(src: &Path, dst: &Path) {
    for dir in [src, dst] {
        std::fs::remove_file(dir.join(CACHE_PREFIX)).ok();
    }
}

/// Plan both sides, the way `cmd_compare` does.
fn plan_both<'a>(
    so: &'a OpenSide,
    do_: &'a OpenSide,
    common: &CommonOpts,
    s: &girsync::effective::Side,
    d: &girsync::effective::Side,
) -> girsync::planner::PairPlan {
    let (s_label, d_label) = (
        format!("src {}", s.cache_path().display()),
        format!("dst {}", d.cache_path().display()),
    );
    plan_pairs(
        SideRequest {
            entries: &so.phase_a.map,
            algos: &common.algos,
            no_trust: false,
            cap: so.cap,
            label: &s_label,
        },
        SideRequest {
            entries: &do_.phase_a.map,
            algos: &common.algos,
            no_trust: false,
            cap: do_.cap,
            label: &d_label,
        },
        common.case_sensitive,
        common.hash_mode,
        common.stat,
    )
    .unwrap_or_else(|e| panic!("two folders cannot fail coverage: {e:#}"))
}

const OPEN_MODE: ScanMode = ScanMode {
    no_trust_cached_hashes: false,
    dry_run: false,
};

/// Resolve both sides at once, exactly as `cmd_compare` does, under `limits`.
fn resolve_overlapping(
    src: &Path,
    dst: &Path,
    common: &CommonOpts,
    limits: RwLimits,
) -> (Resolved, Resolved) {
    let rw = RwRuntime::new(limits).unwrap();
    let (s, d) = (classify(src), classify(dst));
    ensure_distinct_sides(&s, &d).unwrap();
    let mut so = open_side(&s, common, OPEN_MODE).unwrap();
    let mut do_ = open_side(&d, common, OPEN_MODE).unwrap();
    let plans = plan_both(&so, &do_, common, &s, &d);
    let (src_res, dst_res) = resolve_sides_concurrently(
        || resolve_side(&mut so, OPEN_MODE, &plans.src, RwSide::Src, &rw),
        || resolve_side(&mut do_, OPEN_MODE, &plans.dst, RwSide::Dst, &rw),
    );
    let sm = src_res.unwrap_or_else(|e| panic!("resolving src: {e:#}"));
    let dm = dst_res.unwrap_or_else(|e| panic!("resolving dst: {e:#}"));
    ((sm.map, sm.stats), (dm.map, dm.stats))
}

/// Resolve one side after the other, under a serial runtime.
fn resolve_sequential(src: &Path, dst: &Path, common: &CommonOpts) -> (Resolved, Resolved) {
    let rw = serial_rw();
    let (s, d) = (classify(src), classify(dst));
    ensure_distinct_sides(&s, &d).unwrap();
    let mut so = open_side(&s, common, OPEN_MODE).unwrap();
    let mut do_ = open_side(&d, common, OPEN_MODE).unwrap();
    let plans = plan_both(&so, &do_, common, &s, &d);
    let sm = resolve_side(&mut so, OPEN_MODE, &plans.src, RwSide::Src, &rw).unwrap();
    let dm = resolve_side(&mut do_, OPEN_MODE, &plans.dst, RwSide::Dst, &rw).unwrap();
    ((sm.map, sm.stats), (dm.map, dm.stats))
}

/// **Every rw setting answers the same question.**
///
/// One fixture, re-resolved cold at every shape, and for each: the two effective
/// maps — digests included — and the two `ScanStats`, compared whole against the
/// serial run.
///
/// Comparing maps rather than printed verdicts is the stronger claim and the
/// deliberate one. Two runs can agree on every `CHANGED` line while having hashed
/// a different set of files and reached the same conclusion by luck; the maps hold
/// the digests themselves, so they cannot agree that way.
#[test]
fn every_rw_setting_answers_the_same_question() {
    let t = TempRoot::new("rw_same");
    let (src, dst) = fixture(&t);
    let common = common::opts();

    drop_caches(&src, &dst);
    let baseline = resolve_overlapping(&src, &dst, &common, RwLimits::default());
    assert!(
        baseline.0.1.hashed > 50,
        "the fixture must force real hashing (src hashed {}), or the comparison \
         below is vacuous",
        baseline.0.1.hashed
    );

    for (name, limits) in settings() {
        drop_caches(&src, &dst);
        let got = resolve_overlapping(&src, &dst, &common, limits);
        assert_eq!(got.0.0, baseline.0.0, "{name}: src effective map differs");
        assert_eq!(got.1.0, baseline.1.0, "{name}: dst effective map differs");
        assert_eq!(
            got.0.1, baseline.0.1,
            "{name}: src counters differ, so this setting read a different set of \
             files"
        );
        assert_eq!(got.1.1, baseline.1.1, "{name}: dst counters differ");
    }
}

/// **An overlapping resolve agrees with a sequential one.**
///
/// Distinct from the test above on purpose. That varies the *limits* with the call
/// shape fixed; this varies the *call shape* — the commands' overlapping
/// `resolve_sides_concurrently` against a straight sequential resolve — with the
/// limits fixed. The seam this change opened is the concurrency, and pinning only
/// the limits would leave the new threading itself unasserted.
///
/// Two `redb::WriteTransaction`s are also open against two caches at the same
/// time here, which is the first place in the test suite that happens.
#[test]
fn an_overlapping_resolve_agrees_with_a_sequential_one() {
    let t = TempRoot::new("rw_overlap");
    let (src, dst) = fixture(&t);
    let common = common::opts();

    drop_caches(&src, &dst);
    let concurrent = resolve_overlapping(
        &src,
        &dst,
        &common,
        RwLimits::Split {
            src: nz(4),
            dst: nz(4),
        },
    );

    drop_caches(&src, &dst);
    let sequential = resolve_sequential(&src, &dst, &common);

    assert_eq!(
        concurrent.0.0, sequential.0.0,
        "src map is the same either way"
    );
    assert_eq!(
        concurrent.1.0, sequential.1.0,
        "dst map is the same either way"
    );
    assert_eq!(
        concurrent.0.1, sequential.0.1,
        "src counters are the same either way"
    );
    assert_eq!(
        concurrent.1.1, sequential.1.1,
        "dst counters are the same either way"
    );
}

/// **A window boundary writes every row exactly once, with the digest computed.**
///
/// The other half of "same question", aimed at the part windowing alone could get
/// wrong. A digest is written through the batched handle once its window has
/// finished, so a row lost at a boundary would still produce a correct map for
/// *this* run and a stale cache for the *next* one — invisible to any test that
/// only reads the run's output.
///
/// So this asserts the persisted rows rather than the returned map, at the
/// tightest limits where the window is smallest and the boundaries most frequent.
#[test]
fn a_window_boundary_writes_every_row_exactly_once() {
    let t = TempRoot::new("rw_window");
    let (src, dst) = fixture(&t);
    let common = common::opts();

    drop_caches(&src, &dst);
    let ((smap, _), _) = resolve_overlapping(
        &src,
        &dst,
        &common,
        RwLimits::Split {
            src: nz(1),
            dst: nz(1),
        },
    );

    let rows = recs_of(&src);
    for (rel, rec) in &smap {
        let row = rows
            .get(rel)
            .unwrap_or_else(|| panic!("{rel} was hashed but left no cache row"));
        assert_eq!(
            rec.hashes, row.hashes,
            "{rel}: the row on disk disagrees with the map the run returned"
        );
        assert_eq!(rec.size, row.size, "{rel}: size disagrees");
        assert_eq!(rec.mtime_ns, row.mtime_ns, "{rel}: mtime disagrees");
    }
    // And nothing the tree does not hold was invented.
    for rel in rows.keys() {
        assert!(
            smap.contains_key(rel),
            "{rel} has a cache row but is not in the resolved map"
        );
    }
}

/// **Concurrent two-permit acquisition terminates, and admits only what it should.**
///
/// `split(1,1)` is the only shape in which a copy has to take *two* permits, src
/// before dst, and therefore the only one in which this crate can block a thread
/// on another's progress. If the acquisition order in
/// [`RwLimiter::acquire_copy`](girsync::rw::RwLimiter::acquire_copy) ever inverted,
/// this would hang rather than fail — so it guards a property nothing else can
/// observe.
///
/// The arrangement matters. Every thread reaches a barrier **before** asking for
/// its permit, so all eight are genuinely contending rather than accidentally
/// serialised by scheduling, and each holds what it took briefly so the others
/// have to wait. A barrier *after* acquiring would be a different test entirely:
/// under these limits exactly one thread can get past it, so it would deadlock
/// rather than measure anything.
///
/// The counter turns "it finished" into "it let everyone through". A run that
/// deadlocked would hang; one that quietly degraded to a single-permit copy would
/// let `peak` reach 2 and fail here.
#[test]
fn concurrent_copies_admit_only_what_the_limits_allow() {
    let threads = 8usize;
    let rw = RwRuntime::new(RwLimits::Split {
        src: nz(1),
        dst: nz(1),
    })
    .unwrap();
    assert_eq!(rw.limits().pool_size(), 2, "one worker per permit, no more");

    let start = Barrier::new(threads);
    let in_flight = AtomicUsize::new(0);
    let peak = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        let rw = &rw;
        for _ in 0..threads {
            let (start, in_flight, peak) = (&start, &in_flight, &peak);
            scope.spawn(move || {
                start.wait();
                let _copy = rw.acquire_copy();
                let now = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                std::thread::sleep(std::time::Duration::from_millis(5));
                in_flight.fetch_sub(1, Ordering::SeqCst);
            });
        }
    });

    assert_eq!(
        peak.load(Ordering::SeqCst),
        1,
        "split(1,1) admits exactly one copy: it needs both permits, and there is \
         one of each"
    );
    assert_eq!(
        rw.limiter().in_flight(RwSide::Src),
        0,
        "every permit came back, or the next copy would wait forever"
    );
    assert_eq!(rw.limiter().in_flight(RwSide::Dst), 0);
}

/// **A copy is bounded by its tighter side, because it draws on both.**
///
/// Under `split(2,1)` one copy leaves a spare src permit and no dst permit, so a
/// second copy cannot start even though src has room — the dst budget is what
/// binds. That is the observable consequence of a copy consuming *both* permits,
/// and it is what makes `--rw-dual-drive` (`split(1,1)`) run copies one at a time
/// while letting the two sides' hash phases overlap.
///
/// The *order* those two permits are taken in is not observable from here — no
/// external probe can tell `src → dst` from `dst → src` without instrumenting the
/// limiter. It is a structural invariant stated at
/// [`RwLimiter::acquire_copy`](girsync::rw::RwLimiter::acquire_copy), and what is
/// testable about it is that the wait-for graph it creates terminates, which is
/// what `concurrent_copies_admit_only_what_the_limits_allow` does.
#[test]
fn a_copy_is_bounded_by_its_tighter_side() {
    let rw = RwRuntime::new(RwLimits::Split {
        src: nz(2),
        dst: nz(1),
    })
    .unwrap();

    let _first = rw.acquire_copy();
    assert_eq!(rw.limiter().in_flight(RwSide::Src), 1, "src permit spent");
    assert_eq!(rw.limiter().in_flight(RwSide::Dst), 1, "dst permit spent");
    assert!(
        rw.limiter().try_acquire(RwSide::Src).is_some(),
        "src has a spare permit, so src is not what is binding"
    );
    assert!(
        rw.limiter().try_acquire(RwSide::Dst).is_none(),
        "dst is exhausted, so a second copy cannot start however much src has left"
    );

    drop(_first);
    assert!(
        rw.limiter().try_acquire(RwSide::Src).is_some(),
        "and both permits came back"
    );
    assert!(rw.limiter().try_acquire(RwSide::Dst).is_some());
}
