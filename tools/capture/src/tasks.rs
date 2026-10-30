//! The one rule for a [`tokio::task::JoinError`].
//!
//! Two `JoinSet` loops run user work: [`crate::blobs::Store::fill`] and
//! [`crate::http::Fetcher::map_concurrently`]. Both can lose a task to a panic
//! or to cancellation, and throwing the `JoinError` away — silently, or behind
//! a generic "panicked" row — loses the cause an operator needs, which is in
//! the payload, not in `Display`. The rule lives here once and both loops call
//! it with their own label. The two direct `spawn_blocking` joins —
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
