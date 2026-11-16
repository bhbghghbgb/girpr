//! A canned-reply loopback origin shared by the unit tests.
//!
//! Several tests need an origin that answers with fixed bytes or a fixed
//! status and counts the requests it saw. The server deliberately has no route
//! logic: the tests that need per-target replies or a gated response own those
//! servers themselves, because that routing is the thing they exercise.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A throwaway HTTP server that answers the nth request with the nth body,
/// repeating the last one, and counts the requests it saw.
///
/// The return value is the base URL and the number of accepted connections.
/// Every reply is `200 OK` with the body's length as `Content-Length` and a
/// closed connection, which is all the callers here need.
pub(crate) async fn serve_bodies(bodies: Vec<&'static str>) -> (String, Arc<AtomicUsize>) {
    assert!(!bodies.is_empty(), "the server needs at least one body");
    serve("200 OK", bodies).await
}

/// A throwaway HTTP server that answers every request with `status` and an
/// empty body, and counts the requests it saw.
///
/// The return value is the base URL and the number of accepted connections.
/// The status is fixed: this server never changes its mind between attempts.
pub(crate) async fn serve_status(status: &'static str) -> (String, Arc<AtomicUsize>) {
    serve(status, vec![""]).await
}

/// The accept/count/one-shot-reply loop the two servers above share.
async fn serve(status: &'static str, bodies: Vec<&'static str>) -> (String, Arc<AtomicUsize>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind the test server");
    let addr = listener.local_addr().expect("the test server address");
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&hits);
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let n = counter.fetch_add(1, Ordering::Relaxed);
            let body = bodies[n.min(bodies.len() - 1)];
            tokio::spawn(async move {
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let mut buf = [0u8; 4096];
                let _ = socket.read(&mut buf).await;
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.flush().await;
            });
        }
    });
    (format!("http://{addr}"), hits)
}
