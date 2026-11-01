//! Terminal window title reporting.
//!
//! Mirrors the runner [`Status`] into the terminal title (for example
//! `App Name > Compile 42%`) so build progress stays visible from a background
//! tab or the window switcher.

use std::io::{IsTerminal, Write, stdout};

use super::state::Status;

/// Push the current title onto the terminal's title stack (xterm `CSI 22;2 t`).
const PUSH_TITLE: &str = "\x1b[22;2t";
/// Restore the title saved by [`PUSH_TITLE`] (xterm `CSI 23;2 t`).
const POP_TITLE: &str = "\x1b[23;2t";

/// Human-readable phase text for a runner status.
pub(crate) fn phase_for(status: &Status) -> String {
    match status {
        Status::Locking => "Locking dependencies".to_string(),
        Status::Fetching(p) => format!("Fetch {p}%"),
        Status::Downloading(p) => format!("Download {p}%"),
        Status::Compiling(p) => format!("Compile {p}%"),
        Status::Building(p) => format!("Build {p}%"),
        Status::Launching => "Launching".to_string(),
        Status::Running => "Running".to_string(),
        Status::Idling => "Idle".to_string(),
        Status::Error => "Build failed".to_string(),
    }
}

/// Full window title: `App Name > Phase`.
pub(crate) fn title_for(app_name: &str, status: &Status) -> String {
    format!("{app_name}: {}", phase_for(status))
}

/// The `OSC 2` escape sequence that sets the window title to `title`.
pub(crate) fn set_title_sequence(title: &str) -> String {
    format!("\x1b]2;{title}\x1b\\")
}

/// Keeps the terminal title in sync with the runner status and restores the
/// original title when dropped.
pub struct TitleReporter {
    app_name: String,
    enabled: bool,
    last: Option<String>,
}

impl TitleReporter {
    /// Create a reporter, saving the current title. Does nothing unless stdout
    /// is a terminal, so the escapes never leak into pipes or CI logs.
    pub fn new(app_name: &str) -> Self {
        let enabled = stdout().is_terminal()
            && std::env::var("TERM").map_or(true, |term| term != "dumb");
        if enabled {
            write_escape(PUSH_TITLE);
        }
        Self {
            app_name: app_name.to_string(),
            enabled,
            last: None,
        }
    }

    /// Update the title for `status`, writing only when the text changed.
    pub fn update(&mut self, status: &Status) {
        if !self.enabled {
            return;
        }
        let title = title_for(&self.app_name, status);
        if self.last.as_deref() == Some(title.as_str()) {
            return;
        }
        write_escape(&set_title_sequence(&title));
        self.last = Some(title);
    }
}

impl Drop for TitleReporter {
    fn drop(&mut self) {
        if self.enabled {
            write_escape(POP_TITLE);
        }
    }
}

fn write_escape(sequence: &str) {
    let mut out = stdout();
    let _ = out.write_all(sequence.as_bytes());
    let _ = out.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiling_title_includes_app_name_and_percentage() {
        assert_eq!(
            title_for("My App", &Status::Compiling(42)),
            "My App: Compile 42%"
        );
        assert_eq!(
            title_for("My App", &Status::Compiling(100)),
            "My App: Compile 100%"
        );
    }

    #[test]
    fn other_phases_have_distinct_titles() {
        assert_eq!(phase_for(&Status::Fetching(5)), "Fetch 5%");
        assert_eq!(phase_for(&Status::Downloading(7)), "Download 7%");
        assert_eq!(phase_for(&Status::Building(80)), "Build 80%");
        assert_eq!(phase_for(&Status::Locking), "Locking dependencies");
        assert_eq!(phase_for(&Status::Launching), "Launching");
        assert_eq!(phase_for(&Status::Running), "Running");
        assert_eq!(phase_for(&Status::Idling), "Idle");
        assert_eq!(phase_for(&Status::Error), "Build failed");
    }

    #[test]
    fn title_sequence_is_a_terminated_osc_2() {
        assert_eq!(set_title_sequence("Compiling 1%"), "\x1b]2;Compiling 1%\x1b\\");
    }

    #[test]
    fn disabled_reporter_never_records_a_title() {
        let mut reporter = TitleReporter {
            app_name: "My App".to_string(),
            enabled: false,
            last: None,
        };
        reporter.update(&Status::Compiling(10));
        assert!(reporter.last.is_none());
    }
}
