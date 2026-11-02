//! Phase timing for the log.
//!
//! Every phase reports one `duration_ms` field on its completion line, so an
//! operator can see where a run spent its time. Durations come from a monotonic
//! clock and are whole milliseconds; a phase that never started never logs one.

use std::time::Instant;

/// Whole milliseconds between two monotonic instants.
///
/// `saturating_duration_since` keeps a clock that is not perfectly monotonic
/// from wrapping into an enormous duration.
pub(crate) fn millis_between(start: Instant, end: Instant) -> u64 {
    end.saturating_duration_since(start).as_millis() as u64
}

/// The start of one phase, so its completion line can carry `duration_ms`.
///
/// The timer logs on drop, which covers every early return and error path. A
/// phase whose existing completion line already carries the duration calls
/// [`disarm`](Self::disarm) instead, so the value is not logged twice.
pub(crate) struct PhaseTimer {
    phase: &'static str,
    start: Instant,
    logged: bool,
}

impl PhaseTimer {
    /// Start timing `phase`.
    pub(crate) fn start(phase: &'static str) -> Self {
        Self {
            phase,
            start: Instant::now(),
            logged: false,
        }
    }

    /// Whole milliseconds since this phase started.
    pub(crate) fn elapsed_ms(&self) -> u64 {
        millis_between(self.start, Instant::now())
    }

    /// Emit this phase's own completion line and stop the timer.
    pub(crate) fn finish(mut self) {
        self.log();
    }

    /// Keep the duration out of a separate line: the phase's existing
    /// completion line already carries [`elapsed_ms`](Self::elapsed_ms).
    pub(crate) fn disarm(mut self) {
        self.logged = true;
    }

    fn log(&mut self) {
        if self.logged {
            return;
        }
        self.logged = true;
        tracing::info!(
            phase = % self.phase,
            duration_ms = self.elapsed_ms(),
            "phase ended"
        );
    }
}

impl Drop for PhaseTimer {
    fn drop(&mut self) {
        self.log();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// The conversion is whole milliseconds from an injected pair of instants,
    /// so the test never depends on how long the test itself took.
    #[test]
    fn elapsed_milliseconds_are_whole_and_saturating() {
        let start = Instant::now();
        assert_eq!(millis_between(start, start), 0);
        assert_eq!(millis_between(start, start + Duration::from_millis(42)), 42);
        assert_eq!(
            millis_between(start, start + Duration::from_micros(1_999)),
            1,
            "the sub-millisecond part is truncated, not rounded"
        );
        assert_eq!(
            millis_between(start, start.checked_sub(Duration::from_millis(1)).unwrap()),
            0,
            "a backwards reading must not wrap"
        );
    }

    /// The stable field name is what a reader greps; the value is not asserted.
    #[test]
    fn the_timer_logs_phase_and_duration() {
        let logs = crate::repair::test_support::logs_of(|| {
            PhaseTimer::start("unit").finish();
        });
        assert!(logs.contains("phase=unit"), "{logs}");
        assert!(logs.contains("duration_ms="), "{logs}");
    }
}
