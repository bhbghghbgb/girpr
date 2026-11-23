//! girpr — GI low-disk repair patcher.
//!
//! Brings a possibly-corrupt, possibly-any-older-version GI install to the
//! current live version with minimal extra disk. Unattended by design: args in,
//! exit code + `REPORT`/`PROGRESS`/`SUMMARY` stdout lines out, no UI.
//!
//! Core = Starward's repair-in-chunk-mode (version-agnostic, per-file
//! `{path}_tmp` streaming repair, verified local-chunk reuse, atomic promote)
//! plus Collapse's extra-file purge as a pre/post phase. Background:
//! `docs/01-repair-findings-and-verdict.md`, spec:
//! `docs/02-repair-chunk-spec.md`, module plan:
//! `docs/03-repair-rust-implementation-plan.md`.
//!
//! ## Public surface
//!
//! The binary is the product, so the library exposes one deliberate seam:
//!
//! - [`run`] — the whole pipeline, and its configuration ([`RunCtx`]) and
//!   typed failure ([`RunFailure`]);
//! - [`Summary`], the types its public fields name ([`SummaryFlags`], [`Meter`],
//!   [`Ceiling`]), [`OutputFormat`], [`Record`], [`emit`](fn@emit) and
//!   [`summary_record`] — the run counters and the stdout automation contract;
//! - [`Args`] plus the audio helpers — the command-line parser;
//! - [`Biz`] — the game/channel selector;
//! - [`init_tracing`], [`flush_log`], [`log_file_announcement`], [`Level`] and
//!   [`girpr_log`] — the logging entry points and the policy type `init_tracing`
//!   returns.
//!
//! Everything else is an implementation detail. Integration tests reach the
//! internal types they construct through [`test_support`], a hidden seam with
//! a narrow, documented list.
//!
//! The modules behind the seam are private: `cli` / `config` (the argument
//! surface and validated per-run configuration), `logging` (console + file
//! `tracing` layers over the shared `girpr-log` policy), `repair` (the
//! pipeline, split into phases) and `biz` (biz → launcher id / game id /
//! channel tuple). The run counters, the record data plane, every stdout line
//! and the monotonic phase timer live in `girpr-report`, which also owns the
//! `Meter`/`StoppedBy` group that was mutually recursive with `Summary`; the
//! HYP/Sophon fetch path (`hyp`, `sophon`) and the attempt loop (`retry`)
//! live in `girpr-fetch`, the audio scan-file boundary in `girpr-audio`, and the
//! low-level scan/hash/path/ini/counter mechanics in narrow `girpr-util-*`
//! leaves. This crate depends on all of them directly under module aliases, so
//! edits here leave their code cached, and an edit to one leaf leaves the others
//! cached too.
//!
//! The binary (`main`, a thin shim) is: parse → install tracing → build the
//! run config → [`run`] → print `SUMMARY` → flush the log → return the exit
//! code.

mod biz;
mod cli;
mod config;
mod logging;
mod repair;
// The canned-reply loopback origin the unit tests share.
#[cfg(test)]
mod test_http;
// The shared loopback origin behind the repair phases' route-based tests.
#[cfg(test)]
mod test_origin;
// The report plane lives in its own crate so an edit to the repair modules
// below leaves the counters, the record data plane, every stdout line and the
// phase timer cached. These aliases keep every existing `crate::report::…` and
// `crate::phase::…` call site pointing at the same names, while this facade
// depends on the crate directly rather than only re-exporting it.
use girpr_report as phase;
use girpr_report as report;
// The fetch path lives in its own crate so an edit to `repair`, `report` or
// `cli` leaves the whole reqwest/zstd/prost tree cached. These aliases keep
// every existing `crate::hyp::…`, `crate::sophon::…` and `crate::retry::…`
// call site pointing at the same names, while the facade depends on the crate
// directly rather than only re-exporting it.
use girpr_fetch::{hyp, retry, sophon};
// The low-level mechanics live in narrow leaf crates so an edit to `repair`,
// `report` or `cli` leaves them cached, and an edit to one leaf leaves the
// others cached. This local alias keeps every existing `crate::util::…` call
// site pointing at the same names; there is no facade crate to force an edit
// to one leaf through every consumer.
mod util {
    pub use girpr_util_hash::*;
    pub use girpr_util_ini::*;
    pub use girpr_util_path::*;
}

// The log policy crate, re-exported so that a caller can name the type
// [`init_tracing`] returns without adding a dependency of its own: the policy
// is part of this crate's public signature, so it is part of this crate's
// public surface.
pub use girpr_log;

pub use biz::Biz;
pub use cli::{Args, is_audio_none, normalize_audio_lang, parse_audio_selection};
pub use config::{RunCtx, RunExit, RunFailure};
pub use girpr_log::Level;
pub use girpr_report::{
    Ceiling, OutputFormat, Record, Summary, SummaryFlags, emit, summary_record,
};
pub use logging::{flush_log, init_tracing, log_file_announcement};
pub use repair::run;
pub use repair::scope::ledger::Meter;

/// The tests' seam.
///
/// These items are implementation details. They are public only so that the
/// integration tests can construct a run and inspect its results without
/// importing a private module, and so that a unit test can share a helper with
/// them. Nothing here is production API.
///
/// The list is deliberately explicit:
///
/// - `Lock`, `lock_path`, `takes_lock` and `Limits` let a test build a
///   [`RunCtx`], drive the real pipeline, and inspect the lock policy and the
///   file a run leaves behind. `RepairMode` and `PathScope` are not re-exported
///   here: they are values of the `girpr-repair-policy` and
///   `girpr-repair-plan` leaves, which tests name directly just as the
///   production callers do;
/// - `BackoffPolicy` chooses the retry wait (real or immediate) without a test
///   sleeping for real;
/// - `SophonChunk`, `SophonChunkFile` and `SophonChunkManifest` build the
///   offline mock's manifests; `ChunkFetch`, `ChunkSink` and `stream_chunk_to`
///   are what the peak-heap probe measures;
/// - `join_url` resolves a prefix into a `reqwest::Url` and appends an id, the
///   way the fetches do, so `tests/repair_offline.rs` builds an advertised
///   manifest's URL through it rather than restating the join;
/// - `EMPTY_MD5` is the digest the fixtures use for an absent chunk;
/// - `terminal_retcode` is the retcode classifier the fixture builds its
///   terminal failures with, so a test cannot disagree with the client about
///   which codes are final;
/// - `stamp_ago`, `stamp_ago_from`, `stamp_ahead` and `stamp_ahead_from`
///   build host-relative fixture timestamps, so the repair's age rule is not
///   tested against a fixed epoch that a clock could pass;
/// - `FixtureRefusal` and `skip_fixture_refusal` report a fixture this host
///   cannot create, so a case skips out loud unless an opt-in variable makes
///   that refusal a failure;
/// - `RepairDiagnostics` and `take_repair_diagnostics` are the structured
///   decisions an in-process run made (write direction, salvage figures, trim
///   and tier choices, in-place chunk accounting), so a test can assert on
///   them without spawning the binary and parsing its log file. The record is
///   thread-local and read only here; a shipped run never looks at it.
///
/// `Record` and `summary_record` repeat the root exports on purpose: a test can
/// import everything it needs from this one module. That
/// repetition is a convenience of the test seam, not a second API.
#[doc(hidden)]
pub mod test_support {
    use std::time::{Duration, SystemTime};

    pub use crate::hyp::terminal_retcode;
    pub use crate::repair::diagnostics::RepairDiagnostics;
    pub use crate::repair::scope::Limits;
    pub use crate::report::{Record, summary_record};
    pub use crate::retry::BackoffPolicy;
    pub use crate::sophon::stream::join_url;
    pub use crate::sophon::{
        ChunkFetch, ChunkSink, SophonChunk, SophonChunkFile, SophonChunkManifest, stream_chunk_to,
    };
    pub use crate::util::EMPTY_MD5;
    pub use girpr_lock::{Lock, lock_path, takes_lock};

    // One refusal type across the product's tests and the path leaf's
    // unit-test-only fixture helpers, so a test can pass its seam value
    // straight into the junction/symlink classifiers.
    pub use girpr_util_path::test_support::{FixtureRefusal, skip_fixture_refusal};

    /// `now` minus `age`, or `None` when this platform cannot represent the
    /// result.
    ///
    /// The subtraction is **not** floored at the Unix epoch: a clock on either
    /// side of the epoch is aged by the requested `age`, so a fixture's
    /// "older than one coarse tick" property holds on a pre-epoch clock too.
    /// `None` is the only answer when the platform cannot reach that far back;
    /// a caller must then skip the case rather than run it with a stamp that
    /// is younger than the one it asked for.
    ///
    /// The result is never later than `now`; an `age` the platform can
    /// represent as a distinct offset makes it strictly older.
    pub fn stamp_ago_from(now: SystemTime, age: Duration) -> Option<SystemTime> {
        now.checked_sub(age)
    }

    /// [`stamp_ago_from`] relative to the current clock.
    pub fn stamp_ago(age: Duration) -> Option<SystemTime> {
        stamp_ago_from(SystemTime::now(), age)
    }

    /// Drain the structured decisions the most recent in-process run on this
    /// thread recorded, leaving a fresh record behind.
    ///
    /// Calling this before a run clears any leftovers; calling it after one
    /// reads exactly that run's decisions. A shipped run writes to the same
    /// record but never calls this, so nothing it emits changes.
    ///
    /// Records are attributed to the thread that started the repair loop, so
    /// they survive file tasks being moved onto the blocking pool. A caller
    /// must drain on that same thread, which a current-thread runtime (the
    /// default for `#[tokio::test]`) guarantees.
    pub fn take_repair_diagnostics() -> RepairDiagnostics {
        crate::repair::diagnostics::take()
    }

    /// `now` plus `age`, or `None` when this platform cannot represent the
    /// result.
    ///
    /// The mirror of [`stamp_ago_from`]. `None` matters here for the same
    /// reason: clamping to the current clock would turn an intended
    /// future-dated fixture into an ordinary one without saying so.
    pub fn stamp_ahead_from(now: SystemTime, age: Duration) -> Option<SystemTime> {
        now.checked_add(age)
    }

    /// [`stamp_ahead_from`] relative to the current clock.
    pub fn stamp_ahead(age: Duration) -> Option<SystemTime> {
        stamp_ahead_from(SystemTime::now(), age)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::clock_support::skip_pre_epoch;

        /// The age is exact on both sides of the epoch and across it, and the
        /// clock itself is never handed back as if it were older. A pre-epoch
        /// clock is the case that used to silently lose the age, and a clock
        /// shortly after the epoch is the other direction of the same
        /// crossing.
        ///
        /// Both crossings need a moment before the epoch, so each one is
        /// built with a checked subtraction: a host whose clock cannot
        /// represent that moment skips the case that needs it instead of
        /// panicking on it.
        #[test]
        fn an_ago_stamp_keeps_its_age_on_either_side_of_the_epoch_and_across_it_backward() {
            let day = Duration::from_secs(24 * 60 * 60);

            let now = SystemTime::UNIX_EPOCH + Duration::from_secs(3 * day.as_secs());
            assert_eq!(
                stamp_ago_from(now, day),
                now.checked_sub(day),
                "an ordinary clock gets exactly the requested age"
            );

            // A clock before the epoch. The instant is built with a checked
            // subtraction, so a host whose clock cannot represent it says so
            // instead of panicking on it.
            let before_epoch = SystemTime::UNIX_EPOCH.checked_sub(Duration::from_secs(60));
            let aged = before_epoch.and_then(|clock| stamp_ago_from(clock, day));
            let expected = before_epoch.and_then(|clock| clock.checked_sub(day));
            match (before_epoch, aged, expected) {
                (Some(before_epoch), Some(aged), Some(expected)) => {
                    assert_eq!(
                        aged, expected,
                        "a pre-epoch clock gets the requested age, not itself"
                    );
                    assert!(
                        aged < before_epoch,
                        "an ago stamp is never in the clock's future"
                    );
                }
                _ => skip_pre_epoch("a clock a minute before the epoch with a day of history"),
            }

            // A clock shortly after the epoch is the other way the age can
            // cross it: the subtraction reaches back past the epoch, so a
            // floor there would return a stamp younger than requested.
            let after_epoch = SystemTime::UNIX_EPOCH + Duration::from_secs(60);
            match (
                stamp_ago_from(after_epoch, day),
                after_epoch.checked_sub(day),
            ) {
                (Some(aged), Some(expected)) => {
                    assert_eq!(
                        aged, expected,
                        "a clock less than the age past the epoch is aged past it, not clamped to it"
                    );
                    assert!(
                        aged < SystemTime::UNIX_EPOCH && aged < after_epoch,
                        "the backward crossing lands before the epoch and before the clock"
                    );
                }
                _ => skip_pre_epoch("a clock a minute after the epoch with a day of history"),
            }
        }

        /// A platform that cannot reach the age reports it instead of passing
        /// the current clock off as an old stamp.
        #[test]
        fn an_unrepresentable_ago_stamp_is_reported_not_clamped() {
            assert_eq!(
                stamp_ago_from(SystemTime::UNIX_EPOCH, Duration::from_secs(u64::MAX)),
                None,
                "an age no platform can represent is not the current clock"
            );
        }

        /// The mirror helper is exact, and an offset the platform cannot
        /// represent is reported rather than clamped to the clock.
        #[test]
        fn an_ahead_stamp_is_exact_or_reported() {
            let now = SystemTime::UNIX_EPOCH + Duration::from_secs(60);
            assert_eq!(
                stamp_ahead_from(now, Duration::from_secs(1)),
                Some(now + Duration::from_secs(1))
            );
            assert_eq!(
                stamp_ahead_from(SystemTime::UNIX_EPOCH, Duration::from_secs(u64::MAX)),
                None,
                "an offset no platform can represent is not the current clock"
            );
        }
    }
}

/// The clock fixtures' skip, shared by the unit tests that need an instant the
/// host's `SystemTime` may not be able to represent.
#[cfg(test)]
pub(crate) mod clock_support {
    /// Report that a host cannot represent an instant a clock fixture needs,
    /// or fail when `GIRPR_TEST_REQUIRE_PRE_EPOCH` says the case must not be
    /// skipped.
    ///
    /// A run meant to prove the backward crossing must not pass having
    /// checked nothing, so the presence-based switch turns the skip into a
    /// failure. The printed line keeps the `SKIP precondition:` wording the
    /// runner is configured to surface.
    pub(crate) fn skip_pre_epoch(subject: &str) {
        skip_pre_epoch_decision(
            std::env::var_os("GIRPR_TEST_REQUIRE_PRE_EPOCH").is_some(),
            subject,
        );
    }

    fn skip_pre_epoch_decision(required: bool, subject: &str) {
        assert!(
            !required,
            "GIRPR_TEST_REQUIRE_PRE_EPOCH is set, but this host cannot represent {subject}"
        );
        eprintln!(
            "SKIP precondition: this host cannot represent {subject}; set \
             GIRPR_TEST_REQUIRE_PRE_EPOCH (to any value) to fail instead"
        );
    }
}
