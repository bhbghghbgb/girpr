//! A canned-reply loopback origin shared by the unit tests.
//!
//! Several tests need an origin that answers with fixed bytes or a fixed
//! status and counts the requests it saw. The server deliberately has no route
//! logic: the tests that need per-target replies or a gated response own those
//! servers themselves, because that routing is the thing they exercise.
//!
//! The listener, response writer and request journal come from the
//! dependency-free loopback leaf; these helpers only script the status/body
//! sequence and read back the hit count. This contract stays local to fetch —
//! the product's and both capture mocks script different replies and pin
//! different parts of the journal.

use girpr_loopback_mock::{Reply, ScriptedServer};

/// A canned server. The hit count is the only record fetch's tests read.
pub(crate) struct Mock {
    server: ScriptedServer,
}

impl Mock {
    /// How many requests reached the server.
    pub(crate) async fn hits(&self) -> usize {
        self.server.hits()
    }

    /// `http://127.0.0.1:<port>`.
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

/// A throwaway HTTP server that answers the nth request with `status` and the
/// nth body, repeating the last one, and counts the requests it saw.
///
/// Used where a caller must prove a status is decided before its body is read
/// (for example a non-2xx carrying a JSON wrapper that used to be trusted).
pub(crate) async fn serve_with_status(status: u16, bodies: Vec<&'static str>) -> Mock {
    serve(status, bodies).await
}

/// The canned-response setup the two servers above share.
///
/// The script walks `bodies` in order, repeating the last one once it runs out.
async fn serve(status: u16, bodies: Vec<&'static str>) -> Mock {
    let replies = bodies
        .into_iter()
        .map(|body| Reply::new(status, body))
        .collect();
    Mock {
        server: ScriptedServer::start(replies),
    }
}
