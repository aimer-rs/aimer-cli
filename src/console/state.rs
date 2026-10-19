use std::borrow::Cow;

use crate::commands::run::cargo_message::ErrorReport;
use crate::commands::run::panic_report::PanicReport;
use crate::commands::run::utilities::{AppOutput, StyledLog};
use crate::console::log_history::LogHistory;
use crate::resource_monitor::ResourceMonitorStatus;

/// Maximum number of log lines retained per pane before old lines are dropped.
pub const MAX_LINES: usize = 32768;

/// The runner lifecycle status shown in the status bar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Locking,
    Fetching(u8),
    Compiling(u8),
    Building(u8),
    Launching,
    Running,
    Idling,
    Error,
}

/// Events sent from runner/build threads to the console event loop.
pub enum RunnerEvent {
    BuildLog(String),
    /// The errors of a failed build, to be shown as one block at the end of the
    /// build output. Sent as a report rather than as ready-made lines so the
    /// pane can lay it out for its own width.
    BuildReport(ErrorReport),
    AppLog(StyledLog),
    /// A widget panic the running app recovered from, to be shown as one block.
    /// Sent as a report rather than as ready-made lines so the pane can lay it
    /// out for its own width, exactly like a [`BuildReport`](Self::BuildReport).
    AppPanic(PanicReport),
    StatusChange(Status),
    HotReload,
}

impl From<AppOutput> for RunnerEvent {
    /// The event one line of application output becomes.
    #[inline]
    fn from(output: AppOutput) -> Self {
        match output {
            AppOutput::Log(line) => Self::AppLog(line),
            AppOutput::Panic(report) => Self::AppPanic(report),
        }
    }
}

/// Which pane currently has focus in the console.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum ConsoleType {
    App,
    Build,
    Inspector,
    ResourceMonitor,
}

impl ConsoleType {
    /// Cycle to the next pane (App → Build → Inspector → Resources → App).
    pub fn next(&self) -> ConsoleType {
        match self {
            ConsoleType::App => ConsoleType::Build,
            ConsoleType::Build => ConsoleType::Inspector,
            ConsoleType::Inspector => ConsoleType::ResourceMonitor,
            ConsoleType::ResourceMonitor => ConsoleType::App,
        }
    }
}

/// Vertical scroll position for a single pane.
pub struct ScrollablePane {
    pub scroll: u16,
}

impl ScrollablePane {
    pub fn new() -> Self {
        Self { scroll: 0 }
    }

    pub fn scroll_up(&mut self, amount: u16) {
        self.scroll = self.scroll.saturating_add(amount);
    }

    pub fn scroll_down(&mut self, amount: u16) {
        self.scroll = self.scroll.saturating_sub(amount);
    }

    pub fn reset(&mut self) {
        self.scroll = 0;
    }
}

impl Default for ScrollablePane {
    fn default() -> Self {
        Self::new()
    }
}

/// Strip ANSI escape sequences and stray control characters from `s`,
/// preserving newlines and tabs.
pub fn strip_ansi(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut in_escape = false;
    for c in s.chars() {
        if in_escape {
            if c.is_ascii_alphabetic() || c == '@' || c == '~' {
                in_escape = false;
            }
        } else if c == '\x1B' {
            in_escape = true;
        } else if !c.is_control() || c == '\n' || c == '\t' {
            result.push(c);
        }
    }
    result
}

/// One entry of the Build Logs pane.
///
/// Most of the pane is build output kept verbatim. The `Compile Error` block of
/// a failed build is the exception: it is a framed, width-filling block, and a
/// block formatted for a width other than the pane's would be wrapped by the
/// renderer, which leaves its panel backgrounds torn across two rows. Keeping
/// the report itself means the pane can lay it out again whenever it is resized.
#[derive(Clone, Debug)]
pub enum BuildEntry {
    /// A line of build output, as the build wrote it.
    Line(String),
    /// The errors of a failed build, laid out on demand.
    Report(ErrorReport),
}

impl BuildEntry {
    /// The text of this entry in a pane `width` cells wide. Lines ignore the
    /// width; a report is laid out for it.
    pub fn render(&self, width: usize) -> Cow<'_, str> {
        match self {
            Self::Line(line) => Cow::Borrowed(line.as_str()),
            Self::Report(report) => Cow::Owned(report.lines_with_width(width).join("\n")),
        }
    }
}

/// One entry of the App Logs pane.
///
/// The pane is the application's own output, one line at a time. A widget panic
/// the framework recovered from is the exception: like the `Compile Error` block
/// of a failed build it is framed and fills the pane, so it is kept as a report
/// and laid out again whenever the pane is resized.
#[derive(Clone, Debug)]
pub enum AppEntry {
    /// A line of application output, already styled by the reader thread.
    Line(StyledLog),
    /// Consecutive identical application lines, kept as one expandable entry.
    RepeatedLine {
        line: StyledLog,
        count: usize,
        expanded: bool,
    },
    /// A recovered widget panic, laid out on demand.
    Panic(PanicReport),
}

impl AppEntry {
    /// The text of this entry in a pane `width` cells wide.
    ///
    /// A line ignores the width and only cares whether the source location of
    /// its log call is shown; a panic block is laid out for the width and is
    /// unaffected by the toggle — it states where it happened itself.
    pub fn render(&self, width: usize, show_source: bool) -> Cow<'_, str> {
        match self {
            Self::Line(line) => Cow::Owned(line.render(show_source)),
            Self::RepeatedLine {
                line,
                count,
                expanded,
            } => {
                let rendered = line.render(show_source);
                if *expanded {
                    let occurrences = (*count).min(MAX_LINES);
                    let capacity = rendered
                        .len()
                        .saturating_mul(occurrences)
                        .saturating_add(occurrences.saturating_sub(1));
                    let mut output = String::with_capacity(capacity);
                    for index in 0..occurrences {
                        if index > 0 {
                            output.push('\n');
                        }
                        output.push_str(&rendered);
                    }
                    Cow::Owned(output)
                } else {
                    Cow::Owned(format!("{rendered} [{count}]"))
                }
            }
            Self::Panic(report) => Cow::Owned(report.lines_with_width(width).join("\n")),
        }
    }
}

/// A single on-screen (already wrapped) row of a log pane, mapped back to the
/// source logical line it came from. Produced by the renderer so the mouse
/// handler can translate screen cells into text positions.
#[derive(Clone, Copy, Debug)]
pub struct VisualRow {
    /// Index of the source logical line in the pane's text.
    pub line: usize,
    /// Char offset within the source line where this visual row starts.
    pub start: usize,
    /// Number of chars displayed on this visual row.
    pub len: usize,
}

/// A character-range selection expressed as `(line, column)` anchor/cursor
/// positions into a pane's logical lines. Columns are char offsets. Modelled
/// after Vim's character-wise visual mode: both endpoints are inclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub anchor: (usize, usize),
    pub cursor: (usize, usize),
}

impl Selection {
    /// Return the `(lower, upper)` endpoints ordered so `lower <= upper`.
    pub fn ordered(&self) -> ((usize, usize), (usize, usize)) {
        if self.anchor <= self.cursor {
            (self.anchor, self.cursor)
        } else {
            (self.cursor, self.anchor)
        }
    }
}

/// Hit-test layout snapshot of the focused log pane, produced by the renderer
/// and consumed by the mouse event handler to map screen cells to text.
#[derive(Clone, Debug)]
pub struct PaneView {
    /// Inner (inside-border) origin and height of the pane, in screen cells.
    pub x: u16,
    pub y: u16,
    pub height: u16,
    /// The visual rows currently displayed, top to bottom.
    pub visible_rows: Vec<VisualRow>,
    /// Plain text of every logical line in selection mode. Empty in the normal
    /// view, which only needs the visible row mapping for mouse clicks.
    pub logical: Vec<String>,
}

/// Pure, testable view/event state for the console TUI. The I/O event loop in
/// `mod.rs` owns one of these and the renderer in `ui.rs` reads from it.
pub struct AppState {
    pub build_logs: LogHistory<BuildEntry>,
    pub app_logs: LogHistory<AppEntry>,
    /// Inner width of the Build Logs pane, in cells, as of the last frame. The
    /// `Compile Error` block is laid out for it; zero until the pane has been
    /// drawn once.
    build_width: usize,
    /// Inner width of the App Logs pane, in cells, as of the last frame. A
    /// recovered panic block is laid out for it; zero until the pane has been
    /// drawn once.
    app_width: usize,
    /// Whether app log lines show the source location of the log call, i.e. the
    /// `(file:line)` suffix of a structured record. Hidden by default to keep
    /// the pane narrow, and toggled with `e`; lines without a location — plain
    /// `println!` output — are unaffected.
    pub show_log_source: bool,
    /// Whether consecutive identical app logs should collapse into expandable
    /// groups. Enabled by the full-screen TUI and left off in inline mode.
    collapse_repeated_app_logs: bool,
    pub status: Status,
    pub pane: ConsoleType,
    /// Latest resource monitor sample from the live run session.
    pub resource_monitor: ResourceMonitorStatus,
    pub build_pane: ScrollablePane,
    pub app_pane: ScrollablePane,
    pub inspector_pane: ScrollablePane,
    pub inspector_full_tree: bool,
    pub inspector_cursor: usize,
    /// Whether the console is in Vim-style selection ("visual") mode. While
    /// `true`, dragging the mouse over the focused log pane highlights a
    /// character range that can be yanked to the clipboard. The scroll wheel
    /// keeps working and the mouse stays captured by the app in both modes.
    pub selection_mode: bool,
    /// The active character-range selection in the focused pane, if any.
    pub selection: Option<Selection>,
    /// True while the left mouse button is held down and dragging a selection.
    pub selecting: bool,
    /// Hit-test layout of the focused log pane from the most recent frame, used
    /// to translate mouse coordinates into `(line, column)` text positions.
    pub last_view: Option<PaneView>,
}

impl AppState {
    pub fn new() -> Self {
        Self::with_repeated_app_log_collapse(false)
    }

    /// Create state for the full-screen TUI, where identical consecutive app
    /// logs are collapsed and can be expanded with a mouse click.
    pub(crate) fn new_for_tui() -> Self {
        Self::with_repeated_app_log_collapse(true)
    }

    fn with_repeated_app_log_collapse(collapse_repeated_app_logs: bool) -> Self {
        Self {
            build_logs: LogHistory::new(MAX_LINES),
            app_logs: LogHistory::new(MAX_LINES),
            build_width: 0,
            app_width: 0,
            show_log_source: false,
            collapse_repeated_app_logs,
            status: Status::Compiling(0),
            pane: ConsoleType::App,
            resource_monitor: ResourceMonitorStatus::default(),
            build_pane: ScrollablePane::new(),
            app_pane: ScrollablePane::new(),
            inspector_pane: ScrollablePane::new(),
            inspector_full_tree: false,
            inspector_cursor: 0,
            selection_mode: false,
            selection: None,
            selecting: false,
            last_view: None,
        }
    }

    /// Append a build log line (carriage returns stripped), capping history.
    ///
    /// The line is parsed into its rendered rows here rather than at draw time,
    /// so a frame costs the same whatever the backlog holds.
    pub fn push_build_log(&mut self, msg: String) {
        let width = self.build_width;
        self.build_logs
            .push(BuildEntry::Line(msg.replace('\r', "")), move |entry| {
                entry.render(width)
            });
    }

    /// Append the `Compile Error` block of a failed build, laid out for the
    /// current pane width.
    pub fn push_build_report(&mut self, report: ErrorReport) {
        let width = self.build_width;
        self.build_logs
            .push(BuildEntry::Report(report), move |entry| entry.render(width));
    }

    /// Tell the pane how wide it is, re-laying its width-dependent entries out
    /// when that changed.
    ///
    /// Called by the renderer on every frame; the rebuild is skipped unless the
    /// width actually changed and the pane holds a block that depends on it, so
    /// an unresized console pays nothing for it.
    pub fn set_build_width(&mut self, width: usize) {
        if self.build_width == width {
            return;
        }
        self.build_width = width;
        let has_report = self
            .build_logs
            .entries()
            .iter()
            .any(|entry| matches!(entry, BuildEntry::Report(_)));
        if has_report {
            self.build_logs.rebuild(move |entry| entry.render(width));
        }
    }

    /// Append an app log line (carriage returns stripped), capping history.
    ///
    /// The line arrives already styled: the reader threads in
    /// [`cargo_build`](crate::commands::run::cargo_build) parse the JSON log
    /// records and colorize them, so the event loop does no formatting work.
    /// Its source location is kept apart from the message so [`show_log_source`]
    /// can hide it later.
    ///
    /// [`show_log_source`]: AppState::show_log_source
    pub fn push_app_log(&mut self, msg: StyledLog) {
        let line = msg.without_carriage_returns();
        if !self.collapse_repeated_app_logs {
            self.push_app_entry(AppEntry::Line(line));
            return;
        }

        let Some(index) = self.app_logs.entries().len().checked_sub(1) else {
            self.push_app_entry(AppEntry::Line(line));
            return;
        };
        let matches_last = match self.app_logs.entries().last() {
            Some(AppEntry::Line(previous))
            | Some(AppEntry::RepeatedLine { line: previous, .. }) => previous == &line,
            Some(AppEntry::Panic(_)) | None => false,
        };
        if !matches_last {
            self.push_app_entry(AppEntry::Line(line));
            return;
        }

        let width = self.app_width;
        let show_source = self.show_log_source;
        let expanded_below_limit = match self.app_logs.entries().last() {
            Some(AppEntry::RepeatedLine {
                count,
                expanded: true,
                ..
            }) => Some(*count < MAX_LINES),
            _ => None,
        };
        if let Some(append_visible_line) = expanded_below_limit {
            let increment = |entry: &mut AppEntry| {
                if let AppEntry::RepeatedLine { count, .. } = entry {
                    *count = count.saturating_add(1);
                }
            };
            if append_visible_line {
                let appended = line.render(show_source);
                self.app_logs
                    .append_to_last(increment, |_| Cow::Owned(appended));
            } else {
                self.app_logs.update_last(increment);
            }
        } else {
            self.app_logs.update_at(
                index,
                |entry| match entry {
                    AppEntry::Line(previous) => {
                        *entry = AppEntry::RepeatedLine {
                            line: previous.clone(),
                            count: 2,
                            expanded: false,
                        };
                    }
                    AppEntry::RepeatedLine { count, .. } => {
                        *count = count.saturating_add(1);
                    }
                    AppEntry::Panic(_) => {}
                },
                move |entry| entry.render(width, show_source),
            );
        }
    }

    /// Append the block of a widget panic the app recovered from, laid out for
    /// the current pane width.
    pub fn push_app_panic(&mut self, report: PanicReport) {
        self.push_app_entry(AppEntry::Panic(report));
    }

    /// Append `entry`, rendered as the pane currently shows it.
    fn push_app_entry(&mut self, entry: AppEntry) {
        let width = self.app_width;
        let show_source = self.show_log_source;
        self.app_logs
            .push(entry, move |entry| entry.render(width, show_source));
    }

    /// Expand or collapse the repeated app log group containing a rendered row.
    /// Returns `true` only when the row belongs to a repeated group.
    pub fn toggle_repeated_app_log_at_row(&mut self, row: usize) -> bool {
        let Some(index) = self.app_logs.entry_index_for_row(row) else {
            return false;
        };
        if !matches!(
            self.app_logs.entries().get(index),
            Some(AppEntry::RepeatedLine { .. })
        ) {
            return false;
        }

        let width = self.app_width;
        let show_source = self.show_log_source;
        self.app_logs.update_at(
            index,
            |entry| {
                if let AppEntry::RepeatedLine { expanded, .. } = entry {
                    *expanded = !*expanded;
                }
            },
            move |entry| entry.render(width, show_source),
        )
    }

    /// Tell the App Logs pane how wide it is, re-laying its width-dependent
    /// entries out when that changed.
    ///
    /// Called by the renderer on every frame; like [`set_build_width`] the
    /// rebuild is skipped unless the width actually changed and the pane holds a
    /// panic block, so a console without one pays nothing for it.
    ///
    /// [`set_build_width`]: AppState::set_build_width
    pub fn set_app_width(&mut self, width: usize) {
        if self.app_width == width {
            return;
        }
        self.app_width = width;
        let has_panic = self
            .app_logs
            .entries()
            .iter()
            .any(|entry| matches!(entry, AppEntry::Panic(_)));
        if has_panic {
            let show_source = self.show_log_source;
            self.app_logs
                .rebuild(move |entry| entry.render(width, show_source));
        }
    }

    /// Show or hide the source location of every app log line at once.
    ///
    /// The whole pane is re-rendered, which is why this is a user action rather
    /// than something the draw path decides.
    pub fn toggle_log_source(&mut self) {
        self.show_log_source = !self.show_log_source;
        let width = self.app_width;
        let show_source = self.show_log_source;
        self.app_logs
            .rebuild(move |entry| entry.render(width, show_source));
    }

    /// The app log lines as they must currently appear on screen.
    pub fn app_log_lines(&self) -> Vec<String> {
        self.app_logs
            .entries()
            .iter()
            .map(|entry| {
                entry
                    .render(self.app_width, self.show_log_source)
                    .into_owned()
            })
            .collect()
    }

    /// The whole build log pane as plain text, used when copying it.
    pub fn build_log_text(&self) -> String {
        self.build_logs
            .entries()
            .iter()
            .map(|entry| entry.render(self.build_width))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The whole app log pane as plain text, used when copying it.
    pub fn app_log_text(&self) -> String {
        self.app_log_lines().join("\n")
    }

    /// Apply a status change, focusing the most relevant pane.
    pub fn apply_status(&mut self, status: Status) {
        match status {
            Status::Error => self.pane = ConsoleType::Build,
            Status::Running => self.pane = ConsoleType::App,
            _ => {}
        }
        self.status = status;
    }

    /// Cycle focus to the next pane.
    pub fn next_pane(&mut self) {
        self.pane = self.pane.next();
    }

    /// Clear and reset the build pane.
    pub fn clear_build(&mut self) {
        self.build_logs.clear();
        self.build_pane.reset();
    }

    /// Clear and reset the app pane.
    pub fn clear_app(&mut self) {
        self.app_logs.clear();
        self.app_pane.reset();
    }

    /// Drop any active selection (and stop an in-progress drag).
    pub fn clear_selection(&mut self) {
        self.selection = None;
        self.selecting = false;
    }

    /// Extract the plain text covered by the current selection, if any, using
    /// the logical lines captured in the most recent [`PaneView`]. Endpoints
    /// are inclusive, matching Vim's character-wise visual mode.
    pub fn selected_text(&self) -> Option<String> {
        let sel = self.selection?;
        let logical = &self.last_view.as_ref()?.logical;
        if logical.is_empty() {
            return None;
        }
        let ((la, ca), (lb, cb)) = sel.ordered();
        let last_line = logical.len() - 1;
        let la = la.min(last_line);
        let lb = lb.min(last_line);

        let line_chars = |idx: usize| -> Vec<char> { logical[idx].chars().collect() };

        if la == lb {
            let chars = line_chars(la);
            if chars.is_empty() {
                return Some(String::new());
            }
            let start = ca.min(chars.len() - 1);
            let end = cb.min(chars.len() - 1);
            Some(chars[start..=end].iter().collect())
        } else {
            let mut out = String::new();
            // First line: from `ca` to its end.
            let first = line_chars(la);
            if !first.is_empty() {
                let start = ca.min(first.len() - 1);
                out.extend(&first[start..]);
            }
            out.push('\n');
            // Middle lines: in full.
            for line in logical.iter().take(lb).skip(la + 1) {
                out.push_str(line);
                out.push('\n');
            }
            // Last line: up to and including `cb`.
            let last = line_chars(lb);
            if !last.is_empty() {
                let end = cb.min(last.len() - 1);
                out.extend(&last[..=end]);
            }
            Some(out)
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
