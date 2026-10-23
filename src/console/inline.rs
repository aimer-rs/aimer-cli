//! Inline terminal presentation, the default console of `aimer run`.
//!
//! The runner still emits the normal [`RunnerEvent`] stream. This module only
//! changes how that stream is presented: completed stages become scrollback,
//! while the active stage and control navbar occupy a small managed region at
//! the bottom of the terminal.

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, IsTerminal, Write, stdout};
use std::net::{IpAddr, Ipv4Addr};
use std::time::{Duration, Instant, SystemTime};

use anyhow::Context;
use arboard::Clipboard;
use colored::Colorize;
use crossterm::event::Event;
use crossterm::terminal;
use tokio::runtime::Runtime;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::commands::run::Device;
use crate::console::hotkeys::{ConsoleAction, ConsolePane, map_key_event};
use crate::console::input;
use crate::console::inspector::InspectorServer;
use crate::console::stage::{StageBook, StageId, StageKind, StageProgress, StageStatus};
use crate::console::state::{AppState, RunnerEvent, Status, strip_ansi};
use crate::session::{LogStream, SessionCommand, SessionHandle};
use crate::targets::Targets;
use crate::tui::RawModeGuard;

const SPINNERS: [&str; 10] = ["⠋", "⠙", "⠚", "⠞", "⠖", "⠦", "⠴", "⠲", "⠳", "⠓"];
const BUILD_LOCK_WAIT_PREFIX: &str = "Blocking waiting for file lock";
const NOTICE_VISIBLE_FOR: Duration = Duration::from_secs(4);

/// State shared by the inline renderer and its input handler.
pub struct InlineSession {
    /// Target label shown in the session header.
    pub target: String,
    /// Existing application/build log state and formatting behaviour.
    pub state: AppState,
    /// Retained stage summaries and expansion state.
    pub stages: StageBook,
    /// Current live-status spinner frame.
    pub frame: usize,
    hot_reload_enabled: bool,
    notice: Option<(String, Instant)>,
    // The cargo message and when the wait began.
    build_lock_wait: Option<(String, Instant)>,
}

impl InlineSession {
    /// Create an empty inline session for `device`.
    pub fn new(device: &Device) -> Self {
        Self::new_with_hot_reload(device, false)
    }

    /// Create an inline session with explicit hot-reload capability.
    pub fn new_with_hot_reload(device: &Device, hot_reload_enabled: bool) -> Self {
        Self {
            target: device.to_string(),
            state: AppState::new(),
            stages: StageBook::new(),
            frame: 0,
            hot_reload_enabled,
            notice: None,
            build_lock_wait: None,
        }
    }

    /// Consume one existing runner event without performing terminal I/O.
    pub fn apply_event(&mut self, event: RunnerEvent) {
        match event {
            RunnerEvent::BuildLog(message) => self.push_build_detail(message),
            RunnerEvent::BuildReport(report) => {
                let lines = report.lines();
                self.state.push_build_report(report);
                let stage = self.ensure_detail_stage(StageKind::Compile);
                for line in lines {
                    let _ = self.stages.append_detail(stage, line);
                }
            }
            RunnerEvent::AppLog(log) => {
                let rendered = log.render(self.state.show_log_source);
                self.state.push_app_log(log);
                let stage = self.ensure_detail_stage(StageKind::Application);
                let _ = self.stages.append_detail(stage, rendered);
            }
            RunnerEvent::AppPanic(report) => {
                let lines = report.lines();
                self.state.push_app_panic(report);
                let stage = self.ensure_detail_stage(StageKind::Application);
                for line in lines {
                    let _ = self.stages.append_detail(stage, line);
                }
            }
            RunnerEvent::StatusChange(status) => self.apply_status(status),
            RunnerEvent::HotReload if self.hot_reload_enabled => {
                self.notice("File change detected; rebuilding...".to_string());
                let stage = self.ensure_detail_stage(StageKind::HotReload);
                let _ = self.stages.append_detail(stage, "File change detected");
            }
            RunnerEvent::HotReload => {}
        }
    }

    /// Apply a lifecycle status and update the corresponding stage.
    pub fn apply_status(&mut self, status: Status) {
        self.build_lock_wait = None;
        self.state.apply_status(status.clone());
        let now = Instant::now();

        if status == Status::Error {
            if let Some(id) = self.stages.active() {
                let _ = self.stages.fail(id, now);
            } else {
                let id = self.stages.start(StageKind::Compile, now);
                let _ = self.stages.fail(id, now);
            }
            return;
        }

        if status == Status::Idling {
            if let Some(id) = self.stages.active()
                && self
                    .stages
                    .stage(id)
                    .is_some_and(|stage| stage.kind() == &StageKind::Application)
            {
                let _ = self.stages.finish(id, now);
            }
            return;
        }

        let kind = match status {
            Status::Locking => StageKind::Locking,
            Status::Fetching(_) => StageKind::Fetching,
            Status::Downloading(_) => StageKind::Downloading,
            Status::Compiling(_) => StageKind::Compile,
            Status::Building(_) => StageKind::Assemble,
            Status::Launching => StageKind::Launch,
            Status::Running => StageKind::Application,
            Status::Error | Status::Idling => return,
        };
        let id = self.ensure_stage(kind);
        let progress = match status {
            Status::Fetching(percent)
            | Status::Downloading(percent)
            | Status::Compiling(percent)
            | Status::Building(percent) => Some(StageProgress::new(percent as u64, Some(100))),
            Status::Locking | Status::Launching | Status::Running => None,
            Status::Error | Status::Idling => None,
        };
        let _ = self.stages.set_progress(id, progress);
    }

    /// Reset retained output before a deliberate hot restart.
    pub fn reset_for_restart(&mut self) {
        self.state.clear_build();
        self.state.clear_app();
        self.state.status = Status::Compiling(0);
        self.stages = StageBook::new();
        self.notice("Hot restart requested".to_string());
        self.build_lock_wait = None;
    }

    /// Show a one-line notice in the live region for a few seconds.
    pub fn notice(&mut self, notice: String) {
        self.notice = Some((notice, Instant::now()));
    }

    fn active_notice(&self, now: Instant) -> Option<&str> {
        self.notice
            .as_ref()
            .filter(|(_, shown_at)| now.saturating_duration_since(*shown_at) < NOTICE_VISIBLE_FOR)
            .map(|(notice, _)| notice.as_str())
    }

    fn push_build_detail(&mut self, message: String) {
        if message.trim_start().starts_with(BUILD_LOCK_WAIT_PREFIX) {
            // Cargo repeats the notice; the wait began at the first one.
            let since = self.build_lock_wait.as_ref().map_or_else(Instant::now, |(_, since)| *since);
            self.build_lock_wait = Some((message.clone(), since));
        }
        self.state.push_build_log(message.clone());
        let stage = self.ensure_detail_stage(self.build_detail_kind());
        let _ = self.stages.append_detail(stage, message);
    }

    /// The stage a build log line belongs to. While cargo is locking, fetching
    /// or downloading, its output stays with that phase instead of opening a
    /// compile stage that would end the phase.
    fn build_detail_kind(&self) -> StageKind {
        self.stages
            .active()
            .and_then(|id| self.stages.stage(id))
            .filter(|stage| stage.status() == StageStatus::Running)
            .map(|stage| stage.kind().clone())
            .filter(StageKind::is_preparation)
            .unwrap_or_else(|| self.build_kind())
    }

    fn build_kind(&self) -> StageKind {
        match self.state.status {
            Status::Building(_) => StageKind::Assemble,
            Status::Launching => StageKind::Launch,
            _ => StageKind::Compile,
        }
    }

    fn ensure_detail_stage(&mut self, kind: StageKind) -> StageId {
        if let Some(id) = self.stages.active()
            && self.stages.stage(id).is_some_and(|stage| {
                stage.status() == StageStatus::Running && stage.kind() == &kind
            })
        {
            return id;
        }
        self.ensure_stage(kind)
    }

    fn is_unstarted_compile(&self, id: StageId) -> bool {
        self.stages.stage(id).is_some_and(|stage| {
            stage.status() == StageStatus::Running
                && stage.kind() == &StageKind::Compile
                && stage
                    .progress()
                    .and_then(StageProgress::percentage)
                    .unwrap_or(0)
                    == 0
        })
    }

    fn ensure_stage(&mut self, kind: StageKind) -> StageId {
        if let Some(id) = self.stages.active() {
            let same_kind = self.stages.stage(id).is_some_and(|stage| {
                stage.status() == StageStatus::Running && stage.kind() == &kind
            });
            if same_kind {
                return id;
            }
            // The build opens a compile stage before cargo says what it is doing.
            // While nothing has compiled it becomes the first phase, so the
            // transcript does not start with an empty compile entry.
            if kind.is_preparation()
                && self.is_unstarted_compile(id)
                && self.stages.retag(id, kind.clone()).is_ok()
            {
                return id;
            }
            let _ = self.stages.finish(id, Instant::now());
        }
        self.stages.start(kind, Instant::now())
    }
}

/// An inline renderer that updates only the most recent managed output region.
pub struct InlineRenderer<W> {
    writer: W,
    // Each frame is composed here and written at once so partial redraws never show.
    frame: Vec<u8>,
    committed_stages: Vec<StageId>,
    emitted_details: HashMap<StageId, usize>,
    live_region_active: bool,
    // A terminal resize can reflow any live line into several physical rows.
    live_lines: Vec<String>,
    last_width: usize,
    separator_pending: bool,
    started: bool,
}

impl<W: Write> InlineRenderer<W> {
    /// Create a renderer writing to `writer`.
    #[inline]
    pub fn new(writer: W) -> Self {
        Self {
            writer,
            frame: Vec::new(),
            committed_stages: Vec::new(),
            emitted_details: HashMap::new(),
            live_region_active: false,
            live_lines: Vec::new(),
            last_width: 1,
            separator_pending: false,
            started: false,
        }
    }

    /// Forget which stage summaries have already been emitted to scrollback.
    ///
    /// The managed region is still cleared on the next render, followed by a
    /// separator before the fresh stage sequence begins.
    pub fn reset(&mut self) {
        self.committed_stages.clear();
        self.emitted_details.clear();
        self.separator_pending = true;
    }

    /// Render one frame at the supplied terminal width.
    ///
    /// Completed summaries and newly visible details are appended as ordinary
    /// terminal lines. Only the live status area — the optional application-log
    /// separator, active stage, and navbar — is rewritten.
    pub fn render(&mut self, session: &InlineSession, width: usize) -> io::Result<()> {
        let width = width.max(1);
        self.clear_live_region(width)?;

        if self.separator_pending {
            self.append_line(separator_line(width), width)?;
            self.separator_pending = false;
        }

        if !self.started {
            self.append_line(format!("◆ Aimer · {}", session.target), width)?;
            self.append_line(String::new(), width)?;
            self.started = true;
        }

        let now = Instant::now();
        let newly_committed: Vec<(StageId, String)> = session
            .stages
            .stages()
            .iter()
            .filter(|stage| {
                matches!(
                    stage.status(),
                    StageStatus::Succeeded | StageStatus::Failed | StageStatus::Cancelled
                ) && !self.committed_stages.contains(&stage.id())
            })
            .map(|stage| {
                (
                    stage.id(),
                    fit_line(render_stage_summary(stage, now, false), width),
                )
            })
            .collect();
        for (id, summary) in newly_committed {
            self.append_line(summary, width)?;
            self.committed_stages.push(id);
        }

        let application_details_present = self.append_expanded_details(session, width)?;
        let application_separator = application_details_present.then(|| separator_line(width));

        let mut live_lines = Vec::with_capacity(4);
        live_lines.extend(application_separator);
        live_lines.push(render_active_stage(session, width));
        if let Some(notice) = session.active_notice(now) {
            live_lines.push(render_notice(notice, width));
        }
        live_lines.push(render_navbar(session, width));
        // The cursor rests at the end of the last live row, so rows are joined
        // by newlines rather than terminated by them.
        let last = live_lines.len() - 1;
        for (index, line) in live_lines.iter().enumerate() {
            write!(self.frame, "\r\x1b[2K{line}")?;
            if index != last {
                self.frame.push(b'\n');
            }
        }
        self.flush_frame()?;
        self.live_lines = live_lines;
        self.last_width = width;
        self.live_region_active = true;
        Ok(())
    }

    /// Leave the terminal on a clean line before returning to the shell.
    pub fn finish(&mut self) -> io::Result<()> {
        if self.live_region_active {
            let width = terminal::size()
                .map(|(width, _)| width as usize)
                .unwrap_or(self.last_width);
            self.clear_live_region(width)?;
            self.frame.extend_from_slice(b"\r\n");
            self.flush_frame()?;
        }
        Ok(())
    }

    fn clear_live_region(&mut self, width: usize) -> io::Result<()> {
        if self.live_region_active {
            let rows = self
                .live_lines
                .iter()
                .map(|line| physical_rows(line, width))
                .sum::<usize>();
            self.frame.extend_from_slice(b"\r\x1b[2K");
            for _ in 1..rows {
                self.frame.extend_from_slice(b"\x1b[1A\r\x1b[2K");
            }
            // A terminal that reflowed differently than `physical_rows`
            // predicted must not leave stale rows below the live region.
            self.frame.extend_from_slice(b"\r\x1b[J");
            self.live_region_active = false;
        }
        Ok(())
    }

    fn append_line(&mut self, line: String, width: usize) -> io::Result<()> {
        let line = fit_line(line, width);
        write!(self.frame, "\r\x1b[2K{line}\n")
    }

    fn flush_frame(&mut self) -> io::Result<()> {
        self.writer.write_all(&self.frame)?;
        self.frame.clear();
        self.writer.flush()
    }

    fn append_expanded_details(
        &mut self,
        session: &InlineSession,
        width: usize,
    ) -> io::Result<bool> {
        let mut application_details_present = false;
        for stage in session.stages.stages() {
            if stage.kind() != &StageKind::Application
                && !stage.is_expanded()
                && stage.status() != StageStatus::Failed
            {
                continue;
            }

            let details = detail_text_lines(stage);
            let emitted = self
                .emitted_details
                .get(&stage.id())
                .copied()
                .unwrap_or_default();
            let is_application = stage.kind() == &StageKind::Application;
            if is_application && !details.is_empty() {
                application_details_present = true;
            }
            let prefix = detail_prefix(stage.kind());
            for detail in details.iter().skip(emitted) {
                self.append_line(format!("{prefix}{detail}"), width)?;
            }
            self.emitted_details.insert(stage.id(), details.len());
        }
        Ok(application_details_present)
    }

    /// Return the wrapped writer after the renderer is no longer needed.
    #[inline]
    pub fn into_inner(self) -> W {
        self.writer
    }
}

fn physical_rows(line: &str, width: usize) -> usize {
    // The cursor is at the end of the last live line, so every wrapped row
    // above it must be cleared before the next frame starts at the top.
    let width = width.max(1);
    display_width(line).saturating_sub(1) / width + 1
}

fn display_width(line: &str) -> usize {
    UnicodeWidthStr::width(strip_ansi(line).as_str())
}

fn separator_line(width: usize) -> String {
    "─".repeat(width).dimmed().to_string()
}

#[cfg(test)]
fn render_lines(session: &InlineSession, width: usize) -> Vec<String> {
    render_managed_lines(session, width, &[])
}

fn render_active_stage(session: &InlineSession, width: usize) -> String {
    let line = if let Some((message, since)) = &session.build_lock_wait {
        // Name the cause: this is cargo queueing behind another process, not a
        // stalled build, and pressing `r` cannot shorten it.
        format!(
            "  {} {} {}",
            SPINNERS[session.frame % SPINNERS.len()],
            message.yellow().bold(),
            format!(
                "· {} · another cargo process (editor check, rust-analyzer) holds the lock",
                format_duration(since.elapsed())
            )
            .dimmed()
        )
    } else if let Some(id) = session.stages.active()
        && let Some(stage) = session.stages.stage(id)
    {
        let indicator = match stage.status() {
            StageStatus::Running => SPINNERS[session.frame % SPINNERS.len()],
            StageStatus::Succeeded => "✓",
            StageStatus::Failed => "✗",
            StageStatus::Cancelled => "⊘",
        };
        format!(
            "  {indicator} {} {}",
            styled_stage_label(stage.kind().label()),
            stage_metric(stage, Instant::now())
        )
    } else {
        format!("  status: {}", status_label(&session.state.status))
    };
    truncate_styled(&line, width)
}

fn detail_prefix(kind: &StageKind) -> &'static str {
    if kind == &StageKind::Application {
        ""
    } else {
        "    "
    }
}

fn render_notice(notice: &str, width: usize) -> String {
    fit_line(format!("  {notice}"), width).yellow().to_string()
}

fn render_navbar(session: &InlineSession, width: usize) -> String {
    let mut navbar = String::from("  ↑/↓ select · ");
    if session.hot_reload_enabled {
        navbar.push_str("^R hot-reload · ");
    }
    navbar.push_str("r restart · ⏎ expand · ⇧Q quit");
    fit_line(navbar, width).dimmed().to_string()
}

#[cfg(test)]
fn render_managed_lines(
    session: &InlineSession,
    width: usize,
    committed_stages: &[StageId],
) -> Vec<String> {
    let now = Instant::now();
    let mut lines = vec![format!("◆ Aimer · {}", session.target), String::new()];

    for stage in session.stages.stages() {
        if committed_stages.contains(&stage.id()) {
            continue;
        }
        let selected = session.stages.selected() == Some(stage.id());
        lines.push(render_stage_summary(stage, now, selected));

        let show_tail = stage.kind() == &StageKind::Application
            && stage.status() == StageStatus::Running
            && !stage.is_expanded();
        if stage.is_expanded() || show_tail {
            let detail_lines = detail_text_lines(stage);
            let start = if show_tail {
                detail_lines.len().saturating_sub(APPLICATION_TAIL_LINES)
            } else {
                detail_lines.len().saturating_sub(MAX_EXPANDED_LINES)
            };
            if start > 0 && !show_tail {
                lines.push(format!("    ... {} earlier lines", start));
            }
            for detail in detail_lines.into_iter().skip(start) {
                lines.push(format!("{}{}", detail_prefix(stage.kind()), detail));
            }
        }
    }

    if let Some(notice) = session.active_notice(now) {
        lines.push(format!("  {notice}"));
    }
    lines.push(String::new());
    lines.push(render_navbar(session, width));
    lines.push("  1 app · 2 build · 3 inspector · F12 inspector".to_string());

    lines
        .into_iter()
        .map(|line| fit_line(line, width))
        .collect()
}

fn render_stage_summary(
    stage: &crate::console::stage::Stage,
    now: Instant,
    selected: bool,
) -> String {
    let marker = if selected { "▸" } else { " " };
    let icon = match stage.status() {
        StageStatus::Running => "□",
        StageStatus::Succeeded => "✓",
        StageStatus::Failed => "✗",
        StageStatus::Cancelled => "⊘",
    };
    let detail_lines: usize = stage
        .details()
        .iter()
        .map(|detail| detail.line_count())
        .sum();
    format!(
        "{marker} {icon} {} {} · {} lines",
        styled_stage_label(stage.kind().label()),
        stage_metric(stage, now),
        detail_lines,
    )
}

/// What a stage reports next to its label: the percentage while it is running
/// and has a determinate one, otherwise how long it took (or has taken so far).
fn stage_metric(stage: &crate::console::stage::Stage, now: Instant) -> String {
    if stage.status() == StageStatus::Running
        && let Some(percent) = stage.progress().and_then(StageProgress::percentage)
    {
        return format!("{percent}%");
    }
    let elapsed = stage
        .timing()
        .elapsed()
        .unwrap_or_else(|| stage.timing().elapsed_at(now));
    format_duration(elapsed)
}

fn styled_stage_label(label: &str) -> String {
    let padding = " ".repeat(28usize.saturating_sub(label.chars().count()));
    format!("{}{}", label.bright_cyan().bold(), padding)
}

fn detail_text_lines(stage: &crate::console::stage::Stage) -> Vec<String> {
    stage
        .details()
        .iter()
        .flat_map(|detail| detail.as_str().lines().map(str::to_owned))
        .collect()
}

fn fit_line(line: String, width: usize) -> String {
    // ANSI-styled detail must remain intact. Let the terminal wrap a styled
    // line rather than truncating in the middle of an escape sequence and
    // leaking its style into the rest of the managed region.
    if line.contains('\x1b') {
        return line;
    }
    truncate_styled(&line, width)
}

/// Truncate `line` to `width` terminal columns, keeping ANSI styling intact.
///
/// Live rows use this so a narrow window never wraps them: every live row then
/// occupies exactly one terminal row, which keeps the redraw arithmetic exact
/// across a resize. Scrollback lines keep using [`fit_line`] so log text is
/// never cut.
fn truncate_styled(line: &str, width: usize) -> String {
    let width = width.max(1);
    if display_width(line) <= width {
        return line.to_owned();
    }
    let budget = width - 1;
    let mut out = String::with_capacity(line.len());
    let mut used = 0;
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            out.push(c);
            for next in chars.by_ref() {
                out.push(next);
                if next.is_ascii_alphabetic() || next == '@' || next == '~' {
                    break;
                }
            }
            continue;
        }
        let columns = c.width().unwrap_or(0);
        if used + columns > budget {
            break;
        }
        used += columns;
        out.push(c);
    }
    out.push('…');
    if line.contains('\x1b') {
        out.push_str("\x1b[0m");
    }
    out
}

fn format_duration(duration: Duration) -> String {
    let secs = duration.as_secs();
    if secs >= 60 {
        format!("{}m{:02}s", secs / 60, secs % 60)
    } else if secs > 0 {
        format!(
            "{}.{:01}s",
            duration.as_secs(),
            duration.subsec_millis() / 100
        )
    } else {
        format!("{}ms", duration.as_millis())
    }
}

fn status_label(status: &Status) -> &'static str {
    match status {
        Status::Locking => "locking",
        Status::Fetching(_) => "fetching",
        Status::Downloading(_) => "downloading",
        Status::Compiling(_) => "compiling",
        Status::Building(_) => "assembling",
        Status::Launching => "launching",
        Status::Running => "running",
        Status::Idling => "idle",
        Status::Error => "error",
    }
}

/// Start the experimental inline console.
pub fn start(
    device: Device,
    pkg_name: String,
    release: bool,
    hot_reload_enabled: bool,
    control_session: Option<SessionHandle>,
) -> anyhow::Result<()> {
    // Raw mode needs a terminal on both ends: without one (a pipe, CI, an IDE's
    // run window) fall back to plain logs rather than failing to start.
    if !stdout().is_terminal() || !io::stdin().is_terminal() {
        return super::start_no_tui(device, pkg_name, release, control_session);
    }

    let _guard = RawModeGuard::new()?;
    let (mut tx, mut rx) = crossbeam::channel::unbounded();
    let inspector_runtime =
        Runtime::new().context("failed to start inspector server tokio runtime")?;
    let inspector_server_address = match device.target {
        Targets::Ios | Targets::Android => IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)),
        _ => IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
    };
    let inspector_handle =
        InspectorServer::start(inspector_server_address, 9229, inspector_runtime.handle())
            .context("failed to start inspector server")?;

    let project_root = super::get_project_root(false)
        .map_err(|_| anyhow::anyhow!("failed to get project root"))?;
    let mut file_writer = if hot_reload_enabled && device.target == Targets::Web {
        let lib_path = project_root.join("src/main.rs");
        Some(
            File::options()
                .append(true)
                .read(true)
                .create(false)
                .open(lib_path)
                .context("failed to open src/main.rs for inline reload")?,
        )
    } else {
        None
    };

    let mut current_child = super::spawn_runner(
        device.clone(),
        pkg_name.clone(),
        tx.clone(),
        inspector_handle.address,
        inspector_handle.port,
        release,
        control_session.clone(),
    );

    let mut session = InlineSession::new_with_hot_reload(&device, hot_reload_enabled);
    let mut renderer = InlineRenderer::new(stdout());
    let tick_rate = Duration::from_millis(100);
    let mut last_tick = Instant::now();
    let mut quit = false;

    while !quit {
        while let Some(command) = control_session
            .as_ref()
            .and_then(SessionHandle::take_command)
        {
            match command {
                SessionCommand::Restart { reply } => {
                    if control_session
                        .as_ref()
                        .expect("session command requires a session")
                        .accept_restart(reply)
                        .is_err()
                    {
                        continue;
                    }
                    // The superseded run keeps its old channel, which is dropped here,
                    // so its late events cannot reach the new run's transcript.
                    current_child.stop();
                    (tx, rx) = crossbeam::channel::unbounded();
                    session.apply_status(Status::Compiling(0));
                    renderer.reset();
                    current_child = super::spawn_runner(
                        device.clone(),
                        pkg_name.clone(),
                        tx.clone(),
                        inspector_handle.address,
                        inspector_handle.port,
                        release,
                        control_session.clone(),
                    );
                }
                SessionCommand::Stop { reply } => {
                    current_child.stop();
                    control_session
                        .as_ref()
                        .expect("session command requires a session")
                        .complete_stop(reply);
                    quit = true;
                }
                SessionCommand::ClearLogs { request, reply } => {
                    let stream = request.stream;
                    control_session
                        .as_ref()
                        .expect("session command requires a session")
                        .complete_clear_logs(request, reply);
                    match stream {
                        Some(LogStream::App) => session.state.app_logs.clear(),
                        Some(LogStream::Build) => session.state.build_logs.clear(),
                        _ => {}
                    }
                }
            }
        }
        if quit {
            break;
        }
        while let Ok(event) = rx.try_recv() {
            super::observe_session_event(control_session.as_ref(), &event);
            session.apply_event(event);
        }

        let width = terminal::size()
            .map(|(width, _)| width as usize)
            .unwrap_or(120);
        renderer.render(&session, width)?;

        let timeout = tick_rate
            .checked_sub(last_tick.elapsed())
            .unwrap_or(Duration::ZERO);
        for event in input::drain_terminal(timeout)? {
            let Event::Key(key) = event else {
                continue;
            };
            let Some(action) = map_key_event(&key) else {
                continue;
            };
            match action {
                ConsoleAction::Quit => {
                    current_child.stop();
                    quit = true;
                }
                ConsoleAction::HotReload => {
                    if !hot_reload_enabled {
                        continue;
                    }
                    if let Some(file_writer) = file_writer.as_mut() {
                        file_writer.write_all(b" ")?;
                        file_writer.set_modified(SystemTime::now())?;
                        session.notice("Hot reload requested".to_string());
                    } else {
                        session.notice(
                            "Hot reload is unavailable for this target; press r to restart"
                                .to_string(),
                        );
                    }
                }
                ConsoleAction::HotRestart => {
                    if let Some(control_session) = control_session.as_ref()
                        && control_session.begin_next_run().is_err()
                    {
                        session.notice("Restart is busy; try again shortly".to_string());
                        continue;
                    }
                    current_child.stop();
                    (tx, rx) = crossbeam::channel::unbounded();
                    session.reset_for_restart();
                    renderer.reset();
                    current_child = super::spawn_runner(
                        device.clone(),
                        pkg_name.clone(),
                        tx.clone(),
                        inspector_handle.address,
                        inspector_handle.port,
                        release,
                        control_session.clone(),
                    );
                }
                ConsoleAction::SelectPane(pane) => select_pane(&mut session, pane),
                ConsoleAction::NextPane => {
                    session.state.pane = match session.state.pane {
                        super::ConsoleType::App => super::ConsoleType::Build,
                        super::ConsoleType::Build => super::ConsoleType::Inspector,
                        super::ConsoleType::Inspector | super::ConsoleType::ResourceMonitor => {
                            super::ConsoleType::App
                        }
                    }
                }
                ConsoleAction::ToggleInspector => {
                    inspector_handle.send_toggle();
                    session.state.pane = super::ConsoleType::Inspector;
                }
                ConsoleAction::ToggleInspectorTree => {
                    session.state.inspector_full_tree = !session.state.inspector_full_tree;
                }
                ConsoleAction::ToggleSourceLocations => session.state.toggle_log_source(),
                ConsoleAction::ToggleSelectionMode => {
                    session.state.selection_mode = !session.state.selection_mode;
                    if !session.state.selection_mode {
                        session.state.clear_selection();
                    }
                }
                ConsoleAction::YankSelection
                | ConsoleAction::CopyPane
                | ConsoleAction::CopySelectionOrPane => copy_focused_pane(&session),
                ConsoleAction::ClearPane => match session.state.pane {
                    super::ConsoleType::App => session.state.app_logs.clear(),
                    super::ConsoleType::Build => session.state.build_logs.clear(),
                    super::ConsoleType::Inspector => {}
                    super::ConsoleType::ResourceMonitor => {}
                },
                ConsoleAction::ScrollUp | ConsoleAction::PageUp => {
                    let count = if matches!(action, ConsoleAction::PageUp) {
                        10
                    } else {
                        1
                    };
                    for _ in 0..count {
                        session.stages.select_previous();
                    }
                }
                ConsoleAction::ScrollDown | ConsoleAction::PageDown => {
                    let count = if matches!(action, ConsoleAction::PageDown) {
                        10
                    } else {
                        1
                    };
                    for _ in 0..count {
                        session.stages.select_next();
                    }
                }
                ConsoleAction::ExpandStage => {
                    session.stages.toggle_selected();
                }
            }
        }

        if last_tick.elapsed() >= tick_rate {
            session.frame = session.frame.wrapping_add(1);
            last_tick = Instant::now();
        }
    }

    renderer.finish()?;
    Ok(())
}

fn select_pane(session: &mut InlineSession, pane: ConsolePane) {
    session.state.pane = match pane {
        ConsolePane::App => super::ConsoleType::App,
        ConsolePane::Build => super::ConsoleType::Build,
        ConsolePane::Inspector => super::ConsoleType::Inspector,
    };
    session.state.clear_selection();
}

fn copy_focused_pane(session: &InlineSession) {
    let text = match session.state.pane {
        super::ConsoleType::App => session.state.app_log_text(),
        super::ConsoleType::Build => session.state.build_log_text(),
        super::ConsoleType::Inspector => return,
        super::ConsoleType::ResourceMonitor => return,
    };
    if let Ok(mut clipboard) = Clipboard::new() {
        let _ = clipboard.set_text(text);
    }
}

#[cfg(test)]
const APPLICATION_TAIL_LINES: usize = 3;
#[cfg(test)]
const MAX_EXPANDED_LINES: usize = 200;

#[cfg(test)]
mod tests;
