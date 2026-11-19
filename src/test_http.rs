//! A canned-reply loopback origin shared by the unit tests.
//!
//! Several tests need an origin that answers with fixed bytes or a fixed
//! status and counts the requests it saw. The server deliberately has no route
//! logic: the tests that need per-target replies or a gated response own those
//! servers themselves, because that routing is the thing they exercise.
//!
//! The listener, response writer and request journal belong to `wiremock`;
//! these helpers only script the status/body sequence and read back what the
//! library recorded.

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
    /// How many requests reached the server, from the library's journal.
    pub(crate) async fn hits(&self) -> usize {
        self.server
            .received_requests()
            .await
            .expect("the request journal is enabled")
            .len()
    }

    /// `http://127.0.0.1:<port>`, from the library.
    pub(crate) fn uri(&self) -> String {
        self.server.uri()
    }
}

/// A throwaway HTTP server that answers the nth request with the nth body,
/// repeating the last one, and counts the requests it saw.
///
/// Every reply is `200 OK` with the body as its bytes.
pub(crate) async fn serve_bodies(bodies: Vec<&'static str>) -> Mock {
    serve(200, bodies).await
}

/// A throwaway HTTP server that answers every request with `status` and an
/// empty body, and counts the requests it saw.
///
/// The status is fixed: this server never changes its mind between attempts.
pub(crate) async fn serve_status(status: u16) -> Mock {
    serve(status, vec![""]).await
}

/// The canned-response setup the two servers above share.
///
/// The responder walks `bodies` in order, repeating the last one once the
/// script runs out. The count is kept only to advance that cursor; the
/// authoritative request log is the library's journal.
async fn serve(status: u16, bodies: Vec<&'static str>) -> Mock {
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
            ResponseTemplate::new(status).set_body_bytes(answers[i].clone())
        })
        .mount(&server)
        .await;
    Mock { server }
}
