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
use std::net::{SocketAddr, TcpListener, TcpStream};
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
/// releases a blocked `accept`, and the driver thread is joined, so no
/// listener or thread outlives the test that started it.
pub struct ScriptedServer {
    base: String,
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    driver: Option<JoinHandle<()>>,
    journal: Arc<Mutex<Vec<RecordedRequest>>>,
    hits: Arc<AtomicUsize>,
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

        let driver_stop = Arc::clone(&stop);
        let driver_journal = Arc::clone(&journal);
        let driver_hits = Arc::clone(&hits);
        let driver = thread::spawn(move || {
            for stream in listener.incoming() {
                if driver_stop.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(mut stream) = stream else {
                    break;
                };
                serve_one(&mut stream, &replies, &driver_journal, &driver_hits);
            }
        });

        Self {
            base,
            addr,
            stop,
            driver: Some(driver),
            journal,
            hits,
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
            // A connection releases a `accept` that is already blocked; the
            // driver sees the stop flag before it can answer it.
            let _ = TcpStream::connect(self.addr);
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

    let mut parts = request_line.trim_end().split_whitespace();
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
