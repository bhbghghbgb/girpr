//! A canned-reply loopback origin for the product's own unit tests.
//!
//! Tests here need an origin that answers with fixed bytes; the server has no
//! route logic, so a test that needs per-target replies or a gated response
//! owns its own server because that routing is what it exercises.
//!
//! This is deliberately a local helper, not a shared fixture: the fetch crate
//! and both capture tools keep their own canned servers because each encodes
//! a different contract. The listener, response writer and request journal
//! come from the wiremock-backed loopback leaf; this helper only scripts the
//! body sequence.

use girpr_loopback_mock::{Reply, ScriptedServer};

/// A canned server. This helper's tests read the base URL only.
pub(crate) struct Mock {
    server: ScriptedServer,
}

impl Mock {
    /// `http://127.0.0.1:<port>`.
    pub(crate) fn uri(&self) -> String {
        self.server.uri()
    }
}

/// A throwaway HTTP server that answers the nth request with the nth body,
/// repeating the last one. Every reply is `200 OK` with the body as its bytes.
pub(crate) async fn serve_bodies(bodies: Vec<&'static str>) -> Mock {
    let replies = bodies
        .into_iter()
        .map(|body| Reply::new(200, body))
        .collect();
    Mock {
        server: ScriptedServer::start(replies),
    }
}
