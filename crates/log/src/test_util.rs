//! Helpers for this crate's own unit tests.
//!
//! Two shapes recur when testing the install mechanics: a cloneable byte sink
//! shared between a scoped subscriber and its assertion, and a layer that
//! records the targets of the events that reach it. Keeping one of each here
//! lets the install and filter tests focus on their own behaviour instead of
//! restating the locking and `Layer` plumbing.

use std::io::Write;
use std::sync::{Arc, Mutex};

use tracing::{Event, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;

/// A cloneable byte sink shared between a scoped subscriber and its test.
///
/// The clones all append to one buffer, which is what lets a test read back
/// what a writer installed on another handle rendered.
#[derive(Clone, Default)]
pub(crate) struct SharedBytes(Arc<Mutex<Vec<u8>>>);

impl SharedBytes {
    /// Every byte written so far, in order.
    pub(crate) fn snapshot(&self) -> Vec<u8> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

impl Write for SharedBytes {
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

impl<'a> MakeWriter<'a> for SharedBytes {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// A layer that records the targets of the events that reach it, as text.
#[derive(Clone, Default)]
pub(crate) struct Targets(Arc<Mutex<Vec<String>>>);

impl Targets {
    /// The targets seen so far, in event order.
    pub(crate) fn seen(&self) -> Vec<String> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

impl<S> Layer<S> for Targets
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(event.metadata().target().to_string());
    }
}
