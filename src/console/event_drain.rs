//! Time-boxed draining of the runner event channel.
//!
//! A compile can emit thousands of log lines in one burst, and every line is
//! parsed for ANSI colors as it is applied. Applying the whole backlog before
//! the next draw freezes the screen and the keyboard until it is done, so each
//! frame only spends a fixed budget on events and leaves the rest for the next
//! frame. Every line applied inside the budget is drawn right after it, so a
//! burst appears progressively as it is parsed instead of all at once when the
//! last line is done.

use std::time::{Duration, Instant};

use crossbeam::channel::Receiver;

use super::state::RunnerEvent;

/// The longest a single frame spends applying queued runner events.
///
/// Kept under a 60 fps frame (about 16 ms) so the redraw that shows the lines
/// just parsed is never held back for longer than one frame.
pub(crate) const EVENT_BUDGET: Duration = Duration::from_millis(12);

/// Apply queued events until the channel is empty or `budget` is spent.
///
/// Returns `true` when events are still queued, so the caller can skip waiting
/// for input and come straight back for the rest after drawing.
pub(crate) fn drain_events(
    rx: &Receiver<RunnerEvent>,
    budget: Duration,
    mut apply: impl FnMut(RunnerEvent),
) -> bool {
    let started = Instant::now();
    while let Ok(event) = rx.try_recv() {
        apply(event);
        if started.elapsed() >= budget {
            return !rx.is_empty();
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn log(n: usize) -> RunnerEvent {
        RunnerEvent::BuildLog(format!("line {n}"))
    }

    #[test]
    fn drains_everything_within_the_budget() {
        let (tx, rx) = crossbeam::channel::unbounded();
        for n in 0..10 {
            tx.send(log(n)).unwrap();
        }
        let mut applied = 0;
        let backlog = drain_events(&rx, Duration::from_secs(5), |_| applied += 1);
        assert_eq!(applied, 10);
        assert!(!backlog);
    }

    #[test]
    fn stops_when_the_budget_is_spent_and_reports_the_backlog() {
        let (tx, rx) = crossbeam::channel::unbounded();
        for n in 0..10 {
            tx.send(log(n)).unwrap();
        }
        let mut applied = 0;
        let backlog = drain_events(&rx, Duration::ZERO, |_| applied += 1);
        assert_eq!(applied, 1);
        assert!(backlog);
        assert_eq!(rx.len(), 9);
    }

    #[test]
    fn an_empty_channel_has_no_backlog() {
        let (_tx, rx) = crossbeam::channel::unbounded::<RunnerEvent>();
        assert!(!drain_events(&rx, EVENT_BUDGET, |_| {}));
    }
}
