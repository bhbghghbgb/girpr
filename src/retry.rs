//! One retry loop, with the decision to retry left to the caller.
//!
//! The loop owns the parts every retrying fetch was writing for itself:
//! attempts, the linear backoff, and which error is the last one. The caller
//! owns the classifier — a retcode table, an HTTP status rule, a file-level
//! failure — and answers [`Retry`] or [`RetryState`] per attempt.
//!
//! A retry may need to keep work across attempts (a scratch buffer, a byte
//! counter, a partially written file). [`RetryState`] and [`retry_state`] hand
//! that state back to the loop on a retry, so the attempt future can own it
//! while it runs instead of borrowing it from the closure.

use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use crate::sleep::Sleep;

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
/// - `Retry` sleeps for the attempt's backoff and continues;
/// - after the last attempt, the last retry error is returned with
///   `after {attempts} attempts` as its context.
///
/// The sleep is taken between attempts only: exhausting the attempts does not
/// pay for a wait before returning.
pub async fn retry<T, F, Fut>(attempts: u32, sleep: &dyn Sleep, attempt: F) -> anyhow::Result<T>
where
    F: FnMut(u32) -> Fut,
    Fut: Future<Output = Retry<T>>,
{
    let mut attempt = attempt;
    // The closure's own state is the state the loop threads, so the adapter
    // below does not have to borrow it out of a captured variable.
    retry_state(attempts, sleep, &mut attempt, |attempt, n| async move {
        match attempt(n).await {
            Retry::Done(v) => RetryState::Done(v),
            Retry::Fatal(e) => RetryState::Fatal(e),
            Retry::Retry(e) => RetryState::Retry(attempt, e),
        }
    })
    .await
}

/// Run `attempt` up to `attempts` times, handing `state` to each attempt and
/// taking it back on a retry.
///
/// A `Done` or `Fatal` answer ends the loop; the state is dropped with the
/// successful result or the error. The backoff policy is the same as
/// [`retry`].
pub async fn retry_state<T, S, F, Fut>(
    attempts: u32,
    sleep: &dyn Sleep,
    mut state: S,
    mut attempt: F,
) -> anyhow::Result<T>
where
    F: FnMut(S, u32) -> Fut,
    Fut: Future<Output = RetryState<T, S>>,
{
    let mut last_err: Option<anyhow::Error> = None;
    let salt = next_salt();
    for n in 1..=attempts {
        match attempt(state, n).await {
            RetryState::Done(v) => return Ok(v),
            RetryState::Fatal(e) => return Err(e),
            RetryState::Retry(next, e) => {
                state = next;
                last_err = Some(e);
            }
        }
        if n < attempts {
            sleep.sleep(backoff(n, salt)).await;
        }
    }
    Err(last_err
        .unwrap_or_else(|| anyhow::anyhow!("no attempts"))
        .context(format!("after {attempts} attempts")))
}

/// Distinguishes one retry loop from every other loop in this process.
static NEXT_SALT: AtomicU64 = AtomicU64::new(0);

/// A salt for one retry loop: unique within the process, and varied across
/// runs by the process id.
fn next_salt() -> u64 {
    let instance = NEXT_SALT.fetch_add(1, Ordering::Relaxed);
    u64::from(std::process::id()).wrapping_add(instance)
}

/// The wait before the next attempt: `attempt` seconds, plus a small spread.
///
/// The spread is derived from the attempt number and the loop's [`next_salt`],
/// so loops started together do not retry in lockstep while a test can still
/// use [`crate::sleep::NoSleep`] and assert attempt counts instead of
/// durations.
fn backoff(attempt: u32, salt: u64) -> Duration {
    Duration::from_secs(u64::from(attempt)) + jitter(attempt, salt)
}

/// The spread added to an attempt's whole seconds.
///
/// The range is just under 251 ms, and the multiplier shares no factor with it,
/// so distinct loop salts land on distinct spreads by construction (until the
/// salt space wraps, far beyond any realistic number of concurrent loops).
fn jitter(attempt: u32, salt: u64) -> Duration {
    const RANGE: u64 = 250_000_000;
    const MULTIPLIER: u64 = 0x9E37_79B1;

    let mixed = salt
        .wrapping_mul(MULTIPLIER)
        .wrapping_add(u64::from(attempt).wrapping_mul(0x9E37_79B9_7F4A_7C15));
    Duration::from_nanos(mixed % RANGE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sleep::{NoSleep, SleepFuture};
    use std::sync::{Arc, Mutex};

    /// A sleep that records what it was asked to wait for and waits for none of
    /// it.
    #[derive(Debug, Clone, Default)]
    struct RecordingSleep(Arc<Mutex<Vec<Duration>>>);

    impl RecordingSleep {
        fn waits(&self) -> Vec<Duration> {
            self.0.lock().expect("recording sleep").clone()
        }
    }

    impl Sleep for RecordingSleep {
        fn sleep(&self, d: Duration) -> SleepFuture {
            self.0.lock().expect("recording sleep").push(d);
            Box::pin(async {})
        }
    }

    /// The wait for `attempt` keeps its whole seconds and stays under the
    /// spread bound; the exact spread belongs to the loop's salt.
    fn assert_backoff(d: Duration, attempt: u32) {
        let base = Duration::from_secs(u64::from(attempt));
        assert!(d >= base, "{d:?} is shorter than {base:?}");
        assert!(
            d < base + Duration::from_millis(251),
            "{d:?} exceeds the spread bound"
        );
    }

    #[tokio::test]
    async fn success_on_the_first_try_does_not_sleep() {
        let sleep = RecordingSleep::default();
        let mut calls = 0;
        let value = retry(5, &sleep, |_n| {
            calls += 1;
            async { Retry::Done(7) }
        })
        .await
        .expect("the first attempt succeeds");
        assert_eq!(value, 7);
        assert_eq!(calls, 1);
        assert!(sleep.waits().is_empty());
    }

    #[tokio::test]
    async fn success_after_retries_keeps_the_attempt_numbers() {
        let sleep = RecordingSleep::default();
        let mut seen = Vec::new();
        let value = retry(5, &sleep, |n| {
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
        let waits = sleep.waits();
        assert_eq!(waits.len(), 2);
        assert_backoff(waits[0], 1);
        assert_backoff(waits[1], 2);
    }

    #[tokio::test]
    async fn fatal_stops_after_one_attempt() {
        let sleep = RecordingSleep::default();
        let mut calls = 0;
        let err = retry(5, &sleep, |_n| {
            calls += 1;
            async { Retry::<()>::Fatal(anyhow::anyhow!("refused")) }
        })
        .await
        .expect_err("a fatal answer is an error");
        assert_eq!(calls, 1, "a fatal answer must not be attempted again");
        assert!(sleep.waits().is_empty());
        assert_eq!(err.to_string(), "refused");
    }

    #[tokio::test]
    async fn exhausting_the_attempts_returns_the_last_error() {
        let sleep = RecordingSleep::default();
        let mut calls = 0;
        let err = retry(5, &sleep, |n| {
            calls += 1;
            async move { Retry::<()>::Retry(anyhow::anyhow!("attempt {n}")) }
        })
        .await
        .expect_err("all attempts fail");
        assert_eq!(calls, 5);
        let waits = sleep.waits();
        assert_eq!(waits.len(), 4, "the final attempt must not pay for a wait");
        for (wait, attempt) in waits.iter().zip(1..) {
            assert_backoff(*wait, attempt);
        }
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
        let sleep = NoSleep;
        let value = retry_state(
            5,
            &sleep,
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

    #[test]
    fn backoff_is_linear_with_a_bounded_spread() {
        for n in 1..=5 {
            for salt in [0, 1, 7, u64::MAX] {
                assert_backoff(backoff(n, salt), n);
            }
        }
    }

    #[tokio::test]
    async fn concurrent_retries_at_the_same_attempt_get_distinct_delays() {
        const CALLERS: usize = 8;
        let sleep = RecordingSleep::default();
        let mut callers = Vec::with_capacity(CALLERS);
        for _ in 0..CALLERS {
            let sleep = sleep.clone();
            callers.push(tokio::spawn(async move {
                retry(2, &sleep, |n| async move {
                    if n == 1 {
                        Retry::Retry(anyhow::anyhow!("again"))
                    } else {
                        Retry::Done(())
                    }
                })
                .await
                .expect("the second attempt succeeds");
            }));
        }
        for caller in callers {
            caller.await.expect("the retry task runs");
        }

        let waits = sleep.waits();
        assert_eq!(waits.len(), CALLERS);
        let mut spreads: Vec<Duration> = waits
            .into_iter()
            .map(|wait| wait - Duration::from_secs(1))
            .collect();
        let count = spreads.len();
        spreads.sort_unstable();
        spreads.dedup();
        assert_eq!(
            spreads.len(),
            count,
            "concurrent retries at the same attempt shared a delay"
        );
    }
}
