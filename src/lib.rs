//! girpr — Genshin Impact low-disk repair patcher.
//!
//! Brings a possibly-corrupt, possibly-any-older-version Genshin install to the
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
//! - [`init_tracing`], [`flush_log`], [`Level`] and [`girpr_log`] — the logging
//!   entry points and the policy type `init_tracing` returns.
//!
//! Everything else is an implementation detail. Integration tests reach the
//! internal types they construct through [`test_support`], a hidden seam with
//! a narrow, documented list.
//!
//! The modules behind the seam are private: `cli` / `config` (the argument
//! surface and validated per-run configuration), `logging` (console + file
//! `tracing` layers over the shared `girpr-log` policy), `repair` (the
//! pipeline, split into phases), `hyp` (HoYoPlay + Sophon `getBuild` JSON
//! APIs), `sophon` (chunk-manifest protobuf, fetch/verify/parse, manifest
//! filter, local chunk-reuse map), `biz` (biz → launcher id / game id /
//! channel tuple), `report` (run counters, the record data plane and every
//! stdout line), `retry` (the shared attempt/backoff loop), `sleep` (the
//! injectable wait), `phase` (the monotonic phase timer) and `util` (MD5, `config.ini`,
//! ignore/blacklist files, server-path containment).
//!
//! The binary (`main`, a thin shim) is: parse → install tracing → build the
//! run config → [`run`] → print `SUMMARY` → flush the log → return the exit
//! code.

mod biz;
mod cli;
mod config;
mod hyp;
mod logging;
mod phase;
mod repair;
mod report;
mod retry;
mod sleep;
mod sophon;
// The canned-reply loopback origin the unit tests share.
#[cfg(test)]
mod test_http;
mod util;

// The log policy crate, re-exported so that a caller can name the type
// [`init_tracing`] returns without adding a dependency of its own: the policy
// is part of this crate's public signature, so it is part of this crate's
// public surface.
pub use girpr_log;

pub use biz::Biz;
pub use cli::{Args, is_audio_none, normalize_audio_lang, parse_audio_selection};
pub use config::{RunCtx, RunExit, RunFailure};
pub use girpr_log::Level;
pub use logging::{flush_log, init_tracing};
pub use repair::run;
pub use repair::scope::ledger::Meter;
pub use report::{Ceiling, OutputFormat, Record, Summary, SummaryFlags, emit, summary_record};

/// The tests' seam.
///
/// These items are implementation details. They are public only so that the
/// integration tests can construct a run and inspect its results without
/// importing a private module, and so that a unit test can share a helper with
/// them. Nothing here is production API.
///
/// The list is deliberately explicit:
///
/// - `RepairMode`, `Lock`, `lock_path`, `takes_lock`, `Limits` and `PathScope`
///   let a test build a [`RunCtx`], drive the real pipeline, and inspect the
///   lock policy and the file a run leaves behind;
/// - `NoSleep` and `RealSleep` choose the wait policy without a test sleeping
///   for real;
/// - `SophonChunk`, `SophonChunkFile` and `SophonChunkManifest` build the
///   offline mock's manifests; `ChunkFetch`, `ChunkSink` and `stream_chunk_to`
///   are what the peak-heap probe measures;
/// - `join_url` is the one URL join the fetches use, so `tests/repair_offline.rs`
///   builds an advertised manifest's URL through it rather than restating the
///   formula;
/// - `EMPTY_MD5` is the digest the fixtures use for an absent chunk;
/// - `terminal_retcode` is the retcode classifier the fixture builds its
///   terminal failures with, so a test cannot disagree with the client about
///   which codes are final;
/// - `stamp_ago`, `stamp_ago_from`, `stamp_ahead` and `stamp_ahead_from`
///   build host-relative fixture timestamps, so the repair's age rule is not
///   tested against a fixed epoch that a clock could pass;
/// - `FixtureRefusal` and `skip_fixture_refusal` report a fixture this host
///   cannot create, so a case skips out loud unless an opt-in variable makes
///   that refusal a failure.
///
/// `Record` and `summary_record` repeat the root exports on purpose: a test can
/// import everything it needs from this one module. That
/// repetition is a convenience of the test seam, not a second API.
#[doc(hidden)]
pub mod test_support {
    use std::time::{Duration, SystemTime};

    pub use crate::config::RepairMode;
    pub use crate::hyp::terminal_retcode;
    pub use crate::repair::lock::{Lock, lock_path, takes_lock};
    pub use crate::repair::scope::{Limits, PathScope};
    pub use crate::report::{Record, summary_record};
    pub use crate::sleep::{NoSleep, RealSleep};
    pub use crate::sophon::stream::join_url;
    pub use crate::sophon::{
        ChunkFetch, ChunkSink, SophonChunk, SophonChunkFile, SophonChunkManifest, stream_chunk_to,
    };
    pub use crate::util::EMPTY_MD5;

    /// Where a refusal to create a host-dependent test fixture came from.
    ///
    /// A host that cannot create one and a test that injected the failure both
    /// take the skip branch, but only the first is a skipped case. The printed
    /// line says which, and only the real skip keeps the `SKIP precondition:`
    /// wording the runner is configured to surface.
    #[derive(Clone, Copy)]
    pub enum FixtureRefusal {
        /// The host refused the fixture.
        Host,
        /// The test supplied the refusal; no host behaviour was observed.
        Injected,
    }

    impl FixtureRefusal {
        fn lead(self, subject: &str) -> String {
            match self {
                Self::Host => format!("SKIP precondition: this host cannot create {subject}"),
                Self::Injected => format!(
                    "simulated refusal: no host was consulted; a host that cannot create \
                     {subject} would skip"
                ),
            }
        }

        /// What the required-case failure says about where the refusal came
        /// from. An injected refusal never consulted the host, so blaming the
        /// host there would name a cause the test did not observe.
        fn failure_lead(self, subject: &str) -> String {
            match self {
                Self::Host => format!("this host cannot create {subject}"),
                Self::Injected => {
                    format!("the refusal to create {subject} was injected; no host was consulted")
                }
            }
        }
    }

    /// Report that a host-dependent fixture could not be created, or fail when
    /// `required_env` says the case must not be skipped.
    ///
    /// `subject` names the fixture (`a directory junction`, `a non-UTF-8
    /// name`) and `detail` is the reason the attempt failed. A run meant to
    /// prove the fixture must not pass having checked nothing, so the
    /// presence-based switch named by `required_env` turns the skip into a
    /// failure. Only a host refusal blames the host: a required injected
    /// refusal fails saying the refusal was injected and no host was
    /// consulted.
    pub fn skip_fixture_refusal(
        refusal: FixtureRefusal,
        subject: &str,
        required_env: &str,
        detail: impl std::fmt::Display,
        required: bool,
    ) {
        assert!(
            !required,
            "{required_env} is set, but {}: {detail}",
            refusal.failure_lead(subject)
        );
        eprintln!(
            "{}: {detail}; set {required_env} (to any value) to fail instead",
            refusal.lead(subject)
        );
    }

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

    mod tests {
        use super::*;

        /// Without the switch, a host that cannot represent the instant skips
        /// the clock case out loud instead of failing it.
        #[test]
        fn a_pre_epoch_fixture_skips_without_the_switch() {
            skip_pre_epoch_decision(false, "a clock a minute before the epoch");
        }

        /// The same host is a failure when the switch is set, whatever its
        /// value: the case must not pass having proved nothing.
        #[test]
        #[should_panic(expected = "GIRPR_TEST_REQUIRE_PRE_EPOCH is set")]
        fn a_pre_epoch_fixture_fails_with_the_switch() {
            skip_pre_epoch_decision(true, "a clock a minute before the epoch");
        }
    }
}
