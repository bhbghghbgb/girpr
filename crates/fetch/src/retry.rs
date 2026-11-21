//! One retry driver, backed by the shared schedule and sleep mechanics.
//!
//! The caller owns the classifier — a retcode table, an HTTP status rule, a
//! file-level failure — and answers [`Retry`] (or [`RetryState`], when work must
//! be carried across attempts) per attempt. This module maps a transient answer
//! onto a wait from the [`Schedule`] and a fatal answer
//! onto an immediate return, then drives the attempts itself so the wait goes
//! through the injected [`Sleep`] hook.
//!
//! A retry may need to keep work across attempts (a scratch buffer, a byte
//! counter, a partially written file). [`RetryState`] and [`retry_state`] hand
//! that state back to a slot the driver's closure and its futures share, so the
//! attempt future can own it while it runs.

use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use girpr_retry::{DelaySequence, RealSleep, Schedule, Sleep, ZeroSleep};

/// What waits between attempts.
///
/// [`BackoffPolicy::real`] is production: `backoff`'s exponential schedule with
/// ±50 % jitter, so loops that start together de-synchronise through the random
/// spread instead of through a deterministic salt. [`BackoffPolicy::none`] is
/// for tests that count attempts rather than durations. Both the schedule and
/// the sleeper come from the shared leaf; this program only chooses the jitter
/// and the zero-wait mode.
#[derive(Clone, Debug)]
pub struct BackoffPolicy {
    schedule: Schedule,
    sleeper: Arc<dyn Sleep>,
}

impl BackoffPolicy {
    /// Production: exponential waits with ±50 % jitter, from one second up to a
    /// thirty-second ceiling.
    pub fn real() -> Self {
        Self {
            schedule: Schedule::exponential(
                Duration::from_secs(1),
                2.0,
                0.5,
                Duration::from_secs(30),
            ),
            sleeper: Arc::new(RealSleep),
        }
    }

    /// Tests: retry with no wait, so an attempt count can be asserted without
    /// spending the delay.
    pub fn none() -> Self {
        Self {
            schedule: Schedule::zero(),
            sleeper: Arc::new(ZeroSleep),
        }
    }

    /// A fresh delay sequence for one loop. Each `retry`/`retry_state` call
    /// needs its own instance because a backoff policy is stateful.
    fn build(&self) -> DelaySequence {
        self.schedule.build()
    }
}

/// One attempt's answer, when the attempt has no state to carry forward.
// `Retry` is the loop's answer and `Retry::Retry` is its natural third variant
// beside `Done` and `Fatal`. The module is private, so this name is internal,
// but the repetition is deliberate: renaming the variant would churn every
// call site for a lint about a name that already reads as intended.
#[allow(clippy::enum_variant_names)]
pub enum Retry<T> {
    /// The attempt succeeded.
    Done(T),
    /// The answer is final; an identical retry will be answered identically.
    Fatal(anyhow::Error),
    /// The attempt failed transiently. Worth another try.
    Retry(anyhow::Error),
}

/// One attempt's answer, with the state the next attempt should start from.
///
/// The variant carries the state so the attempt future can own it rather than
/// borrow it out of the loop.
pub enum RetryState<T, S> {
    /// The attempt succeeded.
    Done(T),
    /// The answer is final; an identical retry will be answered identically.
    Fatal(anyhow::Error),
    /// The attempt failed transiently; `state` is where the next one starts.
    Retry(S, anyhow::Error),
}

/// Run `attempt` up to `attempts` times.
///
/// - `Done` returns immediately;
/// - `Fatal` returns immediately, without another attempt;
/// - `Retry` waits for the policy's backoff and continues;
/// - after the last attempt, the last retry error is returned with
///   `after {attempts} attempts` as its context.
///
/// The wait is taken between attempts only: exhausting the attempts does not
/// pay for a wait before returning. The transient/permanent decision stays
/// here, at the caller's seam; the leaf only supplies delays and sleeps.
pub async fn retry<T, F, Fut>(
    attempts: u32,
    backoff: &BackoffPolicy,
    mut attempt: F,
) -> anyhow::Result<T>
where
    F: FnMut(u32) -> Fut,
    Fut: Future<Output = Retry<T>>,
{
    if attempts == 0 {
        return Err(anyhow::anyhow!("no attempts"));
    }
    let mut delays = backoff.build();
    for attempt_no in 1..=attempts {
        match attempt(attempt_no).await {
            Retry::Done(v) => return Ok(v),
            Retry::Fatal(e) => return Err(e),
            Retry::Retry(e) if attempt_no >= attempts => {
                return Err(e.context(format!("after {attempts} attempts")));
            }
            Retry::Retry(_) => {
                if let Some(wait) = delays.next_delay() {
                    backoff.sleeper.sleep(wait).await;
                }
            }
        }
    }
    Err(anyhow::anyhow!("no attempts"))
}

/// Run `attempt` up to `attempts` times, handing `state` to each attempt and
/// taking it back on a retry.
///
/// The shared leaf's delay sequence cannot carry a value from one attempt's
/// answer into the next. The captured-mutation adapter keeps the state in a
/// slot shared by the loop and its attempts: each attempt takes the state out,
/// and a retry puts the state the attempt returned back for the next call. A
/// `Done` or `Fatal` answer ends the loop with the state dropped; the waits are
/// the same as [`retry`].
#[allow(clippy::expect_used)] // the slot is always filled on entry and refilled on a retry
pub async fn retry_state<T, S, F, Fut>(
    attempts: u32,
    backoff: &BackoffPolicy,
    state: S,
    mut attempt: F,
) -> anyhow::Result<T>
where
    F: FnMut(S, u32) -> Fut,
    Fut: Future<Output = RetryState<T, S>>,
{
    if attempts == 0 {
        return Err(anyhow::anyhow!("no attempts"));
    }
    let mut delays = backoff.build();
    let slot = Arc::new(Mutex::new(Some(state)));
    for attempt_no in 1..=attempts {
        let carried = slot
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
            .expect("retry state present for the next attempt");
        match attempt(carried, attempt_no).await {
            RetryState::Done(v) => return Ok(v),
            RetryState::Fatal(e) => return Err(e),
            RetryState::Retry(next, e) => {
                *slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(next);
                if attempt_no >= attempts {
                    return Err(e.context(format!("after {attempts} attempts")));
                }
                if let Some(wait) = delays.next_delay() {
                    backoff.sleeper.sleep(wait).await;
                }
            }
        }
    }
    Err(anyhow::anyhow!("no attempts"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn success_on_the_first_try_stops_immediately() {
        let backoff = BackoffPolicy::none();
        let mut calls = 0;
        let value = retry(5, &backoff, |_n| {
            calls += 1;
            async { Retry::Done(7) }
        })
        .await
        .expect("the first attempt succeeds");
        assert_eq!(value, 7);
        assert_eq!(calls, 1);
    }

    #[tokio::test]
    async fn success_after_retries_keeps_the_attempt_numbers() {
        let backoff = BackoffPolicy::none();
        let mut seen = Vec::new();
        let value = retry(5, &backoff, |n| {
            seen.push(n);
            async move {
                if n < 3 {
                    Retry::Retry(anyhow::anyhow!("attempt {n}"))
                } else {
                    Retry::Done(n)
                }
            }
        })
        .await
        .expect("the third attempt succeeds");
        assert_eq!(value, 3);
        assert_eq!(seen, vec![1, 2, 3]);
    }

    #[tokio::test]
    async fn fatal_stops_after_one_attempt() {
        let backoff = BackoffPolicy::none();
        let mut calls = 0;
        let err = retry(5, &backoff, |_n| {
            calls += 1;
            async { Retry::<()>::Fatal(anyhow::anyhow!("refused")) }
        })
        .await
        .expect_err("a fatal answer is an error");
        assert_eq!(calls, 1, "a fatal answer must not be attempted again");
        assert_eq!(err.to_string(), "refused");
    }

    #[tokio::test]
    async fn exhausting_the_attempts_returns_the_last_error() {
        let backoff = BackoffPolicy::none();
        let mut calls = 0;
        let err = retry(5, &backoff, |n| {
            calls += 1;
            async move { Retry::<()>::Retry(anyhow::anyhow!("attempt {n}")) }
        })
        .await
        .expect_err("all attempts fail");
        assert_eq!(calls, 5);
        let chain = format!("{err:#}");
        assert!(chain.contains("after 5 attempts"), "{chain}");
        assert!(chain.contains("attempt 5"), "{chain}");
        assert!(
            !chain.contains("attempt 4"),
            "the last error is the one returned: {chain}"
        );
    }

    #[tokio::test]
    async fn state_is_carried_from_one_attempt_to_the_next() {
        let backoff = BackoffPolicy::none();
        let value = retry_state(
            5,
            &backoff,
            Vec::new(),
            |mut carried: Vec<u32>, n| async move {
                carried.push(n);
                if n < 3 {
                    RetryState::Retry(carried, anyhow::anyhow!("attempt {n}"))
                } else {
                    RetryState::Done(carried)
                }
            },
        )
        .await
        .expect("the third attempt succeeds");
        assert_eq!(value, vec![1, 2, 3]);
    }

    #[tokio::test]
    async fn a_fatal_state_answer_keeps_its_own_error() {
        let backoff = BackoffPolicy::none();
        let mut calls = 0;
        let err = retry_state(5, &backoff, 0u32, |n, attempt| {
            calls += 1;
            async move {
                let _ = n;
                if attempt == 2 {
                    RetryState::<(), u32>::Fatal(anyhow::anyhow!("final"))
                } else {
                    RetryState::Retry(n, anyhow::anyhow!("again"))
                }
            }
        })
        .await
        .expect_err("a fatal answer ends the loop");
        assert_eq!(err.to_string(), "final");
        // The closure is FnMut and its own counter is not the loop's; the loop
        // still stops on the fatal answer rather than exhausting its attempts.
        assert_eq!(calls, 2);
    }
}
