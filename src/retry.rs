//! One retry driver, backed by `backoff`'s exponential policy.
//!
//! The caller owns the classifier — a retcode table, an HTTP status rule, a
//! file-level failure — and answers [`Retry`] (or [`RetryState`], when work must
//! be carried across attempts) per attempt. The driver maps a transient answer
//! onto `backoff::Error::Transient` and a fatal answer onto `Permanent`, then
//! lets `backoff::future::retry` own the attempt loop and the sleeps.
//!
//! A retry may need to keep work across attempts (a scratch buffer, a byte
//! counter, a partially written file). [`RetryState`] and [`retry_state`] hand
//! that state back to a slot the driver's closure and its futures share, so the
//! attempt future can own it while it runs.

use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use backoff::backoff::{Backoff, Zero};
use backoff::future::retry as backoff_retry;
use backoff::{Error, ExponentialBackoffBuilder};

/// What waits between attempts.
///
/// [`BackoffPolicy::real`] is production: `backoff`'s exponential policy with
/// RNG jitter, so loops that start together de-synchronize through the random
/// spread instead of through a deterministic salt. [`BackoffPolicy::none`] is
/// for tests that count attempts rather than durations.
#[derive(Clone, Debug)]
pub struct BackoffPolicy {
    initial_interval: Duration,
    multiplier: f64,
    randomization_factor: f64,
    max_interval: Duration,
    /// `false` selects a zero-wait policy instead of the exponential one.
    real: bool,
}

impl BackoffPolicy {
    /// Production: exponential waits with ±50% jitter, from one second up to a
    /// thirty-second ceiling.
    pub fn real() -> Self {
        Self {
            initial_interval: Duration::from_secs(1),
            multiplier: 2.0,
            randomization_factor: 0.5,
            max_interval: Duration::from_secs(30),
            real: true,
        }
    }

    /// Tests: retry with no wait, so an attempt count can be asserted without
    /// spending the delay.
    pub fn none() -> Self {
        Self {
            initial_interval: Duration::ZERO,
            multiplier: 1.0,
            randomization_factor: 0.0,
            max_interval: Duration::ZERO,
            real: false,
        }
    }

    /// A fresh `Backoff` for one loop: `backoff::future::retry` consumes the
    /// policy and resets it, so each call needs its own instance.
    fn build(&self) -> Box<dyn Backoff + Send> {
        if !self.real {
            return Box::new(Zero {});
        }
        // The root alias builds an `ExponentialBackoff<SystemClock>`: the
        // builder's `C::default()` supplies the clock, so no caller-managed
        // clock handle is needed here.
        Box::new(
            ExponentialBackoffBuilder::new()
                .with_initial_interval(self.initial_interval)
                .with_multiplier(self.multiplier)
                .with_randomization_factor(self.randomization_factor)
                .with_max_interval(self.max_interval)
                .with_max_elapsed_time(None)
                .build(),
        )
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
/// pay for a wait before returning.
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
    let policy = backoff.build();
    let mut made = 0u32;
    backoff_retry(policy, || {
        made += 1;
        let attempt_no = made;
        let fut = attempt(attempt_no);
        async move {
            match fut.await {
                Retry::Done(v) => Ok(v),
                Retry::Fatal(e) => Err(Error::permanent(e)),
                Retry::Retry(e) if attempt_no >= attempts => Err(Error::permanent(
                    e.context(format!("after {attempts} attempts")),
                )),
                Retry::Retry(e) => Err(Error::transient(e)),
            }
        }
    })
    .await
}

/// Run `attempt` up to `attempts` times, handing `state` to each attempt and
/// taking it back on a retry.
///
/// `backoff::future::retry` re-invokes one closure and cannot carry a value
/// from one attempt's answer into the next. The captured-mutation adapter keeps
/// the state in a slot shared by the closure and its futures: each attempt
/// takes the state out, and a retry puts the state the attempt returned back
/// for the next call. A `Done` or `Fatal` answer ends the loop with the state
/// dropped; the backoff policy is the same as [`retry`].
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
    let policy = backoff.build();
    let slot = Arc::new(Mutex::new(Some(state)));
    let mut made = 0u32;
    backoff_retry(policy, || {
        made += 1;
        let attempt_no = made;
        let carried = slot
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
            .expect("retry state present for the next attempt");
        let fut = attempt(carried, attempt_no);
        let slot = Arc::clone(&slot);
        async move {
            match fut.await {
                RetryState::Done(v) => Ok(v),
                RetryState::Fatal(e) => Err(Error::permanent(e)),
                RetryState::Retry(next, e) => {
                    *slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(next);
                    if attempt_no >= attempts {
                        Err(Error::permanent(
                            e.context(format!("after {attempts} attempts")),
                        ))
                    } else {
                        Err(Error::transient(e))
                    }
                }
            }
        }
    })
    .await
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
