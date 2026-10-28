//! The one rule for a [`tokio::task::JoinError`].
//!
//! Two `JoinSet` loops run user work: [`crate::blobs::Store::fill`] and
//! [`crate::http::Fetcher::map_concurrently`]. Both can lose a task to a panic
//! or to cancellation, and both used to throw the `JoinError` away — one
//! silently, one behind a generic "panicked" row. The cause an operator needs
//! is in the payload, not in `Display`, so the rule lives here once and both
//! loops call it with their own label.

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
}
