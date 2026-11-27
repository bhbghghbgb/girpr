//! The one indexed bounded join window and the one panic-payload recovery rule.
//!
//! Two mechanics are shared here: a `buffer_unordered` window whose results are
//! restored to their input slots, and the downcast that pulls a panic's own
//! message out of a `Box<dyn Any + Send>` (a caught unwind) or a
//! [`tokio::task::JoinError`] (a lost spawned task). A task label and a fallback
//! string are supplied by the caller, because neither is a leaf decision: a
//! caller's published wording is its own contract.
//!
//! Nothing about failure *meaning* lives here. A caller gets
//! [`JoinOutcome::Failed`] for a failure its work reported and
//! [`JoinOutcome::Lost`] for work that never reported, then applies its own
//! policy — collect the failure, fail the run, stay conservative, or ignore it.
//! The blocking-pool entry point takes a thread-setup hook for the same reason:
//! the caller decides which dispatcher or thread-local guard a pool thread
//! carries, while this leaf owns the one `spawn_blocking` call.

use std::any::Any;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;

use futures_util::FutureExt;
use futures_util::stream::{Stream, StreamExt};
use tokio::task::JoinHandle;

/// The fallback used when a caught payload is neither `String` nor `&str`.
///
/// Passed explicitly by each caller: a caller that already publishes a
/// different string keeps its own.
pub const DEFAULT_FALLBACK: &str = "a non-string panic payload";

/// The two strings a lost task's message is spelled with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wording {
    task: String,
    fallback: String,
}

impl Wording {
    /// Bind a task label and a message fallback to one join call.
    pub fn new(task: &str, fallback: &str) -> Self {
        Self {
            task: task.to_string(),
            fallback: fallback.to_string(),
        }
    }
}

/// The payload's own message, when it carries one.
///
/// Neither `JoinError`'s `Display` nor a caught-unwind box includes the text the
/// failing task wrote; that text is the reason an operator needs. `&str` and
/// `String` are the two payloads `panic!` produces.
pub fn panic_text_opt(payload: &(dyn Any + Send)) -> Option<String> {
    payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
}

/// [`panic_text_opt`] with the caller's fallback for a payload with no message.
pub fn panic_text(payload: &(dyn Any + Send), fallback: &str) -> String {
    panic_text_opt(payload).unwrap_or_else(|| fallback.to_string())
}

/// A caught payload's message with the task label attached.
pub fn panic_message(payload: &(dyn Any + Send), wording: &Wording) -> String {
    format!(
        "the {} task panicked: {}",
        wording.task,
        panic_text(payload, &wording.fallback)
    )
}

/// The most useful text a [`tokio::task::JoinError`] carries.
///
/// **`JoinError`'s `Display` does not include the panic payload.** A
/// cancellation is not a panic and must not be reported as one.
pub fn join_failure(error: tokio::task::JoinError, wording: &Wording) -> String {
    if !error.is_panic() {
        return format!("the {} task was cancelled: {error}", wording.task);
    }
    let payload = error.into_panic();
    panic_message(payload.as_ref(), wording)
}

/// What became of one item in a join.
///
/// `Failed` is the work saying what went wrong; `Lost` is the window saying no
/// work reported. `Lost` carries the recovered payload in `why` when there was
/// one; `why: None` is the defensive case of a slot no future ever reported.
#[derive(Debug, PartialEq, Eq)]
pub enum JoinOutcome<E> {
    /// The task returned a failure of its own.
    Failed(E),
    /// The task did not return a result at all.
    Lost { why: Option<String> },
}

/// Run `setup` on a blocking pool thread, then `work`, dropping the guard that
/// `setup` returned only after `work` finishes.
///
/// The dispatcher (and any other thread-local guard) is captured by `setup` at
/// the spawn call site: a pool thread has none of its own. `spawn` is eager, so
/// a caller can start work and await its handle later. This helper does not
/// join, wrap or classify the result — that is the caller's policy.
pub fn spawn_blocking_with<S, G, F, R>(setup: S, work: F) -> JoinHandle<R>
where
    S: FnOnce() -> G + Send + 'static,
    G: 'static,
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    tokio::task::spawn_blocking(move || {
        let _guard = setup();
        work()
    })
}

/// Run `work` on the blocking pool, keeping the cause if it does not return.
pub async fn join_blocking<T, F>(wording: &Wording, work: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    match spawn_blocking_with(|| (), work).await {
        Ok(value) => Ok(value),
        Err(e) => Err(join_failure(e, wording)),
    }
}

/// The bounded window, yielding each `(index, outcome)` as it completes.
///
/// `f` is called once per index and returns that index with its own result. The
/// library bounds how many futures are polled at once (`jobs.max(1)`, so a zero
/// bound is one), and a panic in a buffered future is caught at its boundary
/// and turned into [`JoinOutcome::Lost`] with the payload recovered. A caller
/// that folds as completions arrive keeps its own failure policy and its own
/// cancellation behaviour: dropping this stream drops the buffered futures.
fn window<I, R, E, F, Fut>(
    indices: I,
    jobs: usize,
    wording: Wording,
    mut f: F,
) -> impl Stream<Item = (usize, Result<R, JoinOutcome<E>>)>
where
    I: IntoIterator<Item = usize>,
    F: FnMut(usize) -> Fut,
    Fut: Future<Output = (usize, Result<R, E>)> + Send + 'static,
    R: Send + 'static,
    E: Send + 'static,
{
    futures_util::stream::iter(indices)
        .map(move |i| {
            let work = f(i);
            let wording = wording.clone();
            async move {
                match AssertUnwindSafe(work).catch_unwind().await {
                    Ok((i, result)) => (i, result.map_err(JoinOutcome::Failed)),
                    Err(payload) => {
                        let why = panic_message(payload.as_ref(), &wording);
                        let task = &wording.task;
                        tracing::error!(item = i, error = %why, "a {task} task did not report");
                        (i, Err(JoinOutcome::Lost { why: Some(why) }))
                    }
                }
            }
        })
        .buffer_unordered(jobs.max(1))
}

/// [`window`] over `0..n`.
pub fn join_stream<R, E, F, Fut>(
    n: usize,
    jobs: usize,
    wording: Wording,
    f: F,
) -> impl Stream<Item = (usize, Result<R, JoinOutcome<E>>)>
where
    F: FnMut(usize) -> Fut,
    Fut: Future<Output = (usize, Result<R, E>)> + Send + 'static,
    R: Send + 'static,
    E: Send + 'static,
{
    window(0..n, jobs, wording, f)
}

/// [`window`] over a caller-supplied index iterator.
///
/// An iterator that stops early (for example on fail-fast) lets a caller keep
/// starting no further work once it has seen a failure.
pub fn join_iter<I, R, E, F, Fut>(
    indices: I,
    jobs: usize,
    wording: Wording,
    f: F,
) -> impl Stream<Item = (usize, Result<R, JoinOutcome<E>>)>
where
    I: IntoIterator<Item = usize>,
    F: FnMut(usize) -> Fut,
    Fut: Future<Output = (usize, Result<R, E>)> + Send + 'static,
    R: Send + 'static,
    E: Send + 'static,
{
    window(indices, jobs, wording, f)
}

/// Collect a window into a vector in **input order**.
///
/// Completion order is the library's; each result is written into the slot of
/// the index its future returned. A slot no future reported becomes a
/// [`JoinOutcome::Lost`] with `why: None`, so the caller can still produce a row
/// for that item instead of dropping it.
pub async fn join_indices<R, E, F, Fut>(
    n: usize,
    jobs: usize,
    wording: Wording,
    f: F,
) -> Vec<Result<R, JoinOutcome<E>>>
where
    F: FnMut(usize) -> Fut,
    Fut: Future<Output = (usize, Result<R, E>)> + Send + 'static,
    R: Send + 'static,
    E: Send + 'static,
{
    let mut out: Vec<Option<Result<R, JoinOutcome<E>>>> = (0..n).map(|_| None).collect();
    let mut running = window(0..n, jobs, wording, f);
    while let Some((i, outcome)) = running.next().await {
        out[i] = Some(outcome);
    }
    out.into_iter()
        .map(|r| r.unwrap_or(Err(JoinOutcome::Lost { why: None })))
        .collect()
}

/// [`join_indices`] with a shared slice handed to each future.
///
/// A task receives an `Arc<[T]>` and indexes it by its slot, so a caller builds
/// one shared collection instead of cloning an item per future.
pub async fn join_ordered<T, R, E, F, Fut>(
    items: &Arc<[T]>,
    jobs: usize,
    wording: Wording,
    mut f: F,
) -> Vec<Result<R, JoinOutcome<E>>>
where
    T: Send + Sync + 'static,
    F: FnMut(usize, Arc<[T]>) -> Fut,
    Fut: Future<Output = (usize, Result<R, E>)> + Send + 'static,
    R: Send + 'static,
    E: Send + 'static,
{
    let n = items.len();
    join_indices(n, jobs, wording, |i| f(i, Arc::clone(items))).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn wording() -> Wording {
        Wording::new("probe", DEFAULT_FALLBACK)
    }

    #[tokio::test]
    async fn a_panicking_task_reports_its_payload() {
        let handle = tokio::spawn(async { panic!("metadata job exploded") });
        let e = handle.await.expect_err("the task panicked");
        let why = join_failure(e, &wording());
        assert!(why.contains("metadata job exploded"), "{why}");
        assert!(why.contains("the probe task panicked"), "{why}");
    }

    #[tokio::test]
    async fn a_cancelled_task_is_not_reported_as_a_panic() {
        let handle = tokio::spawn(std::future::pending::<()>());
        handle.abort();
        let e = handle.await.expect_err("the task was cancelled");
        let why = join_failure(e, &wording());
        assert!(why.contains("cancelled"), "{why}");
        assert!(!why.contains("panicked"), "{why}");
    }

    #[test]
    fn a_non_string_payload_uses_the_callers_fallback() {
        let payload = std::panic::catch_unwind(|| std::panic::panic_any(42u32))
            .expect_err("the payload is not a string");
        assert_eq!(
            panic_text(payload.as_ref(), "custom fallback"),
            "custom fallback"
        );
        assert_eq!(
            panic_message(payload.as_ref(), &Wording::new("probe", "custom fallback")),
            "the probe task panicked: custom fallback"
        );
    }

    #[tokio::test]
    async fn results_keep_input_order_and_the_window_bound() {
        let items: Arc<[usize]> = (0..8).collect::<Vec<_>>().into();
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let got = join_ordered(&items, 3, wording(), |i, items| {
            let active = Arc::clone(&active);
            let peak = Arc::clone(&peak);
            async move {
                let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(std::time::Duration::from_millis((8 - items[i]) as u64)).await;
                active.fetch_sub(1, Ordering::SeqCst);
                (i, Ok::<usize, String>(items[i] * 2))
            }
        })
        .await;
        let values: Vec<usize> = got
            .into_iter()
            .map(|r| match r {
                Ok(v) => v,
                Err(_) => panic!("no probe task was lost"),
            })
            .collect();
        assert_eq!(values, (0..8).map(|i| i * 2).collect::<Vec<_>>());
        assert_eq!(peak.load(Ordering::SeqCst), 3, "jobs bounds the window");
    }

    #[tokio::test]
    async fn a_zero_bound_still_runs_one_task_at_a_time() {
        let got = join_indices(4, 0, wording(), |i| async move {
            (i, Ok::<usize, String>(i + 1))
        })
        .await;
        let values: Vec<usize> = got.into_iter().map(|r| r.ok().unwrap()).collect();
        assert_eq!(values, vec![1, 2, 3, 4]);
    }

    #[tokio::test]
    async fn a_panicking_item_keeps_its_slot_and_cause() {
        let items: Arc<[usize]> = [0usize, 1, 2].into();
        let got = join_ordered(&items, 2, wording(), |i, items| async move {
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
    async fn dropping_the_stream_cancels_in_flight_work() {
        use futures::StreamExt;
        use std::sync::Arc;

        struct CancelGuard(Arc<AtomicUsize>);
        impl Drop for CancelGuard {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }

        let cancelled = Arc::new(AtomicUsize::new(0));
        let (started_tx, mut started_rx) = futures::channel::mpsc::channel(3);
        let futures_started_tx = started_tx.clone();
        let futures_cancelled = Arc::clone(&cancelled);
        let stream = join_stream(8, 3, wording(), move |i| {
            let guard = CancelGuard(Arc::clone(&futures_cancelled));
            let mut started = futures_started_tx.clone();
            async move {
                let _guard = guard;
                let _ = started.try_send(());
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                (i, Ok::<usize, String>(0))
            }
        });
        drop(started_tx);

        {
            let mut stream = std::pin::pin!(stream);
            tokio::select! {
                _ = stream.next() => panic!("the stream should still be running"),
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
    async fn a_blocking_join_keeps_the_payload_and_the_value() {
        let why = join_blocking(&wording(), || panic!("blocking work exploded"))
            .await
            .expect_err("the blocking task panicked");
        assert!(why.contains("blocking work exploded"), "{why}");
        assert!(why.contains("the probe task panicked"), "{why}");
        assert_eq!(join_blocking(&wording(), || 7).await, Ok(7));
    }

    #[tokio::test]
    async fn the_spawn_setup_guard_outlives_the_work() {
        use std::sync::atomic::AtomicBool;
        let dropped = Arc::new(AtomicBool::new(false));
        let work_dropped = Arc::clone(&dropped);
        let setup_dropped = Arc::clone(&dropped);
        struct Guard(Arc<AtomicBool>);
        impl Drop for Guard {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let handle = spawn_blocking_with(
            move || Guard(setup_dropped),
            move || {
                assert!(
                    !work_dropped.load(Ordering::SeqCst),
                    "guard alive during work"
                );
                7
            },
        );
        assert_eq!(handle.await.expect("the blocking work finished"), 7);
        assert!(
            dropped.load(Ordering::SeqCst),
            "the guard was dropped afterwards"
        );
    }
}
