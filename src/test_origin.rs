//! A shared loopback origin for the repair phases' unit tests.
//!
//! The two hand-rolled servers in `repair::phases` used to repeat the same
//! accept loop, header parsing and response writing. This module owns that
//! transport once, keeps each phase's routing and reply policy in its caller,
//! and aborts the driver on drop so a route that never answers cannot outlive
//! the test that started it.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Notify, watch};
use tokio::task::{JoinHandle, JoinSet};

/// One reply a route can give. Variants that carry a counter or a gate let a
/// caller hold a request open or let it through once another route was hit,
/// without the test having to time anything.
#[derive(Clone)]
pub(crate) enum Reply {
    Body(Vec<u8>),
    Status(&'static str),
    /// Count the request, then keep the socket open without answering, so an
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
    /// (for example when its connection task is aborted mid-wait) always
    /// releases it, even though the handler never resumes to call `leave`
    /// itself.
    pub(crate) async fn enter(self: &Arc<Self>) -> InFlightGuard {
        // Own the slot before the first await, so an abort at any point
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

/// A running test origin. Dropping it cancels the accept loop, and the
/// `JoinSet` that loop owns aborts every connection task it spawned, so a
/// test never depends on runtime teardown to stop a route that never answers.
pub(crate) struct TestServer {
    base: String,
    driver: JoinHandle<()>,
    live: Arc<AtomicUsize>,
    idle: Arc<Notify>,
}

impl TestServer {
    pub(crate) fn base(&self) -> &str {
        &self.base
    }

    /// Wait until every accepted connection has been reaped.
    pub(crate) async fn wait_until_idle(&self) {
        loop {
            let idle = self.idle.notified();
            tokio::pin!(idle);
            // Register before reading the count, so a wakeup between the
            // check and the await cannot be lost on any runtime flavor.
            idle.as_mut().enable();
            if self.live.load(Ordering::SeqCst) == 0 {
                return;
            }
            idle.as_mut().await;
        }
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        // Aborting the driver drops its `JoinSet`, which aborts the
        // connections still in flight.
        self.driver.abort();
    }
}

/// Serve replies chosen by `route` on an ephemeral loopback port. The caller
/// owns the routing policy; this module owns the socket and the response
/// bytes. `None` is a `404`, the same as an unknown target before the fold-in.
pub(crate) async fn serve<F>(route: F) -> TestServer
where
    F: Fn(&str) -> Option<Reply> + Send + Sync + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let base = format!("http://{}", listener.local_addr().expect("addr"));
    let live = Arc::new(AtomicUsize::new(0));
    let idle = Arc::new(Notify::new());
    let route: Route = Arc::new(route);
    let spawned_live = Arc::clone(&live);
    let spawned_idle = Arc::clone(&idle);

    let driver = tokio::spawn(async move {
        let mut connections = JoinSet::new();
        loop {
            tokio::select! {
                accepted = listener.accept() => {
                    let Ok((socket, _)) = accepted else {
                        continue;
                    };
                    spawned_live.fetch_add(1, Ordering::SeqCst);
                    let route = Arc::clone(&route);
                    connections.spawn(handle_connection(socket, route));
                }
                // Reap a finished connection as soon as it completes, instead
                // of leaving its entry for the next accept.
                reaped = connections.join_next(), if !connections.is_empty() => {
                    if reaped.is_some() {
                        spawned_live.fetch_sub(1, Ordering::SeqCst);
                        if spawned_live.load(Ordering::SeqCst) == 0 {
                            spawned_idle.notify_waiters();
                        }
                    }
                }
            }
        }
    });

    TestServer {
        base,
        driver,
        live,
        idle,
    }
}

/// Answer one request on `socket`.
async fn handle_connection(mut socket: TcpStream, route: Route) {
    let mut buf = vec![0u8; 8192];
    let mut got = 0;
    while got < buf.len() {
        match socket.read(&mut buf[got..]).await {
            Ok(0) => break,
            Ok(n) => {
                got += n;
                if buf[..got].windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            Err(_) => return,
        }
    }
    let head = String::from_utf8_lossy(&buf[..got]).to_string();
    let target = head
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .unwrap_or("/");
    let (status, body, in_flight) = match route(target) {
        Some(Reply::Body(body)) => ("200 OK", body, None),
        Some(Reply::Status(status)) => (status, Vec::new(), None),
        Some(Reply::NeverAfterHit(hits)) => {
            hits.fetch_add(1, Ordering::SeqCst);
            std::future::pending::<()>().await;
            unreachable!("a never-answering route holds the socket open")
        }
        Some(Reply::StatusAfterHit(status, hits)) => {
            while hits.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
            (status, Vec::new(), None)
        }
        Some(Reply::GatedBody { body, gate }) => {
            let guard = gate.enter().await;
            ("200 OK", body, Some(guard))
        }
        None => ("404 Not Found", Vec::new(), None),
    };
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = socket.write_all(head.as_bytes()).await;
    let _ = socket.write_all(&body).await;
    let _ = socket.flush().await;
    // Dropping the guard releases the slot; an aborted connection never
    // reaches this point, but its guard still runs when the task is torn down.
    drop(in_flight);
}
