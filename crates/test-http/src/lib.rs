//! Shared socket/stop/gate mechanics for axum-backed test servers.
//!
//! Every axum test server in the workspace repeats the same three mechanics:
//! bind an ephemeral loopback port, keep a stop signal that a `Drop` can send
//! from the test's thread, and (when a test needs to observe concurrency rather
//! than time it) hold requests in an in-flight gate. This leaf owns exactly
//! those mechanics and nothing else.
//!
//! It deliberately knows no status, body, route table or journal assertion. The
//! caller injects its own axum [`Router`] factory — which is where its canned
//! reply policy lives — and keeps its own contract on top. The wiremock-backed
//! `girpr-loopback-mock` leaf stays separate: axum does not enter it, and the
//! canned wiremock server, the axum replay server and the axum world server keep
//! their distinct contracts.
//!
//! A bind or spawn failure here is a broken test rather than a condition a
//! caller can recover from, so the test-only `expect`/`unwrap` style applies.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::Router;
use tokio::net::TcpListener;
use tokio::sync::{Notify, watch};
use tokio::task::JoinHandle;

/// Requests currently being served, with a gate that holds them until the test
/// lets go, so a concurrency bound is observed instead of timed.
///
/// The counters and the release sender are public because they are the seam the
/// tests use: a test reads `current`/`peak` and sends `true` on `release` once it
/// has observed the bound. The arrival notifications stay private; a caller
/// waits through [`InFlight::wait_for`] / [`InFlight::wait_until_idle`] instead.
pub struct InFlight {
    pub current: AtomicUsize,
    pub peak: AtomicUsize,
    arrived: Notify,
    pub release: watch::Sender<bool>,
}

impl InFlight {
    pub fn new() -> Self {
        let (release, _) = watch::channel(false);
        Self {
            current: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            arrived: Notify::new(),
            release,
        }
    }

    /// Count this request as in flight and wait for the release.
    ///
    /// The returned guard owns the slot, so dropping the request future
    /// (for example when the handler is cancelled mid-wait) always releases
    /// it, even though the handler never resumes to call `leave` itself.
    pub async fn enter(self: &Arc<Self>) -> InFlightGuard {
        // Own the slot before the first await, so a cancel at any point
        // releases it through the guard's `Drop`.
        let guard = InFlightGuard {
            gate: Arc::clone(self),
        };
        let now = self.current.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(now, Ordering::SeqCst);
        self.arrived.notify_waiters();
        let mut release = self.release.subscribe();
        while !*release.borrow_and_update() {
            if release.changed().await.is_err() {
                break;
            }
        }
        guard
    }

    /// Wait until `want` requests are in flight, without assuming when.
    pub async fn wait_for(&self, want: usize) {
        loop {
            let arrived = self.arrived.notified();
            tokio::pin!(arrived);
            // Register before reading the count, so a wakeup between the
            // check and the await cannot be lost on any runtime flavor.
            arrived.as_mut().enable();
            if self.current.load(Ordering::SeqCst) >= want {
                return;
            }
            arrived.as_mut().await;
        }
    }

    /// Wait until every counted request has been released, without assuming
    /// when.
    pub async fn wait_until_idle(&self) {
        loop {
            let arrived = self.arrived.notified();
            tokio::pin!(arrived);
            arrived.as_mut().enable();
            if self.current.load(Ordering::SeqCst) == 0 {
                return;
            }
            arrived.as_mut().await;
        }
    }
}

impl Default for InFlight {
    fn default() -> Self {
        Self::new()
    }
}

/// Owns one counted request slot; dropping it always releases the slot.
pub struct InFlightGuard {
    gate: Arc<InFlight>,
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        self.gate.current.fetch_sub(1, Ordering::SeqCst);
        self.gate.arrived.notify_waiters();
    }
}

/// A running test server. Dropping it signals the stop channel first (so an
/// axum handler that waits on it can settle) and then aborts the accept loop, so
/// a route that never answers cannot outlive the test that started it.
pub struct Server {
    base: String,
    driver: JoinHandle<()>,
    stopping: watch::Sender<bool>,
}

impl Server {
    /// `http://127.0.0.1:<port>` for this instance.
    pub fn base(&self) -> &str {
        &self.base
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        // Signal first: axum spawns each connection as a detached task, so
        // aborting the driver alone would leave gated handlers waiting.
        let _ = self.stopping.send_replace(true);
        self.driver.abort();
    }
}

/// Serve a caller-built router on an ephemeral loopback port.
///
/// `make_router` receives the stop receiver so the caller's state/handlers can
/// settle a held request when the test ends; the matching sender is owned by
/// the returned [`Server`] and fires in its `Drop`. The caller supplies the
/// router — and therefore the whole reply policy — while this leaf supplies the
/// bind, the driver task and the shutdown.
pub async fn serve<F>(make_router: F) -> Server
where
    F: FnOnce(watch::Receiver<bool>) -> Router,
{
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let base = format!("http://{}", listener.local_addr().expect("addr"));
    let (stopping, stopped) = watch::channel(false);
    let router = make_router(stopped);
    let driver = tokio::spawn(async move {
        axum::serve(listener, router).await.expect("serve");
    });
    Server {
        base,
        driver,
        stopping,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;

    #[tokio::test]
    async fn gate_counts_guards_and_waits_without_timing() {
        let gate = Arc::new(InFlight::new());
        let worker = {
            let gate = Arc::clone(&gate);
            tokio::spawn(async move {
                let _guard = gate.enter().await;
            })
        };
        tokio::time::timeout(Duration::from_secs(2), gate.wait_for(1))
            .await
            .expect("the request reaches the gate");
        assert_eq!(gate.current.load(Ordering::SeqCst), 1);
        assert_eq!(gate.peak.load(Ordering::SeqCst), 1);
        let _ = gate.release.send(true);
        worker.await.expect("the released handler finished");
        tokio::time::timeout(Duration::from_secs(2), gate.wait_until_idle())
            .await
            .expect("the guard released its slot");
        assert_eq!(gate.current.load(Ordering::SeqCst), 0);
        assert_eq!(gate.peak.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn server_binds_and_serves_the_injected_router() {
        let server = serve(|_| Router::new().fallback(|| async { "ok" })).await;
        assert!(
            server.base().starts_with("http://127.0.0.1:"),
            "{}",
            server.base()
        );
        let port: u16 = server
            .base()
            .rsplit(':')
            .next()
            .expect("base has a port")
            .parse()
            .expect("numeric port");
        let mut stream = TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("connect");
        stream
            .write_all(b"GET /x HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .await
            .expect("write request");
        let mut bytes = Vec::new();
        stream.read_to_end(&mut bytes).await.expect("read response");
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("ok"), "unexpected response: {text}");
    }
}
