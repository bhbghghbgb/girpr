//! The one ordered-join loop and the one rule for a lost task.
//!
//! Two callers run a bounded ordered join over a plan and both need each result
//! in its input slot: gircap's blob-store fill pass and this crate's
//! [`crate::fetcher::Fetcher::map_concurrently`]. [`join_ordered`] is that loop,
//! written once as a `futures` buffered stream with an ordered collection. A
//! task can be lost to a panic or to cancellation, and throwing the cause away
//! — silently, or behind a generic "panicked" row — loses what an operator
//! needs, which is in the panic payload, not in `Display`. The loop applies the
//! same downcast rule as [`join_failure`] so the caller's outcome still names
//! the cause and the slot it belonged to. The two direct `spawn_blocking` joins
//! — gircap's blob measurement and the request-log write — take that rule
//! through [`join_blocking`].

/// The message a panic payload carries, with the task name attached.
///
/// Neither a `JoinError`'s `Display` nor the `Box<dyn Any + Send>` a caught
/// unwind yields includes the message the failing task wrote. That reason —
/// what an operator actually needs — has to be downcast out of the payload.
/// Sharing this rule keeps the `JoinError` and caught-unwind paths spelling the
/// same text.
fn panic_message(payload: &(dyn std::any::Any + Send), task: &str) -> String {
    let message = payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "a non-string panic payload".to_string());
    format!("the {task} task panicked: {message}")
}

/// The most useful text a [`tokio::task::JoinError`] carries.
///
/// **`JoinError`'s `Display` does not include the panic payload.** It says only
/// that the task panicked, so a message the failing task wrote — the reason an
/// operator actually needs — has to be downcast out of [`into_panic`]. A
/// cancellation is not a panic and must not be reported as one.
///
/// [`into_panic`]: tokio::task::JoinError::into_panic
pub fn join_failure(e: tokio::task::JoinError, task: &str) -> String {
    if !e.is_panic() {
        return format!("the {task} task was cancelled: {e}");
    }
    let payload = e.into_panic();
    panic_message(payload.as_ref(), task)
}

/// What became of one item in [`join_ordered`].
///
/// The two arms are separate because they need different handling: `Failed` is
/// the work saying what went wrong, with its slot known, while `Lost` is the
/// loop saying the work never reported. `Lost` keeps the recovered panic
/// payload in `why`; `why: None` is the defensive fallback for a slot no work
/// ever reported.
pub enum JoinOutcome<E> {
    /// The task returned a failure of its own.
    Failed(E),
    /// The task did not return a result at all.
    Lost {
        /// The message [`join_failure`] would produce, when one was recovered.
        why: Option<String>,
    },
}

/// Run `f` over `items`, at most `jobs` at a time, **in input order**.
///
/// `f` receives the item's index and a clone of the shared slice, and returns
/// `(index, result)`. The index comes back through every path — success, the
/// work's own failure, and a panic — because `buffer_unordered` yields results
/// in completion order and a failed item must still be told apart from one that
/// died. Each result is written into its own slot, which is what restores input
/// order. A panic is caught at the future boundary with the same payload
/// downcast [`join_failure`] applies to a `JoinError`, so it keeps both its slot
/// and its cause instead of collapsing into one generic string.
///
/// **The slice is shared rather than copied per task.** A task receives an
/// `Arc<[T]>` and indexes it by the slot it was given, so an item that owns
/// several allocations — a whole `Want` per chunk — is cloned once into the
/// `Arc` by the caller instead of once per future.
///
/// Concurrency is bounded by `jobs.max(1)` (a zero bound is treated as one),
/// matching the old loop. Dropping this future drops the buffered stream and
/// cancels every in-flight item at its current await point. A slot no task ever
/// reported becomes [`JoinOutcome::Lost`] with `why: None`, so the caller can
/// still produce a row for the item rather than dropping it. `task` names the
/// work in the recovered message and in the log.
pub async fn join_ordered<T, R, E, F, Fut>(
    items: &std::sync::Arc<[T]>,
    jobs: usize,
    task: &str,
    f: F,
) -> Vec<Result<R, JoinOutcome<E>>>
where
    T: Send + Sync + 'static,
    R: Send + 'static,
    E: Send + 'static,
    F: Fn(usize, std::sync::Arc<[T]>) -> Fut,
    Fut: std::future::Future<Output = (usize, Result<R, E>)> + Send + 'static,
{
    use futures::{FutureExt, StreamExt};

    let n = items.len();
    let mut out: Vec<Option<Result<R, JoinOutcome<E>>>> = (0..n).map(|_| None).collect();

    // One future per input slot, polled at most `jobs.max(1)` at a time. The
    // library owns the concurrency bound and the completion signal; collecting
    // by index is the local policy that turns completion order back into input
    // order.
    let mut running = futures::stream::iter(0..n)
        .map(move |i| {
            let work = f(i, std::sync::Arc::clone(items));
            async move {
                // A panic in a buffered future would otherwise unwind through
                // this whole loop into the caller and take the run with it.
                // Catch it at the future boundary and recover the payload with
                // the same downcast `join_failure` uses for a spawned task.
                match std::panic::AssertUnwindSafe(work).catch_unwind().await {
                    Ok((i, r)) => (i, r.map_err(JoinOutcome::Failed)),
                    Err(payload) => {
                        let why = panic_message(payload.as_ref(), task);
                        tracing::error!(item = i, error = %why, "a {task} task did not report");
                        (i, Err(JoinOutcome::Lost { why: Some(why) }))
                    }
                }
            }
        })
        .buffer_unordered(jobs.max(1));

    while let Some((i, outcome)) = running.next().await {
        out[i] = Some(outcome);
    }

    out.into_iter()
        .map(|r| r.unwrap_or(Err(JoinOutcome::Lost { why: None })))
        .collect()
}

/// Run `work` on the blocking pool, keeping the cause if it does not return.
///
/// [`join_failure`] applied at the one place that spawns blocking work, so the
/// next `spawn_blocking` site gets the payload-preserving rule rather than
/// rediscovering `format!("{e}")`. `work` is a parameter rather than a concrete
/// closure so a test can drive the panic arm: no input makes the real blob
/// measurement or request-log write panic on demand.
pub async fn join_blocking<T, F>(task: &str, work: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    match tokio::task::spawn_blocking(work).await {
        Ok(value) => Ok(value),
        Err(e) => Err(join_failure(e, task)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_panicking_task_reports_its_payload() {
        let handle = tokio::spawn(async { panic!("metadata job exploded") });
        let e = handle.await.expect_err("the task panicked");
        let why = join_failure(e, "request");
        assert!(
            why.contains("metadata job exploded"),
            "the payload is the part worth logging: {why}"
        );
        assert!(why.contains("panicked"), "{why}");
    }

    #[tokio::test]
    async fn a_cancelled_task_is_not_reported_as_a_panic() {
        let handle = tokio::spawn(std::future::pending::<()>());
        handle.abort();
        let e = handle.await.expect_err("the task was cancelled");
        let why = join_failure(e, "request");
        assert!(why.contains("cancelled"), "{why}");
        assert!(
            !why.contains("panicked"),
            "a cancelled task is not a panic: {why}"
        );
    }

    #[tokio::test]
    async fn results_keep_input_order_when_jobs_is_more_than_one() {
        let items: std::sync::Arc<[usize]> = (0..8).collect::<Vec<_>>().into();
        let got = join_ordered(&items, 3, "probe", |i, items| async move {
            // Later items finish first, so completion order is the reverse of
            // input order unless each result stays in its own slot.
            tokio::time::sleep(std::time::Duration::from_millis((8 - items[i]) as u64)).await;
            (i, Ok::<usize, String>(items[i] * 2))
        })
        .await;
        let values: Vec<usize> = got
            .into_iter()
            .map(|r| r.unwrap_or_else(|_| panic!("no probe task was lost")))
            .collect();
        assert_eq!(values, (0..8).map(|i| i * 2).collect::<Vec<_>>());
    }

    #[tokio::test]
    async fn never_runs_more_than_jobs_tasks_at_once() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let items: std::sync::Arc<[usize]> = (0..12).collect::<Vec<_>>().into();
        let active = std::sync::Arc::new(AtomicUsize::new(0));
        let peak = std::sync::Arc::new(AtomicUsize::new(0));
        let jobs = 3;
        let got = join_ordered(&items, jobs, "probe", |i, items| {
            let active = std::sync::Arc::clone(&active);
            let peak = std::sync::Arc::clone(&peak);
            async move {
                let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                // A real yield keeps several futures in flight at once, so the
                // ceiling is observable rather than trivially one.
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                active.fetch_sub(1, Ordering::SeqCst);
                (i, Ok::<usize, String>(items[i]))
            }
        })
        .await;

        let values: Vec<usize> = got
            .into_iter()
            .map(|r| r.unwrap_or_else(|_| panic!("no probe task was lost")))
            .collect();
        assert_eq!(
            values,
            (0..12).collect::<Vec<_>>(),
            "order survives the bound"
        );
        assert_eq!(
            peak.load(Ordering::SeqCst),
            jobs,
            "concurrency is capped at jobs, and the bound is actually reached"
        );
    }

    #[tokio::test]
    async fn dropping_the_join_cancels_in_flight_work() {
        use futures::StreamExt;
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct CancelGuard(std::sync::Arc<AtomicUsize>);
        impl Drop for CancelGuard {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }

        let items: std::sync::Arc<[usize]> = (0..8).collect::<Vec<_>>().into();
        let cancelled = std::sync::Arc::new(AtomicUsize::new(0));
        let (started_tx, mut started_rx) = futures::channel::mpsc::channel(3);
        let futures_started_tx = started_tx.clone();
        let futures_cancelled = std::sync::Arc::clone(&cancelled);
        let join = join_ordered(&items, 3, "probe", move |i, _items| {
            let guard = CancelGuard(std::sync::Arc::clone(&futures_cancelled));
            let mut started = futures_started_tx.clone();
            async move {
                let _guard = guard;
                let _ = started.try_send(());
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                (i, Ok::<usize, String>(0))
            }
        });
        drop(started_tx);

        // Let the runtime drive the join until all three in-flight futures have
        // signalled that they started and are suspended on their long sleep.
        // Dropping the block drops the join future itself (not just a pin of it),
        // which must cancel every buffered future at its await point.
        {
            let mut join = std::pin::pin!(join);
            tokio::select! {
                _ = &mut join => panic!("the join should still be running"),
                _ = async {
                    for _ in 0..3 {
                        started_rx.next().await.expect("a future started");
                    }
                } => {}
            }
        }

        assert_eq!(
            cancelled.load(Ordering::SeqCst),
            3,
            "each in-flight future is cancelled at its await point"
        );
    }

    #[tokio::test]
    async fn a_panicking_item_keeps_its_slot_and_cause() {
        let items: std::sync::Arc<[usize]> = [0usize, 1, 2].into();
        let got = join_ordered(&items, 2, "probe", |i, items| async move {
            if i == 1 {
                panic!("probe exploded");
            }
            (i, Ok::<usize, String>(items[i] * 10))
        })
        .await;

        assert_eq!(got[0].as_ref().ok(), Some(&0));
        match &got[1] {
            Err(JoinOutcome::Lost { why: Some(why) }) => {
                assert!(why.contains("probe exploded"), "{why}");
                assert!(why.contains("the probe task panicked"), "{why}");
            }
            _ => panic!("the middle item did not report with its cause"),
        }
        assert_eq!(got[2].as_ref().ok(), Some(&20), "the rest still runs");
    }

    #[tokio::test]
    async fn a_panicking_blocking_task_reports_its_payload() {
        let why = join_blocking("measure", || panic!("blocking work exploded"))
            .await
            .expect_err("the blocking task panicked");
        assert!(why.contains("blocking work exploded"), "{why}");
        assert!(why.contains("the measure task panicked"), "{why}");
    }

    #[tokio::test]
    async fn a_returning_blocking_task_keeps_its_value() {
        assert_eq!(join_blocking("measure", || 7).await, Ok(7));
    }
}
