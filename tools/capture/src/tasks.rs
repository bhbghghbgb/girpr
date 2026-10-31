//! The one ordered-join loop and the one rule for a [`tokio::task::JoinError`].
//!
//! Two callers run a `JoinSet` over a plan and both need each result in its
//! input slot: [`crate::blobs::Store::fill`] and
//! [`crate::http::Fetcher::map_concurrently`]. [`join_ordered`] is that loop,
//! written once. A task can be lost to a panic or to cancellation, and throwing
//! the `JoinError` away — silently, or behind a generic "panicked" row — loses
//! the cause an operator needs, which is in the payload, not in `Display`. The
//! loop applies [`join_failure`] so the caller's outcome still names the cause
//! and the slot it belonged to. The two direct `spawn_blocking` joins —
//! [`crate::blobs::Store`]'s measurement and the request-log write — take the
//! same rule through [`join_blocking`].

/// The most useful text a [`tokio::task::JoinError`] carries.
///
/// **`JoinError`'s `Display` does not include the panic payload.** It says only
/// that the task panicked, so a message the failing task wrote — the reason an
/// operator actually needs — has to be downcast out of [`into_panic`]. A
/// cancellation is not a panic and must not be reported as one.
///
/// [`into_panic`]: tokio::task::JoinError::into_panic
pub(crate) fn join_failure(e: tokio::task::JoinError, task: &str) -> String {
    if !e.is_panic() {
        return format!("the {task} task was cancelled: {e}");
    }
    let payload = e.into_panic();
    let message = payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "a non-string panic payload".to_string());
    format!("the {task} task panicked: {message}")
}

/// What became of one item in [`join_ordered`].
///
/// The two arms are separate because they need different handling: `Failed` is
/// the work saying what went wrong, with its slot known, while `Lost` is the
/// loop saying the task never reported. `Lost` keeps the payload when the task
/// id could be mapped back to a slot; `why: None` means only the item's
/// position identifies it.
pub(crate) enum JoinOutcome<E> {
    /// The task returned a failure of its own.
    Failed(E),
    /// The task did not return a result at all.
    Lost {
        /// [`join_failure`]'s text, when the task id was still in `slots`.
        why: Option<String>,
    },
}

/// Run `f` over `items`, at most `jobs` at a time, **in input order**.
///
/// `f` receives the item's index and a clone of the item, and returns
/// `(index, result)`. The index has to come back even when the work failed, or
/// a failed task is indistinguishable from one that died. A `JoinSet` does not
/// carry it through a panic — it names the failing task by id — so the loop
/// records each spawn's id and maps it back. That is what lets a panic keep
/// both its slot and its payload instead of collapsing into one generic string.
///
/// A slot no task ever reported becomes [`JoinOutcome::Lost`] with `why: None`,
/// so the caller can still produce a row for the item rather than dropping it.
/// `task` names the work in [`join_failure`]'s text and in the log.
pub(crate) async fn join_ordered<T, R, E, F, Fut>(
    items: &[T],
    jobs: usize,
    task: &str,
    f: F,
) -> Vec<Result<R, JoinOutcome<E>>>
where
    T: Clone + Send + 'static,
    R: Send + 'static,
    E: Send + 'static,
    F: Fn(usize, T) -> Fut,
    Fut: std::future::Future<Output = (usize, Result<R, E>)> + Send + 'static,
{
    let n = items.len();
    let mut out: Vec<Option<Result<R, JoinOutcome<E>>>> = (0..n).map(|_| None).collect();
    // Task id -> item index. The map holds at most `jobs` entries: a completion
    // removes its id and the replacement inserts a new one.
    let mut slots: std::collections::HashMap<tokio::task::Id, usize> =
        std::collections::HashMap::new();
    let mut set = tokio::task::JoinSet::new();
    let mut next = 0_usize;

    while next < n && set.len() < jobs.max(1) {
        let i = next;
        next += 1;
        slots.insert(set.spawn(f(i, items[i].clone())).id(), i);
    }
    while let Some(joined) = set.join_next_with_id().await {
        match joined {
            Ok((id, (i, r))) => {
                slots.remove(&id);
                out[i] = Some(r.map_err(JoinOutcome::Failed));
            }
            Err(e) => {
                // Recover the index *before* `join_failure` consumes the error,
                // so the cause can travel with the item it belongs to.
                let id = e.id();
                let why = join_failure(e, task);
                match slots.remove(&id) {
                    Some(i) => {
                        tracing::error!(
                            item = i,
                            error = %why,
                            "a {task} task did not report"
                        );
                        out[i] = Some(Err(JoinOutcome::Lost { why: Some(why) }));
                    }
                    // Unreachable while every spawn's id is recorded, but a
                    // failure must still be visible rather than silently lost
                    // if that ever stops being true.
                    None => tracing::error!(
                        error = %why,
                        "a {task} task did not report and its index could not be recovered"
                    ),
                }
            }
        }
        if next < n {
            let i = next;
            next += 1;
            slots.insert(set.spawn(f(i, items[i].clone())).id(), i);
        }
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
pub(crate) async fn join_blocking<T, F>(task: &str, work: F) -> Result<T, String>
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
        let items: Vec<usize> = (0..8).collect();
        let got = join_ordered(&items, 3, "probe", |i, item| async move {
            // Later items finish first, so completion order is the reverse of
            // input order unless each result stays in its own slot.
            tokio::time::sleep(std::time::Duration::from_millis((8 - item) as u64)).await;
            (i, Ok::<usize, String>(item * 2))
        })
        .await;
        let values: Vec<usize> = got
            .into_iter()
            .map(|r| r.unwrap_or_else(|_| panic!("no probe task was lost")))
            .collect();
        assert_eq!(values, (0..8).map(|i| i * 2).collect::<Vec<_>>());
    }

    #[tokio::test]
    async fn a_panicking_item_keeps_its_slot_and_cause() {
        let items = [0usize, 1, 2];
        let got = join_ordered(&items, 2, "probe", |i, item| async move {
            if i == 1 {
                panic!("probe exploded");
            }
            (i, Ok::<usize, String>(item * 10))
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
