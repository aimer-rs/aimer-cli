//! Smooth display of the build percentage.
//!
//! The runner reports progress in coarse jumps (a crate finishing can move the
//! percentage by tens of points at once). [`ProgressEase`] remembers the
//! percentage currently *shown* for the running phase and moves it toward the
//! reported one a point per tick, so the bar and the label climb instead
//! of leaping. The reported [`Status`] itself is never altered.

use std::mem::{Discriminant, discriminant};
use std::time::Duration;

use crate::console::state::Status;

/// The percentage carried by a build-phase status, if it has one.
pub fn percent_of(status: &Status) -> Option<u8> {
    match status {
        Status::Fetching(p)
        | Status::Downloading(p)
        | Status::Compiling(p)
        | Status::Building(p) => Some(*p),
        _ => None,
    }
}

/// `status` with its percentage replaced by `percent`; statuses without a
/// percentage are returned unchanged.
fn with_percent(status: &Status, percent: u8) -> Status {
    match status {
        Status::Fetching(_) => Status::Fetching(percent),
        Status::Downloading(_) => Status::Downloading(percent),
        Status::Compiling(_) => Status::Compiling(percent),
        Status::Building(_) => Status::Building(percent),
        other => other.clone(),
    }
}

/// How often the shown percentage advances by one point.
pub const STEP_INTERVAL: Duration = Duration::from_millis(40);

/// The next shown percentage when it trails `target`: exactly one point, so
/// every number in between is shown (25 -> 35 renders 26, 27, ... 35), and
/// never past the target.
fn next_step(shown: u8, target: u8) -> u8 {
    if shown < target { shown + 1 } else { target }
}

/// Tracks the percentage shown for the current build phase.
#[derive(Clone, Debug, Default)]
pub struct ProgressEase {
    shown: Option<(Discriminant<Status>, u8)>,
}

impl ProgressEase {
    /// The percentage to show for `target`.
    ///
    /// It snaps to the reported value when the phase changed since the last
    /// tick or when the report moved backwards (a restart), so the display
    /// never claims more than was reported.
    fn shown_for(&self, target: &Status) -> Option<u8> {
        let reported = percent_of(target)?;
        Some(match self.shown {
            Some((phase, shown)) if phase == discriminant(target) && shown <= reported => shown,
            _ => reported,
        })
    }

    /// `target` as it should be displayed right now.
    pub fn displayed(&self, target: &Status) -> Status {
        match self.shown_for(target) {
            Some(percent) => with_percent(target, percent),
            None => target.clone(),
        }
    }

    /// Advance the shown percentage one step toward `target`.
    pub fn tick(&mut self, target: &Status) {
        self.shown = match (self.shown_for(target), percent_of(target)) {
            (Some(shown), Some(reported)) => {
                Some((discriminant(target), next_step(shown, reported)))
            }
            _ => None,
        };
    }
}

#[cfg(test)]
mod tests;
