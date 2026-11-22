//! A dependency-free loopback HTTP server for canned test replies.
//!
//! Each [`ScriptedServer`] binds its own ephemeral loopback port, answers a
//! scripted sequence of replies (the last one repeats) and records the requests
//! it saw. There is no route logic and no runtime dependency: the listener,
//! the reply writer and the request journal are all in this crate, using only
//! `std`.
//!
//! The server deliberately knows nothing about what a reply or a request means
//! to any program that uses it. Each caller keeps its own canned-server
//! contract on top — which statuses and headers it scripts, which hit counts it
//! pins and which parts of the journal it asserts — so the repair, fetch and
//! capture tests do not end up sharing a single fake-server policy.
//!
//! A bind or mutex failure here is a broken test rather than a condition a
//! caller can recover from, so the test-only `expect`/`unwrap` style used by the
//! other fixture helpers applies to this crate too.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::io::{BufRead, BufReader, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

/// One scripted reply: a status, a body and the response headers to send with
/// it. Header names are lowercased when they are added, so every caller spells
/// the same header the same way on the wire.
#[derive(Clone, Debug)]
pub struct Reply {
    pub status: u16,
    pub body: Vec<u8>,
    pub headers: Vec<(String, String)>,
}

impl Reply {
    /// A reply with the given status and body bytes and no extra headers.
    pub fn new(status: u16, body: impl AsRef<[u8]>) -> Self {
        Self {
            status,
            body: body.as_ref().to_vec(),
            headers: Vec::new(),
        }
    }

    /// Append a response header. A name with several values produces several
    /// header lines, in the order they were appended.
    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers
            .push((name.to_ascii_lowercase(), value.to_owned()));
        self
    }
}

/// One request the server received, as recorded in its journal.
#[derive(Clone, Debug)]
pub struct RecordedRequest {
    pub method: String,
    pub target: String,
    pub headers: Headers,
}

/// The request headers of one [`RecordedRequest`], keyed case-insensitively.
#[derive(Clone, Debug, Default)]
pub struct Headers(Vec<(String, HeaderValue)>);

impl Headers {
    /// The first value recorded for `name`, if any.
    pub fn get(&self, name: &str) -> Option<&HeaderValue> {
        self.0
            .iter()
            .find(|(recorded, _)| recorded.eq_ignore_ascii_case(name))
            .map(|(_, value)| value)
    }

    /// Whether a header with `name` was recorded.
    pub fn contains_key(&self, name: &str) -> bool {
        self.get(name).is_some()
    }
}

/// One recorded header value. `to_str` mirrors the accessor callers use to
/// compare a value with the string they expect on the wire.
#[derive(Clone, Debug)]
pub struct HeaderValue(String);

impl HeaderValue {
    pub fn to_str(&self) -> Result<&str, std::str::Utf8Error> {
        Ok(&self.0)
    }
}

/// A canned server on its own ephemeral loopback port.
///
/// Dropping it stops the accept loop before returning: a wake-up connection
/// releases a blocked `accept`, the in-flight connection (if any) is shut down
/// so a stalled request read cannot keep the driver alive, and the driver
/// thread is joined. No listener or thread outlives the test that started it.
pub struct ScriptedServer {
    base: String,
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    driver: Option<JoinHandle<()>>,
    journal: Arc<Mutex<Vec<RecordedRequest>>>,
    hits: Arc<AtomicUsize>,
    active: Arc<Mutex<Option<TcpStream>>>,
    expected: Option<u64>,
}

impl ScriptedServer {
    /// Start a server that answers `replies` in order, repeating the last one.
    pub fn start(replies: Vec<Reply>) -> Self {
        Self::start_inner(replies, None)
    }

    /// Start a server that also verifies the request count on drop. A caller
    /// that scripts an answer it must **not** consume uses this form to pin the
    /// exact number of requests the test was supposed to make.
    pub fn start_expecting(replies: Vec<Reply>, expected: u64) -> Self {
        Self::start_inner(replies, Some(expected))
    }

    fn start_inner(replies: Vec<Reply>, expected: Option<u64>) -> Self {
        assert!(
            !replies.is_empty(),
            "a canned server needs at least one reply"
        );
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let addr = listener.local_addr().expect("local addr");
        let base = format!("http://{addr}");

        let stop = Arc::new(AtomicBool::new(false));
        let journal = Arc::new(Mutex::new(Vec::new()));
        let hits = Arc::new(AtomicUsize::new(0));
        // The connection currently being served, if any. `Drop` shuts its
        // socket down so a client that never finishes a request cannot stall
        // `driver.join()` in a read (or write) that has no other wake-up.
        let active = Arc::new(Mutex::new(None));

        let driver_stop = Arc::clone(&stop);
        let driver_journal = Arc::clone(&journal);
        let driver_hits = Arc::clone(&hits);
        let driver_active = Arc::clone(&active);
        let driver = thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else {
                    break;
                };
                // Publish the connection before serving it so `Drop` can always
                // reach it, then read the stop flag under the same lock `Drop`
                // uses to take the slot. Either we register before `Drop` takes
                // it (and `Drop` shuts the socket down), or we see the flag and
                // skip serving — so a connection accepted around a concurrent
                // `Drop` cannot slip into a blocking read untracked.
                {
                    let mut guard = driver_active.lock().unwrap();
                    *guard = Some(stream.try_clone().expect("clone stream"));
                    if driver_stop.load(Ordering::SeqCst) {
                        *guard = None;
                        break;
                    }
                }
                serve_one(&mut stream, &replies, &driver_journal, &driver_hits);
                *driver_active.lock().unwrap() = None;
            }
        });

        Self {
            base,
            addr,
            stop,
            driver: Some(driver),
            journal,
            hits,
            active,
            expected,
        }
    }

    /// `http://127.0.0.1:<port>` for this instance.
    pub fn base(&self) -> &str {
        &self.base
    }

    /// The same base URL under the name the canned-server helpers use.
    pub fn uri(&self) -> String {
        self.base.clone()
    }

    /// How many requests reached this server.
    pub fn hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }

    /// Every request recorded so far, in arrival order.
    pub fn requests(&self) -> Vec<RecordedRequest> {
        self.journal.lock().unwrap().clone()
    }
}

impl Drop for ScriptedServer {
    fn drop(&mut self) {
        if !self.stop.swap(true, Ordering::SeqCst) {
            // A connection releases an `accept` that is already blocked; the
            // driver sees the stop flag before it can answer it.
            let _ = TcpStream::connect(self.addr);
        }
        // If a connection is being served, its read (or write) may be blocked
        // on a client that never finishes or reads the request. Shutting the
        // shared socket down unblocks it so the driver can observe the stop
        // flag and exit, and `join` always returns.
        if let Some(stream) = self.active.lock().unwrap().take() {
            let _ = stream.shutdown(Shutdown::Both);
        }
        if let Some(driver) = self.driver.take() {
            let _ = driver.join();
        }
        if let Some(expected) = self.expected {
            let got = self.hits() as u64;
            assert_eq!(
                got, expected,
                "unexpected number of requests reached the mock"
            );
        }
    }
}

/// Read one request, record it, then answer it and close the connection.
fn serve_one(
    stream: &mut TcpStream,
    replies: &[Reply],
    journal: &Mutex<Vec<RecordedRequest>>,
    hits: &AtomicUsize,
) {
    let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
    let mut request_line = String::new();
    let Ok(n) = reader.read_line(&mut request_line) else {
        return;
    };
    if n == 0 {
        return;
    }

    let mut headers = Vec::new();
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        if line == "\r\n" || line == "\n" {
            break;
        }
        if let Some((name, value)) = line.trim_end_matches(['\r', '\n']).split_once(':') {
            headers.push((
                name.trim().to_ascii_lowercase(),
                HeaderValue(value.trim().to_owned()),
            ));
        }
    }

    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_owned();
    let target = parts.next().unwrap_or_default().to_owned();

    let index = hits.fetch_add(1, Ordering::SeqCst).min(replies.len() - 1);
    journal.lock().unwrap().push(RecordedRequest {
        method,
        target,
        headers: Headers(headers),
    });

    let reply = &replies[index];
    let mut head = format!("HTTP/1.1 {} {}\r\n", reply.status, reason(reply.status));
    for (name, value) in &reply.headers {
        head.push_str(name);
        head.push_str(": ");
        head.push_str(value);
        head.push_str("\r\n");
    }
    head.push_str(&format!(
        "Content-Length: {}\r\nConnection: close\r\n\r\n",
        reply.body.len()
    ));
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(&reply.body);
    let _ = stream.flush();
}

/// The reason phrase for the statuses the canned servers script. A caller
/// never reads it (every response is parsed by numeric status), but keeping
/// the usual spelling makes the reply line look like the real origin's.
fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        206 => "Partial Content",
        301 => "Moved Permanently",
        302 => "Found",
        304 => "Not Modified",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        415 => "Unsupported Media Type",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "Status",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    /// A client that connects, starts a request line and never finishes it
    /// must not keep `Drop` stuck in `driver.join()`.
    #[test]
    fn drop_does_not_hang_on_an_unfinished_request_line() {
        let server = ScriptedServer::start(vec![Reply::new(200, "ok")]);
        let addr = server.addr;

        let client = thread::spawn(move || {
            let mut stream = TcpStream::connect(addr).expect("connect");
            // No line terminator, so `serve_one` blocks in `read_line`. Then
            // block waiting for a reply that cannot arrive until the line ends.
            stream.write_all(b"GET / HTTP/1.").expect("write");
            let mut byte = [0u8; 1];
            let _ = stream.read(&mut byte);
        });

        // Wait for the driver to publish the accepted connection without
        // sleeping: yield until the slot fills or the deadline passes.
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline && server.active.lock().unwrap().is_none() {
            thread::yield_now();
        }
        assert!(
            server.active.lock().unwrap().is_some(),
            "server never accepted the stalled connection"
        );

        let (done_tx, done_rx) = mpsc::channel();
        thread::spawn(move || {
            drop(server);
            let _ = done_tx.send(());
        });
        done_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("ScriptedServer::drop blocked on the stalled read");

        // The shutdown also releases the client's blocking read.
        client.join().expect("client thread panicked");
    }
}
