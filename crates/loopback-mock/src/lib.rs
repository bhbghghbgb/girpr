//! A wiremock-backed loopback HTTP server for canned test replies.
//!
//! Each [`ScriptedServer`] owns a `wiremock` `MockServer` on its own ephemeral
//! loopback port, answers a scripted sequence of replies (the last one repeats)
//! and records the requests it saw. Listener, HTTP parsing, keep-alive, header
//! writing and shutdown all belong to the library; this crate keeps only the
//! small scripting contract its callers share: reply order, the invocation
//! count and the request journal.
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

use std::sync::{Arc, Mutex};
use std::thread;

use wiremock::http::HeaderMap;
use wiremock::matchers::any;
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

/// One scripted reply: a status, a body and the response headers to send with
/// it. Header names are lowercased when they are added, so every caller spells
/// the same header the same way on the wire. Repeating a name appends another
/// header line, in the order the values were added.
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
    pub headers: HeaderMap,
}

/// Turns a scripted reply into a wiremock response and records the incoming
/// request synchronously. wiremock's own journal is async to read; recording in
/// the responder keeps `hits()` and `requests()` on the same synchronous API the
/// callers already use, and counts exactly the requests that a mock answered.
struct RecordingRespond {
    template: ResponseTemplate,
    journal: Arc<Mutex<Vec<RecordedRequest>>>,
}

impl Respond for RecordingRespond {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let target = format!(
            "{}{}",
            request.url.path(),
            request
                .url
                .query()
                .map(|query| format!("?{query}"))
                .unwrap_or_default()
        );
        self.journal.lock().unwrap().push(RecordedRequest {
            method: request.method.as_str().to_owned(),
            target,
            headers: request.headers.clone(),
        });
        self.template.clone()
    }
}

impl Reply {
    /// The reply's bytes as a wiremock response. An empty mime leaves the
    /// caller's own `content-type` headers (if any) untouched.
    fn into_template(self) -> ResponseTemplate {
        let mut template = ResponseTemplate::new(self.status);
        for (name, value) in self.headers {
            template = template.append_header(name, value);
        }
        template.set_body_raw(self.body, "")
    }
}

/// A canned server on its own ephemeral loopback port.
///
/// Dropping it stops the underlying wiremock server before returning: no
/// listener or background task outlives the test that started it. A server
/// built by [`ScriptedServer::start_expecting`] also verifies its exact hit
/// count on drop.
pub struct ScriptedServer {
    base: String,
    // Kept for its `Drop`: wiremock shuts the server down and verifies its
    // (empty) expectation set when this field is dropped.
    _server: MockServer,
    journal: Arc<Mutex<Vec<RecordedRequest>>>,
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
        let journal = Arc::new(Mutex::new(Vec::new()));
        let last = replies.len() - 1;
        let templates: Vec<ResponseTemplate> =
            replies.into_iter().map(Reply::into_template).collect();

        // wiremock's constructors are async and need a Tokio reactor. The
        // leaf's API is synchronous and is frequently called from inside an
        // existing runtime, so drive a fresh runtime on a short-lived helper
        // thread instead of nesting a `Runtime::block_on`.
        let thread_journal = Arc::clone(&journal);
        let server = thread::scope(|scope| {
            scope
                .spawn(move || {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .expect("build tokio runtime");
                    runtime.block_on(async {
                        let server = MockServer::start().await;
                        for (index, template) in templates.into_iter().enumerate() {
                            let mut mock = Mock::given(any()).respond_with(RecordingRespond {
                                template,
                                journal: Arc::clone(&thread_journal),
                            });
                            // Equal-priority mocks are tried in registration
                            // order. Every reply answers once except the last,
                            // which has no cap — that is "last reply repeats".
                            if index != last {
                                mock = mock.up_to_n_times(1);
                            }
                            mock.mount(&server).await;
                        }
                        server
                    })
                })
                .join()
                .expect("mock server start thread panicked")
        });

        Self {
            base: server.uri(),
            _server: server,
            journal,
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
        self.journal.lock().unwrap().len()
    }

    /// Every request recorded so far, in arrival order.
    pub fn requests(&self) -> Vec<RecordedRequest> {
        self.journal.lock().unwrap().clone()
    }
}

impl Drop for ScriptedServer {
    fn drop(&mut self) {
        if let Some(expected) = self.expected {
            let got = self.hits() as u64;
            assert_eq!(
                got, expected,
                "unexpected number of requests reached the mock"
            );
        }
        // `self.server` is dropped next: wiremock shuts the server down and
        // verifies its own expectations (none are registered here).
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpStream;

    fn port_of(server: &ScriptedServer) -> u16 {
        server
            .base()
            .rsplit(':')
            .next()
            .expect("base has a port")
            .parse()
            .expect("numeric port")
    }

    /// Status and body of one raw GET, so the script can be pinned without
    /// taking a dependency on an HTTP client in this leaf.
    fn get(base_port: u16) -> (u16, String) {
        let mut stream = TcpStream::connect(("127.0.0.1", base_port)).expect("connect");
        stream
            .write_all(b"GET /x HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .expect("write request");
        let mut bytes = Vec::new();
        stream.read_to_end(&mut bytes).expect("read response");
        let text = String::from_utf8_lossy(&bytes);
        let status = text
            .split_whitespace()
            .nth(1)
            .expect("status line")
            .parse()
            .expect("numeric status");
        let body = text
            .split_once("\r\n\r\n")
            .expect("end of headers")
            .1
            .trim()
            .to_owned();
        (status, body)
    }

    /// Replies are consumed in script order, then the last one repeats.
    #[test]
    fn scripted_replies_run_in_order_and_the_last_repeats() {
        let server =
            ScriptedServer::start(vec![Reply::new(201, "first"), Reply::new(202, "second")]);
        let port = port_of(&server);
        let seen: Vec<_> = (0..4).map(|_| get(port)).collect();
        assert_eq!(
            seen,
            vec![
                (201, "first".to_owned()),
                (202, "second".to_owned()),
                (202, "second".to_owned()),
                (202, "second".to_owned()),
            ]
        );
        assert_eq!(server.hits(), 4);
        assert_eq!(server.requests().len(), 4);
    }

    /// A scripted answer that was never consumed fails the pinned count on drop.
    #[test]
    #[should_panic(expected = "unexpected number of requests reached the mock")]
    fn start_expecting_asserts_hit_count_on_drop() {
        let server = ScriptedServer::start_expecting(vec![Reply::new(200, "ok")], 1);
        assert_eq!(server.hits(), 0);
    }
}
