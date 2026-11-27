//! The ordered-join seam gircap's callers already resolve through.
//!
//! The mechanics — the indexed `buffer_unordered(jobs.max(1))` window, result
//! restoration by input slot, and the downcast that recovers a panic's own
//! message — live in the shared logic-free `girpr-async-join` leaf. This module
//! keeps the `crate::tasks::*` paths used by the fetcher, the blob store and the
//! request-log write unchanged, together with gircap's published wording: the
//! fallback for a non-string payload and the two "task panicked / was
//! cancelled" sentences. `JoinOutcome` -> `ApiError` mapping stays at each
//! caller, because that is program policy.

use std::sync::Arc;

use girpr_async_join::{DEFAULT_FALLBACK, Wording};

pub use girpr_async_join::JoinOutcome;

fn wording(task: &str) -> Wording {
    Wording::new(task, DEFAULT_FALLBACK)
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
    girpr_async_join::join_failure(e, &wording(task))
}

/// Run `f` over `items`, at most `jobs` at a time, **in input order**.
///
/// `f` receives the item's index and a clone of the shared slice, and returns
/// `(index, result)`. A panic is caught at the future boundary and keeps both
/// its slot and its cause instead of collapsing into one generic string.
/// Dropping this future cancels every in-flight item at its current await
/// point; a slot no task ever reported becomes [`JoinOutcome::Lost`] with
/// `why: None`, so the caller can still produce a row for the item.
pub async fn join_ordered<T, R, E, F, Fut>(
    items: &Arc<[T]>,
    jobs: usize,
    task: &str,
    f: F,
) -> Vec<Result<R, JoinOutcome<E>>>
where
    T: Send + Sync + 'static,
    R: Send + 'static,
    E: Send + 'static,
    F: FnMut(usize, Arc<[T]>) -> Fut,
    Fut: std::future::Future<Output = (usize, Result<R, E>)> + Send + 'static,
{
    girpr_async_join::join_ordered(items, jobs, wording(task), f).await
}

/// Run `work` on the blocking pool, keeping the cause if it does not return.
///
/// `work` is a parameter rather than a concrete closure so a test can drive the
/// panic arm: no input makes the real blob measurement or request-log write
/// panic on demand.
pub async fn join_blocking<T, F>(task: &str, work: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    girpr_async_join::join_blocking(&wording(task), work).await
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
