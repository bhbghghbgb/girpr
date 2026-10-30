//! Test-only helpers shared by the repair unit tests.
//!
//! A log line is part of the operator contract for the best-effort paths — a
//! refused audio move, a directory the sweep could not remove — so the tests
//! read it. `with_default` is thread-local, so a captured subscriber does not
//! race the other tests; nothing in the library installs a global subscriber
//! (only `main` does).

use std::io::Write;
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Run `f` with a debug-level subscriber whose lines are captured.
pub(crate) fn logs_of(f: impl FnOnce()) -> String {
    let buf = Captured::default();
    let writer = buf.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    tracing::subscriber::with_default(subscriber, f);
    String::from_utf8(buf.0.lock().unwrap().clone()).unwrap()
}
