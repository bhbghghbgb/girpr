//! A shared loopback origin for the repair phases' unit tests.
//!
//! The caller owns the routing and reply policy; axum owns the listener, HTTP
//! parsing, keep-alive and response framing. The ephemeral bind, the stop
//! signal and the in-flight gate are shared with the other axum test servers
//! through `girpr-test-http`; this module keeps only the repair phases'
//! route→reply contract. Dropping the server tells every in-flight handler to
//! stop and aborts the accept loop, so a route that never answers cannot outlive
//! the test that started it.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::Router;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use tokio::sync::watch;

// The gate and the server handle are mechanics with no reply policy: they live
// in the shared leaf and are re-exported under the names the phase tests use.
pub(crate) use girpr_test_http::{InFlight, Server as TestServer};

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

/// Serve replies chosen by `route` on an ephemeral loopback port. The caller
/// owns the routing policy; the leaf owns the socket and axum writes the
/// response bytes. `None` is a `404`, the same as an unknown target before the
/// fold-in.
pub(crate) async fn serve<F>(route: F) -> TestServer
where
    F: Fn(&str) -> Option<Reply> + Send + Sync + 'static,
{
    girpr_test_http::serve(move |stopping| {
        let policy = Arc::new(OriginPolicy {
            route: Arc::new(route),
            stopping,
        });
        Router::new().fallback(handle).with_state(policy)
    })
    .await
}
