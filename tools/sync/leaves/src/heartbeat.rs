//! When a long loop should emit its next progress event.
//!
//! The walk, phase A and phase C each count their own work and each render
//! their own event; the one thing they share is the question "is it time yet?".
//! That question has two clocks: the loop's count and the time since the last
//! event. This leaf owns the second clock and its reset, and asks an injected
//! predicate the question. It owns no message, no field and no log level: the
//! caller that knows what its event says also decides when to write it.

use std::time::{Duration, Instant};

/// The shared clock and reset behind a loop's progress events.
///
/// The trigger is a parameter rather than a rule because the three callers
/// deliberately disagree: the walk counts entries on a fixed period, phase A
/// counts settled files or a five-second gap, and phase C adds a final-count
/// trigger so the last file is reported too. What is identical is the clock
/// they read and the way it rearms after an event; that is what lives here, so
/// the three loops cannot drift on either.
pub struct Heartbeat<F> {
    start: Instant,
    last: Instant,
    trigger: F,
}

impl<F> Heartbeat<F>
where
    F: FnMut(usize, Duration) -> bool,
{
    /// Start a progress clock with the caller's trigger predicate.
    ///
    /// Both clocks start here, at construction, which is where the loop that
    /// owns this heartbeat starts its own work.
    pub fn new(trigger: F) -> Self {
        let now = Instant::now();
        Self {
            start: now,
            last: now,
            trigger,
        }
    }

    /// Ask the trigger whether `count` warrants a progress event.
    ///
    /// The duration handed to the trigger is the time since the previous
    /// *tick*, not since construction, so a time-budget trigger measures the
    /// gap between events. A tick rearms that clock before returning, so the
    /// next check sees a fresh gap.
    ///
    /// `Some` is the seconds since construction, for the event's `elapsed_s`
    /// field; `None` says this iteration emits nothing. The caller renders the
    /// event — with its own name, fields and level — only on `Some`.
    pub fn tick(&mut self, count: usize) -> Option<f64> {
        let since_last = self.last.elapsed();
        if (self.trigger)(count, since_last) {
            self.last = Instant::now();
            Some(self.start.elapsed().as_secs_f64())
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// The predicate is the whole cadence: a count-only trigger fires at
    /// exactly its multiples and nowhere else.
    #[test]
    fn the_injected_trigger_picks_the_ticks() {
        let mut hb = Heartbeat::new(|count, _| count % 3 == 0);
        let mut fired = Vec::new();
        for count in 1..=7 {
            if hb.tick(count).is_some() {
                fired.push(count);
            }
        }
        assert_eq!(fired, vec![3, 6]);
    }

    /// A final-count trigger is independent of the modulo: it fires at the
    /// target even when the target is not a multiple, and only once there.
    #[test]
    fn a_final_count_trigger_fires_at_the_target() {
        let mut hb = Heartbeat::new(|count, _| count.is_multiple_of(2) || count == 3);
        let mut fired = Vec::new();
        for count in 1..=5 {
            if hb.tick(count).is_some() {
                fired.push(count);
            }
        }
        assert_eq!(fired, vec![2, 3, 4]);
    }

    /// The duration the trigger sees is the gap since the last tick, and a
    /// tick rearms it: a budget that just fired does not fire again on the
    /// very next iteration because of the gap that preceded it.
    #[test]
    fn a_time_budget_measures_since_the_last_tick_and_rearms() {
        let gaps = RefCell::new(Vec::new());
        let mut hb = Heartbeat::new(|count, since_last| {
            gaps.borrow_mut().push(since_last);
            count == 1 || since_last >= Duration::from_millis(30)
        });
        assert!(hb.tick(1).is_some(), "the count trigger fires");
        assert!(
            hb.tick(2).is_none(),
            "no budget has elapsed since that tick"
        );
        std::thread::sleep(Duration::from_millis(40));
        assert!(hb.tick(3).is_some(), "the budget fires after the gap");
        assert!(hb.tick(4).is_none(), "the budget rearmed on the last tick");
        let gaps = gaps.borrow();
        assert!(
            gaps[1] < gaps[2],
            "the second gap is measured from the reset: {:?} vs {:?}",
            gaps[1],
            gaps[2]
        );
    }

    /// The value returned with a tick is elapsed since construction, not since
    /// the last tick, so an event's `elapsed_s` means the same thing whichever
    /// trigger fired it.
    #[test]
    fn a_tick_reports_elapsed_since_construction() {
        let mut hb = Heartbeat::new(|count, _| count == 1 || count == 2);
        std::thread::sleep(Duration::from_millis(20));
        let first = hb.tick(1).expect("the count trigger fires");
        let second = hb.tick(2).expect("the count trigger fires again");
        assert!(first > 0.0, "some time passed before the first tick");
        assert!(second >= first, "elapsed grows rather than restarting");
    }
}
