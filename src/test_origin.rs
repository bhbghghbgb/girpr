//! A shared loopback origin for the repair phases' unit tests.
//!
//! The caller owns the routing and reply policy; axum owns the listener, HTTP
//! parsing, keep-alive and response framing. Dropping the server tells every
//! in-flight handler to stop and aborts the accept loop, so a route that never
//! answers cannot outlive the test that started it.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::Router;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use tokio::net::TcpListener;
use tokio::sync::{Notify, watch};
use tokio::task::JoinHandle;

/// One reply a route can give. Variants that carry a counter or a gate let a
/// caller hold a request open or let it through once another route was hit,
/// without the test having to time anything.
#[derive(Clone)]
pub(crate) enum Reply {
    Body(Vec<u8>),
    Status(&'static str),
    /// Count the request, then keep it open without answering, so an
    /// uncancelled fetch would wait forever.
    NeverAfterHit(Arc<AtomicUsize>),
    /// Answer `status` only once another route has been hit, so the failure
    /// settles with that fetch demonstrably in flight.
    StatusAfterHit(&'static str, Arc<AtomicUsize>),
    /// Serve `body` after the test releases the gate, so a test observes how
    /// many requests the client started rather than timing it.
    GatedBody {
        body: Vec<u8>,
        gate: Arc<InFlight>,
    },
}

impl Reply {
    /// A body supplied as text, for callers whose canned replies are string
    /// constants rather than already-encoded bytes.
    pub(crate) fn text(body: &'static str) -> Self {
        Self::Body(body.as_bytes().to_vec())
    }
}

/// The caller-owned routing policy this server drives.
type Route = Arc<dyn Fn(&str) -> Option<Reply> + Send + Sync>;

/// Requests currently being served, with a gate that holds them until the
/// test lets go, so a concurrency bound is observed instead of timed.
pub(crate) struct InFlight {
    pub(crate) current: AtomicUsize,
    pub(crate) peak: AtomicUsize,
    arrived: Notify,
    pub(crate) release: watch::Sender<bool>,
}

impl InFlight {
    pub(crate) fn new() -> Self {
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
    pub(crate) async fn enter(self: &Arc<Self>) -> InFlightGuard {
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
    pub(crate) async fn wait_for(&self, want: usize) {
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
    pub(crate) async fn wait_until_idle(&self) {
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

/// Owns one counted request slot; dropping it always releases the slot.
pub(crate) struct InFlightGuard {
    gate: Arc<InFlight>,
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        self.gate.current.fetch_sub(1, Ordering::SeqCst);
        self.gate.arrived.notify_waiters();
    }
}

/// Handler state: the caller's route plus the signal that ends the test.
struct OriginPolicy {
    route: Route,
    stopping: watch::Receiver<bool>,
}

/// Answer one request by applying the caller's route and running its reply
/// policy. axum owns the request line, headers and response framing.
async fn handle(State(policy): State<Arc<OriginPolicy>>, request: Request) -> Response {
    let target = request
        .uri()
        .path_and_query()
        .map(|value| value.as_str())
        .unwrap_or("/")
        .to_owned();
    answer((policy.route)(&target), &policy.stopping).await
}

async fn answer(reply: Option<Reply>, stopping: &watch::Receiver<bool>) -> Response {
    let mut stopping = stopping.clone();
    let respond = async {
        match reply {
            Some(Reply::Body(body)) => (StatusCode::OK, body).into_response(),
            Some(Reply::Status(reason)) => status_code(reason).into_response(),
            Some(Reply::NeverAfterHit(hits)) => {
                hits.fetch_add(1, Ordering::SeqCst);
                std::future::pending::<()>().await;
                unreachable!("a never-answering route holds the request open")
            }
            Some(Reply::StatusAfterHit(reason, hits)) => {
                while hits.load(Ordering::SeqCst) == 0 {
                    tokio::task::yield_now().await;
                }
                status_code(reason).into_response()
            }
            Some(Reply::GatedBody { body, gate }) => {
                let _guard = gate.enter().await;
                (StatusCode::OK, body).into_response()
            }
            None => StatusCode::NOT_FOUND.into_response(),
        }
    };
    // A held request must not outlive its test: cancelling `respond` drops
    // the in-flight guard and lets hyper close the connection.
    tokio::select! {
        response = respond => response,
        _ = until_stopping(&mut stopping) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}

async fn until_stopping(receiver: &mut watch::Receiver<bool>) {
    while !*receiver.borrow_and_update() {
        if receiver.changed().await.is_err() {
            break;
        }
    }
}

/// Scripted statuses carry a full reason phrase; axum takes a typed code and
/// writes the canonical reason line itself.
fn status_code(reason: &'static str) -> StatusCode {
    let digits = reason.split_whitespace().next().unwrap_or(reason);
    StatusCode::from_bytes(digits.as_bytes()).expect("scripted status carries a 3-digit code")
}

/// A running test origin. Dropping it releases every held handler and aborts
/// the accept loop, so the test never depends on runtime teardown to stop a
/// route that never answers.
pub(crate) struct TestServer {
    base: String,
    driver: JoinHandle<()>,
    stopping: watch::Sender<bool>,
}

impl TestServer {
    pub(crate) fn base(&self) -> &str {
        &self.base
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        // Signal first: axum spawns each connection as a detached task, so
        // aborting the driver alone would leave gated handlers waiting.
        let _ = self.stopping.send_replace(true);
        self.driver.abort();
    }
}

/// Serve replies chosen by `route` on an ephemeral loopback port. The caller
/// owns the routing policy; this module owns the socket and axum writes the
/// response bytes. `None` is a `404`, the same as an unknown target before the
/// fold-in.
pub(crate) async fn serve<F>(route: F) -> TestServer
where
    F: Fn(&str) -> Option<Reply> + Send + Sync + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let base = format!("http://{}", listener.local_addr().expect("addr"));
    let (stopping, _) = watch::channel(false);
    let policy = Arc::new(OriginPolicy {
        route: Arc::new(route),
        stopping: stopping.subscribe(),
    });
    let router = Router::new().fallback(handle).with_state(policy);
    let driver = tokio::spawn(async move {
        axum::serve(listener, router).await.expect("serve");
    });

    TestServer {
        base,
        driver,
        stopping,
    }
}
