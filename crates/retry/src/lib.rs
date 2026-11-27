//! The two mechanical parts of a retry loop: how to compute the next delay, and
//! how to wait for it.
//!
//! Everything that makes a retry loop *someone's own* stays with the caller:
//! which answers are transient, which are final, what work is carried between
//! attempts, and what side effects (a request log, a retained error body) an
//! attempt leaves behind. This crate owns only the part that is the same in
//! every loop.
//!
//! [`Schedule`] builds a delay sequence from four numbers the caller chooses:
//! a base delay, a multiplier, a jitter factor and a cap. Jitter is a parameter
//! and not a policy on purpose. One tool wanting RNG de-synchronisation passes
//! `0.5`; a tool that needs byte-stable artifacts passes `0.0`. [`Sleep`] is the
//! hook the caller injects to wait out a delay: [`RealSleep`] waits (yielding
//! the runtime), while [`ZeroSleep`] and [`RecordingSleep`] keep zero-wait tests
//! honest.

use std::fmt;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use backoff::ExponentialBackoffBuilder;
use backoff::backoff::{Backoff, Zero};

/// Whether an HTTP status is worth another attempt.
///
/// Retried: `408` (request timeout), `429` (too many requests) and any `5xx`
/// (`500..=599`) — a server that is overloaded or briefly broken may answer
/// again on a second try. Everything else is final: a `4xx` means the caller
/// asked for something a retry cannot conjure, and success never reaches this
/// predicate.
///
/// The parameter is a plain `u16` on purpose: callers pass the raw status they
/// received, and the rule is decided here with an exact `500..=599` range rather
/// than a "server error" helper whose acceptance of values outside that range
/// differs between HTTP libraries. Both tools call this one spelling, so their
/// published retry counts cannot drift apart.
pub fn should_retry_status(status: u16) -> bool {
    matches!(status, 408 | 429) || (500..=599).contains(&status)
}

/// A caller-parameterised schedule for the waits between attempts.
///
/// A fresh [`DelaySequence`] is built per loop: a `backoff` policy is stateful,
/// and loops can run concurrently, so the schedule itself holds no delay state.
#[derive(Clone, Debug)]
pub struct Schedule {
    base_delay: Duration,
    factor: f64,
    jitter: f64,
    cap: Duration,
    zero: bool,
}

impl Schedule {
    /// An exponential schedule: `base_delay`, multiplied by `factor` each time,
    /// spread by `jitter`, and clamped at `cap`.
    ///
    /// `jitter` is the caller's decision, not a fixed policy here. `0.0` keeps
    /// every delay exact (needed where artifacts must be byte-stable); `0.5`
    /// spreads each delay by ±50 % so loops that start together de-synchronise
    /// through random wait times.
    pub fn exponential(base_delay: Duration, factor: f64, jitter: f64, cap: Duration) -> Self {
        Self {
            base_delay,
            factor,
            jitter,
            cap,
            zero: false,
        }
    }

    /// A schedule that never waits, for tests that count attempts rather than
    /// spend delays.
    pub fn zero() -> Self {
        Self {
            base_delay: Duration::ZERO,
            factor: 1.0,
            jitter: 0.0,
            cap: Duration::ZERO,
            zero: true,
        }
    }

    /// The stateful delay sequence for one retry loop.
    pub fn build(&self) -> DelaySequence {
        if self.zero {
            return DelaySequence(Box::new(Zero {}));
        }
        DelaySequence(Box::new(
            ExponentialBackoffBuilder::new()
                .with_initial_interval(self.base_delay)
                .with_multiplier(self.factor)
                .with_randomization_factor(self.jitter)
                .with_max_interval(self.cap)
                // The caller bounds attempts. A wall-clock cutoff would add a
                // second bound and a source of nondeterminism for an injected
                // sleeper that need not advance real time at all.
                .with_max_elapsed_time(None)
                .build(),
        ))
    }
}

/// The stateful view of a [`Schedule`] inside one retry loop.
///
/// Callers get the next delay through [`DelaySequence::next_delay`] and need no
/// knowledge of `backoff` or its `Backoff` trait, so the dependency stays in
/// this leaf.
pub struct DelaySequence(Box<dyn Backoff + Send>);

impl DelaySequence {
    /// The delay before the next attempt. `None` means the schedule has ended;
    /// schedules built here never set a finite elapsed-time cap, so a loop that
    /// bounds its own attempts always gets a delay before a further attempt.
    pub fn next_delay(&mut self) -> Option<Duration> {
        self.0.next_backoff()
    }
}

/// Something a retry loop can wait on, so tests can count delays instead of
/// spending them.
///
/// Async on purpose: a blocking `std::thread::sleep` would hold a runtime worker
/// for the whole backoff, so requests that are in flight could not progress
/// while one loop waited. [`RealSleep`] yields instead. The attribute keeps the
/// trait usable as `dyn Sleep`, which is how callers inject a driver without
/// parameterising their own types over it.
#[async_trait]
pub trait Sleep: Send + Sync + fmt::Debug {
    /// Wait for `d`.
    async fn sleep(&self, d: Duration);
}

/// Real wall-clock waiting, without taking the worker with it.
#[derive(Debug, Default)]
pub struct RealSleep;

#[async_trait]
impl Sleep for RealSleep {
    async fn sleep(&self, d: Duration) {
        if !d.is_zero() {
            tokio::time::sleep(d).await;
        }
    }
}

/// A sleeper that never waits.
///
/// Test-only: a caller selects it when a spawned process or an in-process test
/// must exercise a real retry without paying a production backoff. Unlike
/// [`RecordingSleep`] it records nothing, so a child process need not hand a
/// delay log back to its parent.
#[derive(Debug, Default)]
pub struct ZeroSleep;

#[async_trait]
impl Sleep for ZeroSleep {
    async fn sleep(&self, _d: Duration) {}
}

/// Records what it was asked to wait for and returns immediately.
#[derive(Debug, Default)]
pub struct RecordingSleep(Mutex<Vec<Duration>>);

impl RecordingSleep {
    /// Every delay requested so far.
    pub fn delays(&self) -> Vec<Duration> {
        self.lock().clone()
    }

    /// Recover from poisoning: a recorded delay is evidence about a retry
    /// schedule, and losing it to an unrelated panic is worse than reading a
    /// vector that was fully written before the panic.
    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Duration>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[async_trait]
impl Sleep for RecordingSleep {
    async fn sleep(&self, d: Duration) {
        // The macro boxes the method body, so this push happens on first poll
        // rather than at call time. Every caller awaits the sleep before reading
        // `delays()`, so the recorded sequence is unchanged.
        self.lock().push(d);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The retryable-status rule at its boundaries: `408`/`429` and the whole
    /// `5xx` band retry; `407`, the rest of `4xx`, `600` and values outside the
    /// HTTP range do not.
    #[test]
    fn should_retry_status_pins_the_boundaries() {
        for s in [408, 429, 500, 502, 503, 504, 599] {
            assert!(should_retry_status(s), "{s} must be retried");
        }
        for s in [
            0,
            99,
            100,
            200,
            204,
            301,
            400,
            401,
            403,
            404,
            407,
            410,
            451,
            600,
            999,
            1000,
            u16::MAX,
        ] {
            assert!(!should_retry_status(s), "{s} must not be retried");
        }
    }

    /// The three `Schedule` contract cases, folded into one process so nextest
    /// spawns one test instead of three. Every original assertion and message is
    /// kept; a labelled subcase names the schedule that failed.
    #[test]
    fn schedule_contracts() {
        let subcases: &[(&str, &dyn Fn())] = &[
            ("zero_jitter_schedule_is_exact_and_capped", &|| {
                let mut delays = Schedule::exponential(
                    Duration::from_millis(500),
                    2.0,
                    0.0,
                    Duration::from_secs(10),
                )
                .build();
                for expected in [
                    Duration::from_millis(500),
                    Duration::from_millis(1000),
                    Duration::from_millis(2000),
                    Duration::from_millis(4000),
                    Duration::from_millis(8000),
                    Duration::from_secs(10),
                ] {
                    assert_eq!(
                        delays.next_delay(),
                        Some(expected),
                        "zero jitter: expected {expected:?}"
                    );
                }
                for _ in 0..8 {
                    assert_eq!(
                        delays.next_delay(),
                        Some(Duration::from_secs(10)),
                        "zero jitter: capped at 10 s"
                    );
                }
            }),
            ("jitter_spreads_each_delay_within_the_caller_range", &|| {
                let cap = Duration::from_secs(10);
                let mut delays =
                    Schedule::exponential(Duration::from_secs(1), 2.0, 0.5, cap).build();
                // `backoff` spreads the *current* interval, which doubles
                // each call up to the cap; the jitter is ±50 % of that
                // interval, not of the base.
                let mut current = Duration::from_secs(1);
                for _ in 0..16 {
                    let d = delays.next_delay().expect("no elapsed-time cap");
                    let low = current.mul_f64(0.5);
                    let high = current.mul_f64(1.5) + Duration::from_nanos(1);
                    assert!(low <= d && d <= high, "±50 % of {current:?}: {d:?}");
                    current = if current >= cap.div_f64(2.0) {
                        cap
                    } else {
                        current * 2
                    };
                }
            }),
            ("zero_schedule_never_waits", &|| {
                let mut delays = Schedule::zero().build();
                for _ in 0..8 {
                    assert_eq!(
                        delays.next_delay(),
                        Some(Duration::ZERO),
                        "the zero schedule never waits"
                    );
                }
            }),
        ];
        for (label, run) in subcases {
            println!("subcase: {label}");
            run();
        }
    }

    #[tokio::test]
    async fn zero_and_recording_sleepers_return_immediately() {
        tokio::time::timeout(
            Duration::from_millis(100),
            ZeroSleep.sleep(Duration::from_secs(60)),
        )
        .await
        .expect("ZeroSleep must never wait");
        let recorder = RecordingSleep::default();
        tokio::time::timeout(
            Duration::from_millis(100),
            recorder.sleep(Duration::from_secs(60)),
        )
        .await
        .expect("RecordingSleep must never wait");
        assert_eq!(recorder.delays(), vec![Duration::from_secs(60)]);
    }
}
