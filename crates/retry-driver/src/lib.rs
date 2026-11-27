//! The one async retry loop shared by girpr's fetch path and gircap's fetcher.
//!
//! `girpr-retry` owns the two mechanical parts of a retry loop: the delay
//! sequence and the injected sleep. This leaf owns the part directly above them
//! — the attempt loop itself: how many attempts, the first/last-attempt wait
//! rule, the injected sleep, an immediate stop on a fatal answer, and the
//! retention of the last error.
//!
//! Everything that makes a loop *someone's own* stays with the caller: which
//! answers are transient, which are final, what work is carried between
//! attempts, when and what to log, and how a retained error is worded to the
//! program's own error type. The driver is generic over the success value, the
//! carried state and the error, so it adds no policy of its own.

use std::future::Future;
use std::time::Duration;

use girpr_retry::{DelaySequence, Sleep};

/// One attempt's answer to the driver.
///
/// `S` is the work carried into the next attempt (use `()` when there is none),
/// `E` is the caller's error type, and `T` is the success value. The caller
/// decides which of its own outcomes map to which variant; the driver only
/// reads the variant.
pub enum Step<T, S, E> {
    /// The attempt succeeded; the loop stops here.
    Done(T),
    /// The answer is final; the loop stops without another attempt or wait.
    Fatal(E),
    /// The answer is transient; `S` is carried forward and, when attempts
    /// remain, the loop waits before the next attempt.
    Retry(S, E),
}

/// Why the loop ended without a success.
///
/// The distinction between [`Failure::Fatal`] and [`Failure::Exhausted`] is
/// deliberately kept for the caller: a fatal answer and a spent retry budget
/// are different facts, and each program words them in its own error type.
#[derive(Debug, PartialEq, Eq)]
pub enum Failure<E> {
    /// A fatal answer stopped the loop. Carries that answer's error unchanged.
    Fatal(E),
    /// The attempt budget ran out. Carries the **last** retry error, not an
    /// earlier one.
    Exhausted(E),
    /// Zero attempts were requested, so no attempt could be made.
    NoAttempts,
}

/// The loop's configuration: an attempt budget, a fresh delay sequence, and an
/// injected sleeper.
///
/// Built per logical operation. A `DelaySequence` is stateful, so each call
/// gets its own instance; the caller chooses the schedule (and therefore the
/// jitter) that built it.
pub struct Driver<'a> {
    attempts: u32,
    delays: &'a mut DelaySequence,
    sleeper: &'a dyn Sleep,
}

impl<'a> Driver<'a> {
    /// Bind one attempt budget to one delay sequence and one sleeper.
    pub fn new(attempts: u32, delays: &'a mut DelaySequence, sleeper: &'a dyn Sleep) -> Self {
        Self {
            attempts,
            delays,
            sleeper,
        }
    }

    /// Run `attempt` up to `attempts` times, starting from `state`.
    ///
    /// - [`Step::Done`] and [`Step::Fatal`] return immediately; a fatal answer
    ///   costs neither another attempt nor a wait.
    /// - [`Step::Retry`] carries its state forward. When attempts remain, the
    ///   next delay is taken from the sequence, reported through `on_wait`, and
    ///   awaited through the injected sleeper. On the last attempt there is no
    ///   wait: the last error is returned as [`Failure::Exhausted`].
    ///
    /// The attempt function takes the carried state and the one-based attempt
    /// number. `on_wait` receives the number of the attempt that follows the
    /// wait and the delay; it exists so callers can keep their own wait logging
    /// at their own site instead of in this policy-free leaf.
    #[allow(clippy::expect_used)] // the slot is filled on entry and refilled on a retry
    pub async fn drive<T, S, E, F, Fut, W>(
        self,
        state: S,
        mut on_wait: W,
        mut attempt: F,
    ) -> Result<T, Failure<E>>
    where
        F: FnMut(S, u32) -> Fut,
        Fut: Future<Output = Step<T, S, E>>,
        W: FnMut(u32, Duration),
    {
        if self.attempts == 0 {
            return Err(Failure::NoAttempts);
        }
        let mut slot = Some(state);
        for attempt_no in 1..=self.attempts {
            let carried = slot
                .take()
                .expect("retry state present for the next attempt");
            match attempt(carried, attempt_no).await {
                Step::Done(value) => return Ok(value),
                Step::Fatal(error) => return Err(Failure::Fatal(error)),
                Step::Retry(next, error) => {
                    slot = Some(next);
                    if attempt_no >= self.attempts {
                        return Err(Failure::Exhausted(error));
                    }
                    if let Some(wait) = self.delays.next_delay() {
                        on_wait(attempt_no + 1, wait);
                        self.sleeper.sleep(wait).await;
                    }
                }
            }
        }
        // Unreachable: the zero case returned above and every other path
        // returns from inside the loop. Kept as an error rather than a panic so
        // no caller of a policy-free leaf can trip a production panic here.
        Err(Failure::NoAttempts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use girpr_retry::{RecordingSleep, Schedule};

    /// Drive a zero-wait schedule with a caller-supplied answer sequence, and
    /// report the attempt numbers and waits the driver actually produced.
    async fn run(
        attempts: u32,
        answers: Vec<Step<u32, u32, &'static str>>,
    ) -> (
        Result<u32, Failure<&'static str>>,
        Vec<u32>,
        Vec<(u32, Duration)>,
    ) {
        let sleeper = RecordingSleep::default();
        let mut delays = Schedule::zero().build();
        let mut seen = Vec::new();
        let mut waits = Vec::new();
        let mut answers = answers.into_iter();
        let seen_ref = &mut seen;
        let waits_ref = &mut waits;
        let result = Driver::new(attempts, &mut delays, &sleeper)
            .drive(
                0_u32,
                |next, wait| waits_ref.push((next, wait)),
                move |state, n| {
                    seen_ref.push(n);
                    let answer = answers.next().expect("an answer per attempt");
                    let _ = state;
                    async move { answer }
                },
            )
            .await;
        (result, seen, waits)
    }

    /// All loop mechanics in one test so nextest spawns one process. Each
    /// labelled subcase names the behaviour that failed.
    #[tokio::test]
    async fn driver_contracts() {
        for label in [
            "success_on_the_first_try_stops_immediately",
            "success_after_retries_keeps_the_attempt_numbers",
            "fatal_stops_after_one_attempt",
            "exhaustion_returns_the_last_error_without_a_final_wait",
            "state_is_carried_from_one_attempt_to_the_next",
            "zero_attempts_make_no_attempt",
        ] {
            println!("subcase: {label}");
            match label {
                "success_on_the_first_try_stops_immediately" => {
                    let (result, seen, waits) = run(
                        5,
                        vec![Step::Done(7), Step::Retry(0, "unused"), Step::Done(9)],
                    )
                    .await;
                    assert_eq!(result, Ok(7), "{label}: the success");
                    assert_eq!(seen, vec![1], "{label}: the attempt numbers");
                    assert!(waits.is_empty(), "{label}: no wait on success");
                }
                "success_after_retries_keeps_the_attempt_numbers" => {
                    let (result, seen, waits) = run(
                        5,
                        vec![Step::Retry(0, "one"), Step::Retry(0, "two"), Step::Done(3)],
                    )
                    .await;
                    assert_eq!(result, Ok(3), "{label}: the success");
                    assert_eq!(seen, vec![1, 2, 3], "{label}: the attempt numbers");
                    assert_eq!(waits.len(), 2, "{label}: one wait between attempts");
                }
                "fatal_stops_after_one_attempt" => {
                    let (result, seen, waits) =
                        run(5, vec![Step::Fatal("refused"), Step::Done(9)]).await;
                    assert_eq!(result, Err(Failure::Fatal("refused")), "{label}: the error");
                    assert_eq!(seen, vec![1], "{label}: no second attempt");
                    assert!(waits.is_empty(), "{label}: no wait after a fatal answer");
                }
                "exhaustion_returns_the_last_error_without_a_final_wait" => {
                    let (result, seen, waits) = run(
                        3,
                        vec![
                            Step::Retry(0, "one"),
                            Step::Retry(0, "two"),
                            Step::Retry(0, "three"),
                        ],
                    )
                    .await;
                    assert_eq!(
                        result,
                        Err(Failure::Exhausted("three")),
                        "{label}: the last error"
                    );
                    assert_eq!(seen, vec![1, 2, 3], "{label}: every attempt");
                    assert_eq!(waits.len(), 2, "{label}: no wait after the last attempt");
                }
                "state_is_carried_from_one_attempt_to_the_next" => {
                    let sleeper = RecordingSleep::default();
                    let mut delays = Schedule::zero().build();
                    let value = Driver::new(5, &mut delays, &sleeper)
                        .drive(
                            0_u32,
                            |_, _| (),
                            |state, n| {
                                let answer = if n < 3 {
                                    Step::Retry(state + n, "again")
                                } else {
                                    Step::Done(state + n)
                                };
                                async move { answer }
                            },
                        )
                        .await
                        .expect("the third attempt succeeds");
                    assert_eq!(value, 1 + 2 + 3, "{label}: the carried state");
                }
                "zero_attempts_make_no_attempt" => {
                    let (result, seen, waits) = run(0, vec![Step::Done(7)]).await;
                    assert_eq!(result, Err(Failure::NoAttempts), "{label}: the error");
                    assert!(seen.is_empty(), "{label}: no attempt");
                    assert!(waits.is_empty(), "{label}: no wait");
                }
                _ => unreachable!("unhandled subcase: {label}"),
            }
        }
    }

    /// The wait rule pins the schedule order and the reported attempt numbers:
    /// the first attempt never waits, each wait is reported for the attempt it
    /// precedes, and the sleeper sees exactly the delays the schedule produced.
    #[tokio::test]
    async fn waits_are_between_attempts_and_report_the_next_attempt_number() {
        let sleeper = RecordingSleep::default();
        let mut delays = Schedule::exponential(
            Duration::from_millis(500),
            2.0,
            0.0,
            Duration::from_secs(10),
        )
        .build();
        let mut answers = [
            Step::Retry((), "again"),
            Step::Retry((), "again"),
            Step::Done(3),
        ]
        .into_iter();
        let mut waits = Vec::new();
        let value = Driver::new(3, &mut delays, &sleeper)
            .drive(
                (),
                |next, wait| waits.push((next, wait)),
                move |(), _| {
                    let answer = answers.next().expect("an answer per attempt");
                    async move { answer }
                },
            )
            .await
            .expect("the third attempt succeeds");
        assert_eq!(value, 3);
        assert_eq!(
            waits,
            vec![
                (2, Duration::from_millis(500)),
                (3, Duration::from_millis(1000)),
            ],
            "the wait is reported for the following attempt, in schedule order"
        );
        assert_eq!(
            sleeper.delays(),
            vec![Duration::from_millis(500), Duration::from_millis(1000)],
            "the sleeper sees the same delays, and none before the first attempt"
        );
    }
}
