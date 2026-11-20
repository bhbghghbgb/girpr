//! Test-only log capture shared by the workspace's test binaries.
//!
//! Asserting on a log line means installing a subscriber that writes into a
//! buffer the test can read back. `tracing`'s scoped/thread-local subscribers
//! (`with_default` and `set_default`) are what keep that buffer from racing a
//! global subscriber, and the mechanics are the same in every consumer: a
//! debug-level `fmt` subscriber whose writer appends to one
//! `Arc<Mutex<Vec<u8>>>`. They live here, once, in a library crate.
//!
//! Why a public module in a library crate and not a `tests/common` module:
//! `cargo nextest` runs one process per test and does not build doctests, so a
//! helper can only reach the test binaries through a public library surface.
//! Each consumer keeps a thin `logs_of` shim (a re-export or a one-line
//! delegate) so every call site is unchanged.
//!
//! The two entry points differ only in how they scope the subscriber:
//!
//! - [`logs_of`] uses `with_default` for a synchronous closure;
//! - [`logs_of_async`] uses `set_default` across an `.await` and, on a debug
//!   build, asserts a current-thread runtime — a multi-thread runtime would
//!   move spawned work onto threads with no subscriber and silently lose its
//!   lines.

// Test helpers assert their invariants with `expect`; the lint is for code a
// user runs.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::future::Future;
use std::io::Write;
use std::sync::{Arc, Mutex};

/// The capture buffer: cloneable so the subscriber can outlive the caller.
#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .expect("captured log lock")
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Captured {
    /// Everything the subscriber wrote, as UTF-8 text.
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().expect("captured log lock").clone())
            .expect("tracing writes UTF-8")
    }
}

/// The one shared subscriber shape: debug level, no ANSI, one buffer.
fn subscriber(buffer: Captured) -> impl tracing::Subscriber {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .with_writer(move || buffer.clone())
        .finish()
}

/// Run `f` with a debug-level subscriber whose lines are captured and return
/// the rendered lines as text.
///
/// `with_default` scopes the subscriber to this thread for the duration of
/// `f`; nothing in a shipped program installs a global subscriber.
pub fn logs_of(f: impl FnOnce()) -> String {
    let buffer = Captured::default();
    tracing::subscriber::with_default(subscriber(buffer.clone()), f);
    buffer.text()
}

/// Run `f` under a debug-level scoped subscriber and return its output with
/// the lines it emitted.
///
/// **Precondition: the caller must run on a current-thread runtime**, which is
/// what a plain `#[tokio::test]` provides. `set_default` installs the
/// subscriber on the calling thread only, so `f` is polled on the thread that
/// installed it. A multi-thread runtime does not move `f` itself: `block_on`
/// keeps polling the awaited future on the calling thread, so the lines `f`
/// emits are captured there too. What a multi-thread runtime does move is the
/// work `f` spawns: a task started with `tokio::spawn` runs on a worker thread
/// that carries no subscriber, and that task's lines are silently missing from
/// the returned text. A debug build asserts the runtime flavor so that misuse
/// fails instead of returning partial logs.
///
/// Blocking helpers that install the same dispatcher themselves (the
/// product's `spawn_blocking_traced`) are captured too — provided the spawn
/// site already carries the subscriber; the dispatcher is taken at *spawn
/// time*, so one spawned from another spawned task captures the worker's.
pub async fn logs_of_async<F: Future>(f: F) -> (F::Output, String) {
    debug_assert_eq!(
        tokio::runtime::Handle::current().runtime_flavor(),
        tokio::runtime::RuntimeFlavor::CurrentThread,
        "logs_of requires a current-thread runtime; use a plain #[tokio::test]",
    );

    let buffer = Captured::default();
    let guard = tracing::subscriber::set_default(subscriber(buffer.clone()));
    let out = f.await;
    drop(guard);
    (out, buffer.text())
}
