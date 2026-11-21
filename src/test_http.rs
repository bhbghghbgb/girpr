//! A canned-reply loopback origin for the product's own unit tests.
//!
//! Tests here need an origin that answers with fixed bytes; the server has no
//! route logic, so a test that needs per-target replies or a gated response
//! owns its own server because that routing is what it exercises.
//!
//! This is deliberately a local helper, not a shared fixture: the fetch crate
//! and both capture tools keep their own canned servers because each encodes
//! a different contract. The listener, response writer and request journal
//! belong to `wiremock`; this helper only scripts the body sequence.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use wiremock::matchers::any;
use wiremock::{Mock as Reply, MockServer, Request, ResponseTemplate};

/// A canned server. The library's request journal is the only record of what
/// was served; there is no hand-rolled accept loop or response writer here.
pub(crate) struct Mock {
    server: MockServer,
}

impl Mock {
    /// `http://127.0.0.1:<port>`, from the library.
    pub(crate) fn uri(&self) -> String {
        self.server.uri()
    }
}

/// A throwaway HTTP server that answers the nth request with the nth body,
/// repeating the last one. Every reply is `200 OK` with the body as its bytes.
pub(crate) async fn serve_bodies(bodies: Vec<&'static str>) -> Mock {
    let answers: Vec<Vec<u8>> = bodies
        .into_iter()
        .map(|body| body.as_bytes().to_vec())
        .collect();
    assert!(
        !answers.is_empty(),
        "a canned server needs at least one body"
    );
    let cursor = Arc::new(AtomicUsize::new(0));
    let server = MockServer::start().await;
    let next = Arc::clone(&cursor);
    Reply::given(any())
        .respond_with(move |_req: &Request| {
            let i = next.fetch_add(1, Ordering::SeqCst).min(answers.len() - 1);
            ResponseTemplate::new(200).set_body_bytes(answers[i].clone())
        })
        .mount(&server)
        .await;
    Mock { server }
}
