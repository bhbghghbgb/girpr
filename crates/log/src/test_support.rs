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
//! Each surface comes in a synchronous and an async form; the forms differ
//! only in how they scope the subscriber, and the async ones are gated by a
//! separate feature so a sync-only consumer never compiles tokio:
//!
//! - [`logs_of`](crate::test_support::logs_of) and [`logs_of_json`](crate::test_support::logs_of_json)
//!   use `with_default` for a synchronous closure and need nothing but the
//!   `test-support` feature;
//! - [`logs_of_async`](crate::test_support::logs_of_async) and
//!   [`logs_of_async_json`](crate::test_support::logs_of_async_json) use `set_default` across an
//!   `.await` and assert a current-thread runtime on every build — a multi-thread
//!   runtime would move spawned work onto threads with no subscriber and
//!   silently lose its events. They are behind `test-support-async`, which
//!   implies `test-support`.
//!
//! The text helpers keep their original subscriber and output for consumers
//! that assert on rendered lines. The JSON helpers install a JSON-formatted
//! subscriber and return one `serde_json::Value` per event, with the event's
//! fields (including `message`) flattened to the top level. Assertions can
//! then name the documented `level` and `message` fields directly instead of
//! re-implementing the text formatter's prefix. No shipped program installs
//! either subscriber.

// Test helpers assert their invariants with `expect`; the lint is for code a
// user runs.
#![allow(clippy::unwrap_used, clippy::expect_used)]

#[cfg(feature = "test-support-async")]
use std::future::Future;
use std::io::Write;
use std::sync::{Arc, Mutex};

/// A cloneable byte sink shared between a scoped subscriber and its test.
///
/// The storage and `Write` mechanics live here once so the programs that
/// capture logs do not each carry their own `Arc<Mutex<Vec<u8>>>`. How the
/// bytes become text, and where the subscriber is installed, stay local to
/// each consumer: this primitive only owns the bytes.
#[derive(Clone, Default)]
pub struct CaptureBuffer(Arc<Mutex<Vec<u8>>>);

impl Write for CaptureBuffer {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl CaptureBuffer {
    /// Every byte written so far, in order.
    pub fn snapshot(&self) -> Vec<u8> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

#[cfg(test)]
mod capture_buffer_tests {
    use super::*;

    /// Bytes written through `Write` come back from `snapshot()` unchanged,
    /// including bytes that are not valid UTF-8. The sink stores bytes, not
    /// text; turning them into text is the caller's job.
    #[test]
    fn write_and_snapshot_preserve_bytes_exactly() {
        let mut buffer = CaptureBuffer::default();

        let first = b"first line\n";
        assert_eq!(buffer.write(first).expect("write succeeds"), first.len());
        assert_eq!(buffer.snapshot(), first);

        let second = [0xff, 0x00, 0x80, b'\n'];
        buffer.write_all(&second).expect("write_all succeeds");
        let mut expected = first.to_vec();
        expected.extend_from_slice(&second);
        assert_eq!(buffer.snapshot(), expected);

        buffer.flush().expect("flush is a no-op");
        assert_eq!(buffer.snapshot(), expected);
    }

    /// Clones share one buffer: a writer clone sees bytes written through
    /// another handle, which is what lets a scoped subscriber and its test
    /// read the same capture.
    #[test]
    fn clones_share_the_same_bytes() {
        let mut writer = CaptureBuffer::default();
        let reader = writer.clone();
        writer.write_all(b"shared").expect("write_all succeeds");
        assert_eq!(reader.snapshot(), b"shared");
    }
}

/// The one shared subscriber shape: debug level, no ANSI, one buffer.
fn subscriber(buffer: CaptureBuffer) -> impl tracing::Subscriber {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .with_writer(move || buffer.clone())
        .finish()
}

/// The JSON counterpart of [`subscriber`]: one object per event, with the
/// event's fields flattened to the top level so `message` is a documented
/// sibling of `level` rather than buried in a rendered line.
fn json_subscriber(buffer: CaptureBuffer) -> impl tracing::Subscriber {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .json()
        .flatten_event(true)
        .with_writer(move || buffer.clone())
        .finish()
}

/// Split the JSON subscriber's NDJSON output into one `Value` per event.
///
/// The JSON formatter escapes newlines inside strings, so one event is always
/// one line and a multi-line message stays a single record. Blank lines are
/// ignored; anything else must be the object the formatter promised to write.
fn json_records(bytes: &[u8]) -> Vec<serde_json::Value> {
    std::str::from_utf8(bytes)
        .expect("the JSON subscriber writes UTF-8")
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str(line).expect("the JSON subscriber writes one object per line")
        })
        .collect()
}

/// Run `f` with a JSON-formatted subscriber and return its events as records.
///
/// Each record carries the documented `level` and `message` fields (plus the
/// event's own fields and the formatter's `timestamp`/`target`). This is the
/// surface for assertions that would otherwise parse rendered log lines.
pub fn logs_of_json(f: impl FnOnce()) -> Vec<serde_json::Value> {
    let buffer = CaptureBuffer::default();
    tracing::subscriber::with_default(json_subscriber(buffer.clone()), f);
    json_records(&buffer.snapshot())
}

/// Run `f` with a debug-level subscriber whose lines are captured and return
/// the rendered lines as text.
///
/// `with_default` scopes the subscriber to this thread for the duration of
/// `f`; nothing in a shipped program installs a global subscriber.
pub fn logs_of(f: impl FnOnce()) -> String {
    let buffer = CaptureBuffer::default();
    tracing::subscriber::with_default(subscriber(buffer.clone()), f);
    String::from_utf8(buffer.snapshot()).expect("tracing writes UTF-8")
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
/// the returned text. The helper asserts the runtime flavor on every build so
/// that misuse fails instead of returning partial logs.
///
/// Blocking helpers that install the same dispatcher themselves (the
/// product's `spawn_blocking_traced`) are captured too — provided the spawn
/// site already carries the subscriber; the dispatcher is taken at *spawn
/// time*, so one spawned from another spawned task captures the worker's.
#[cfg(feature = "test-support-async")]
fn assert_current_thread_runtime() {
    assert_eq!(
        tokio::runtime::Handle::current().runtime_flavor(),
        tokio::runtime::RuntimeFlavor::CurrentThread,
        "logs_of requires a current-thread runtime; use a plain #[tokio::test]",
    );
}

/// Keep two level-enabling dispatchers registered for the whole test binary.
///
/// `tracing` caches callsite interest process-globally. If a callsite is first
/// emitted by a sibling test running under `NoSubscriber`, its interest is
/// cached as `never()` and a scoped subscriber installed later receives nothing.
/// A rebuild at subscriber-install time heals callsites already registered, but
/// a sibling can register another callsite after it and poison that one again.
/// With more than one live registered dispatcher, callsite registration consults
/// the registry instead of the emitting thread's default, so the levels these
/// keepers enable stay enabled for callsites registered during concurrent tests.
/// They are never installed as defaults, so they capture no events.
///
/// This is a known upstream `tracing` bug: [tokio-rs/tracing#3611](https://github.com/tokio-rs/tracing/issues/3611),
/// [tokio-rs/tracing#2874](https://github.com/tokio-rs/tracing/issues/2874) (an
/// older report of the same mechanism), and
/// [tokio-rs/tracing#2400](https://github.com/tokio-rs/tracing/issues/2400)
/// (registering from inside `dispatcher::get_default`). Candidate fixes are open
/// PRs [tokio-rs/tracing#3614](https://github.com/tokio-rs/tracing/pull/3614) and
/// [tokio-rs/tracing#3615](https://github.com/tokio-rs/tracing/pull/3615).
/// The locked `tracing-core` is 0.1.36 and upstream `main` is still affected, so
/// this workaround (the rebuild plus two registered no-op dispatchers) can be
/// removed only once a fixed `tracing-core` is released and upgraded here.
#[cfg(feature = "test-support-async")]
fn keep_callsites_interested() {
    static KEEPERS: std::sync::OnceLock<(
        tracing::dispatcher::Dispatch,
        tracing::dispatcher::Dispatch,
    )> = std::sync::OnceLock::new();
    KEEPERS.get_or_init(|| {
        (
            tracing::dispatcher::Dispatch::new(subscriber(CaptureBuffer::default())),
            tracing::dispatcher::Dispatch::new(subscriber(CaptureBuffer::default())),
        )
    });
}

#[cfg(feature = "test-support-async")]
pub async fn logs_of_async<F: Future>(f: F) -> (F::Output, String) {
    assert_current_thread_runtime();

    let buffer = CaptureBuffer::default();
    let guard = tracing::subscriber::set_default(subscriber(buffer.clone()));
    keep_callsites_interested();
    // Recompute cached interest while this subscriber is the thread default,
    // healing callsites a sibling registered under `NoSubscriber` before here.
    tracing::callsite::rebuild_interest_cache();
    let out = f.await;
    drop(guard);
    (
        out,
        String::from_utf8(buffer.snapshot()).expect("tracing writes UTF-8"),
    )
}

/// The async counterpart of [`logs_of_json`], with the same runtime
/// precondition as [`logs_of_async`].
#[cfg(feature = "test-support-async")]
pub async fn logs_of_async_json<F: Future>(f: F) -> (F::Output, Vec<serde_json::Value>) {
    assert_current_thread_runtime();

    let buffer = CaptureBuffer::default();
    let guard = tracing::subscriber::set_default(json_subscriber(buffer.clone()));
    let out = f.await;
    drop(guard);
    (out, json_records(&buffer.snapshot()))
}

#[cfg(all(test, feature = "test-support-async"))]
mod tests {
    use super::*;

    /// The precondition must also hold in a release build: a multi-thread
    /// runtime runs spawned work on threads with no subscriber, so returning
    /// those partial logs would read as coverage the test never had. The guard
    /// is always on, so the misuse fails loudly instead.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[should_panic(expected = "logs_of requires a current-thread runtime")]
    async fn a_multi_thread_runtime_is_rejected() {
        let _ = logs_of_async(async {}).await;
    }

    /// The JSON surface shares the precondition: a multi-thread runtime would
    /// leave spawned events out of the capture.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[should_panic(expected = "logs_of requires a current-thread runtime")]
    async fn a_multi_thread_runtime_is_rejected_by_json_capture() {
        let _ = logs_of_async_json(async {}).await;
    }
}

#[cfg(test)]
mod json_capture_tests {
    use super::*;

    /// The documented `level` and `message` fields come back by name, alongside
    /// the event's own fields, without any text parsing by the caller.
    #[test]
    fn json_capture_exposes_level_message_and_event_fields() {
        let records = logs_of_json(|| {
            tracing::warn!(file = "a.dat", "CHECK fail: md5");
            tracing::info!("check-only: total=1 bad=0 damaged_chunks=0");
        });

        assert_eq!(records.len(), 2, "{records:?}");
        assert_eq!(records[0]["level"], "WARN");
        assert_eq!(records[0]["message"], "CHECK fail: md5");
        assert_eq!(records[0]["file"], "a.dat");
        assert_eq!(records[1]["level"], "INFO");
        assert_eq!(
            records[1]["message"],
            "check-only: total=1 bad=0 damaged_chunks=0"
        );
    }

    /// A message that begins with a number, or that looks like a whole rendered
    /// line, cannot displace the level: the level is read from its own field.
    #[test]
    fn a_numeric_or_prefixed_message_does_not_displace_the_level() {
        let records = logs_of_json(|| {
            tracing::info!("2 INFO girpr::check: CHECK fail: md5 file=a.dat");
        });

        assert_eq!(records.len(), 1, "{records:?}");
        assert_eq!(records[0]["level"], "INFO");
        assert_eq!(
            records[0]["message"],
            "2 INFO girpr::check: CHECK fail: md5 file=a.dat"
        );
    }

    /// A newline in the message stays inside one record's `message` field; the
    /// JSON formatter escapes it, so it is not read as a second event.
    #[test]
    fn a_multiline_message_is_one_record() {
        let records = logs_of_json(|| tracing::warn!("first line\nsecond line"));

        assert_eq!(records.len(), 1, "{records:?}");
        assert_eq!(records[0]["message"], "first line\nsecond line");
    }
}
