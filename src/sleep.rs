//! Injectable sleep, so tests can skip retry backoff instead of spending it.
//!
//! Mirror of [`gircap::http::Sleep`]; kept in girpr because girpr has no
//! dependency on gircap and the seam belongs to the crate that sleeps.

use std::fmt;
use std::time::Duration;

/// Something that can be waited on, so tests can skip delays instead of
/// spending them.
///
/// **Async on purpose.** The real implementation uses [`tokio::time::sleep`],
/// which yields the worker rather than blocking it; a test implementation can
/// return immediately so a 15 s backoff is instant.
pub trait Sleep: Send + Sync + fmt::Debug {
    /// Wait for `d`. A no-sleep implementation returns immediately.
    fn sleep(&self, d: Duration) -> SleepFuture;
}

/// The future [`Sleep::sleep`] hands back. Boxed so the trait is object-safe.
pub type SleepFuture = std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'static>>;

/// Real wall-clock waiting, without taking the worker with it.
#[derive(Debug, Default)]
pub struct RealSleep;

impl Sleep for RealSleep {
    fn sleep(&self, d: Duration) -> SleepFuture {
        Box::pin(async move {
            if !d.is_zero() {
                tokio::time::sleep(d).await;
            }
        })
    }
}

/// No waiting: tests use this so retry backoff is instant.
#[derive(Debug, Default)]
pub struct NoSleep;

impl Sleep for NoSleep {
    fn sleep(&self, _d: Duration) -> SleepFuture {
        Box::pin(async {})
    }
}
