//! Cargo's machine-readable build output.
//!
//! `aimer run` asks cargo for JSON messages ([`MESSAGE_FORMAT`]) instead of
//! plain text so the console gets structured access to every diagnostic. Each
//! line cargo writes on stdout is one JSON object; [`CargoMessage::parse`] turns
//! it back into something printable.
//!
//! Two things fall out of that:
//!
//! * **The build pane still looks like a real `cargo build`.** Cargo renders
//!   every diagnostic for a terminal itself and ships the result in the
//!   `rendered` field, colors and carets included, so replaying it reproduces
//!   cargo's own output byte for byte. The `Compiling` / `Finished` status lines
//!   are unaffected: cargo keeps writing those to stderr even in JSON mode.
//! * **Errors can be repeated at the end of the build.** [`ErrorReport`]
//!   collects the failing diagnostics while they stream past and formats them as
//!   one `Compile Error` block, so a failed build ends with everything that went
//!   wrong instead of burying it in the scrollback.

use serde::Deserialize;

use crate::commands::run::framed_block::{block_width, divider, header, panel, resolve_width};
use crate::console::state::strip_ansi;

/// The `--message-format` cargo needs for [`CargoMessage::parse`] to see
/// anything.
///
/// The `-diagnostic-rendered-ansi` suffix is what makes cargo put terminal
/// colors into the `rendered` field of a diagnostic; without it the replayed
/// output would be plain grey text.
pub const MESSAGE_FORMAT: &str = "--message-format=json-diagnostic-rendered-ansi";

/// Severity of a rustc diagnostic, as spelled in cargo's `level` field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiagnosticLevel {
    Error,
    Warning,
    Note,
    Help,
    /// The trailing hints of a failed build, e.g. *"Some errors have detailed
    /// explanations"*. They are printed like any other diagnostic but are not
    /// errors of their own, so they never appear in an [`ErrorReport`].
    FailureNote,
}

impl DiagnosticLevel {
    /// Map cargo's `level` string, or `None` for a level we don't know — such a
    /// message is passed through verbatim rather than guessed at.
    fn from_cargo(level: &str) -> Option<Self> {
        match level {
            "error" | "error: internal compiler error" => Some(Self::Error),
            "warning" => Some(Self::Warning),
            "note" => Some(Self::Note),
            "help" => Some(Self::Help),
            "failure-note" => Some(Self::FailureNote),
            _ => None,
        }
    }

    /// Whether a diagnostic of this level makes the build fail.
    #[inline]
    pub fn is_error(&self) -> bool {
        matches!(self, Self::Error)
    }
}

/// One rustc diagnostic pulled out of a cargo `compiler-message`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    /// How severe rustc considered it.
    pub level: DiagnosticLevel,
    /// The headline message, without the `error[E0308]: ` prefix.
    pub message: String,
    /// The error index code, e.g. `E0308`, when rustc assigned one.
    pub code: Option<String>,
    /// `file:line:column` of the primary span, when the diagnostic has one.
    pub location: Option<String>,
    /// Cargo's own terminal rendering of the diagnostic.
    rendered: Option<String>,
}

impl Diagnostic {
    /// Whether this diagnostic is one of the errors that failed the build.
    #[inline]
    pub fn is_error(&self) -> bool {
        self.level.is_error()
    }

    /// The diagnostic exactly as `cargo build` would have printed it.
    ///
    /// Cargo's own rendering is used when present — that is what keeps the build
    /// pane indistinguishable from a plain `cargo build`. Otherwise a single
    /// `level[code]: message` line is synthesised, so a diagnostic is never
    /// silently dropped.
    pub fn render(&self) -> String {
        match self.rendered.as_deref() {
            Some(rendered) if !rendered.trim().is_empty() => {
                rendered.trim_end_matches('\n').to_string()
            }
            _ => self.headline(),
        }
    }

    /// [`render`](Self::render) split into console lines, since the panes store
    /// one line per entry.
    pub fn render_lines(&self) -> Vec<String> {
        self.render().split('\n').map(str::to_string).collect()
    }

    /// The `level[code]: message` first line of the diagnostic.
    pub fn headline(&self) -> String {
        match &self.code {
            Some(code) => format!("{}[{}]: {}", self.label(), code, self.message),
            None => format!("{}: {}", self.label(), self.message),
        }
    }

    /// How rustc names this level in its output.
    fn label(&self) -> &'static str {
        match self.level {
            DiagnosticLevel::Error => "error",
            DiagnosticLevel::Warning => "warning",
            DiagnosticLevel::Note | DiagnosticLevel::FailureNote => "note",
            DiagnosticLevel::Help => "help",
        }
    }
}

/// A line of cargo's JSON output, reduced to what the console does something
/// with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CargoMessage {
    /// A rustc diagnostic (`compiler-message`).
    Diagnostic(Diagnostic),
    /// A compilation unit finished (`compiler-artifact`), named by its target.
    Artifact { target: String },
    /// The build ended (`build-finished`).
    Finished { success: bool },
    /// A message the console has nothing to add to, e.g.
    /// `build-script-executed`.
    Other,
}

impl CargoMessage {
    /// Parse one line of cargo's JSON output.
    ///
    /// Returns `None` for anything that is not a cargo message — an empty line,
    /// text a build script printed on stdout, or a JSON object without a
    /// `reason`. Callers forward those lines unchanged, so nothing a build emits
    /// is ever swallowed.
    pub fn parse(line: &str) -> Option<Self> {
        let trimmed = line.trim();
        if !trimmed.starts_with('{') {
            return None;
        }
        let raw: RawMessage = serde_json::from_str(trimmed).ok()?;
        Some(match raw.reason.as_str() {
            "compiler-message" => {
                let message = raw.message?;
                let level = DiagnosticLevel::from_cargo(&message.level)?;
                let location = message
                    .spans
                    .iter()
                    .find(|span| span.is_primary)
                    .or_else(|| message.spans.first())
                    .map(RawSpan::location);
                Self::Diagnostic(Diagnostic {
                    level,
                    message: message.message,
                    code: message.code.map(|code| code.code),
                    location,
                    rendered: message.rendered,
                })
            }
            "compiler-artifact" => Self::Artifact {
                target: raw.target.map(|t| t.name).unwrap_or_default(),
            },
            "build-finished" => Self::Finished {
                success: raw.success.unwrap_or(false),
            },
            _ => Self::Other,
        })
    }
}

/// The errors of one build, kept so they can be shown again together once the
/// build has failed.
///
/// A long build scrolls its errors far out of view; replaying them as a single
/// block at the very end means the last thing on screen is always the reason the
/// build failed.
#[derive(Clone, Debug, Default)]
pub struct ErrorReport {
    errors: Vec<Diagnostic>,
}

impl ErrorReport {
    /// An empty report.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Remember `diagnostic` if it is an error; anything else is ignored, so
    /// warnings and notes don't pad the summary.
    pub fn record(&mut self, diagnostic: &Diagnostic) {
        if diagnostic.is_error() {
            self.errors.push(diagnostic.clone());
        }
    }

    /// Whether no error was recorded, in which case [`lines`](Self::lines)
    /// produces nothing.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.errors.is_empty()
    }

    /// The report as console lines, laid out for the current terminal — see
    /// [`lines_with_width`](Self::lines_with_width).
    #[inline]
    pub fn lines(&self) -> Vec<String> {
        self.lines_with_width(block_width())
    }

    /// The report as console lines `width` cells wide: a framed `Compile Error`
    /// block in which every error sits on its own light-red panel, panels
    /// separated by wave rules.
    ///
    /// Cargo's rendering already states the level, the code and the location, so
    /// nothing is written above a panel — the panel itself is what tells the
    /// errors apart.
    ///
    /// A rendering wider than `width` is wrapped rather than truncated: cutting
    /// cargo's carets off would make the diagnostic unreadable, and letting the
    /// row overflow would leave the console to wrap it and tear the panel apart.
    /// A `width` of zero means the width isn't known and [`MIN_WIDTH`] is used
    /// instead; every other width is honoured, however narrow the pane is.
    ///
    /// Empty when no error was recorded, so a successful build appends nothing.
    pub fn lines_with_width(&self, width: usize) -> Vec<String> {
        if self.errors.is_empty() {
            return Vec::new();
        }

        let width = resolve_width(width);
        let mut lines = vec![String::new(), header("Compile Error", width), String::new()];

        for (index, error) in self.errors.iter().enumerate() {
            if index > 0 {
                lines.push(divider(width));
            }
            lines.extend(panel(&error.render_lines(), width));
        }

        lines.push(String::new());
        lines.push(header(
            &match self.errors.len() {
                1 => "1 error".to_string(),
                n => format!("{} errors", n),
            },
            width,
        ));
        lines
    }
}

/// Headline prefixes of the summaries cargo and rustc print once a build has
/// failed.
///
/// They are spelled as errors but restate the failure instead of describing one,
/// and cargo's JSON output doesn't carry them as diagnostics either — so keeping
/// them out is what makes a scraped report identical to a parsed one.
const FAILURE_SUMMARIES: &[&str] = &[
    "could not compile",
    "aborting due to",
    "build failed",
    "failed to compile",
];

/// Collects rustc diagnostics out of build output that is already *rendered*.
///
/// Not every build speaks cargo's JSON: `cargo ndk` appends its own
/// `--message-format json-render-diagnostics`, and `wasm-pack` drives cargo
/// without forwarding the flag at all. Both end up with cargo rendering the
/// diagnostics as plain text on stderr, which leaves
/// [`CargoMessage::parse`] nothing to work with.
///
/// Feeding that stream through this collector recovers the same
/// [`ErrorReport`] the JSON path produces, so every target ends a failed build
/// with the one `Compile Error` block:
///
/// ```ignore
/// let mut scraper = RenderedDiagnostics::new();
/// for line in output.lines() {
///     scraper.push(line);
/// }
/// let report = scraper.finish();
/// ```
///
/// A diagnostic is recognised by its `level[code]: message` headline and runs
/// until the next line that starts in the first column, which is exactly how
/// rustc lays its output out. Lines that belong to no diagnostic — the
/// `Compiling` / `Finished` status lines, a build script's own output — are
/// ignored.
#[derive(Debug, Default)]
pub struct RenderedDiagnostics {
    report: ErrorReport,
    /// The diagnostic whose continuation lines are still arriving.
    pending: Option<PendingDiagnostic>,
}

/// A diagnostic whose headline has been seen and whose body is still being read.
#[derive(Debug)]
struct PendingDiagnostic {
    level: DiagnosticLevel,
    message: String,
    code: Option<String>,
    location: Option<String>,
    /// The lines as they arrived, colors included, so the report replays the
    /// rendering rather than a reconstruction of it.
    rendered: Vec<String>,
}

impl PendingDiagnostic {
    /// The finished diagnostic, with the blank lines rustc leaves after a body
    /// trimmed off.
    fn into_diagnostic(mut self) -> Diagnostic {
        while self
            .rendered
            .last()
            .is_some_and(|line| strip_ansi(line).trim().is_empty())
        {
            self.rendered.pop();
        }
        Diagnostic {
            level: self.level,
            message: self.message,
            code: self.code,
            location: self.location,
            rendered: Some(self.rendered.join("\n")),
        }
    }
}

impl RenderedDiagnostics {
    /// A collector that has seen nothing yet.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one line of build output.
    pub fn push(&mut self, line: &str) {
        let plain = strip_ansi(line);
        if let Some((level, code, message)) = parse_headline(&plain) {
            self.flush();
            if level.is_error() && is_failure_summary(&message) {
                return;
            }
            self.pending = Some(PendingDiagnostic {
                level,
                message,
                code,
                location: None,
                rendered: vec![line.to_string()],
            });
            return;
        }

        let Some(pending) = self.pending.as_mut() else {
            return;
        };
        if !is_continuation(&plain) {
            self.flush();
            return;
        }
        if pending.location.is_none()
            && let Some(location) = plain.split_once("--> ")
        {
            pending.location = Some(location.1.trim().to_string());
        }
        pending.rendered.push(line.to_string());
    }

    /// The report of everything collected so far, closing the diagnostic the
    /// stream ended on.
    pub fn finish(mut self) -> ErrorReport {
        self.flush();
        self.report
    }

    /// Record the diagnostic that was being read, if any.
    fn flush(&mut self) {
        if let Some(pending) = self.pending.take() {
            self.report.record(&pending.into_diagnostic());
        }
    }
}

/// Split a `level[code]: message` headline, as rustc prints it in the first
/// column, into its parts.
///
/// `None` for anything else, including a level this console has no rendering
/// for — the line is then just build output.
fn parse_headline(line: &str) -> Option<(DiagnosticLevel, Option<String>, String)> {
    if line.starts_with(char::is_whitespace) {
        return None;
    }
    let (head, message) = line.split_once(": ")?;
    let (level, code) = match head.split_once('[') {
        Some((level, code)) => (level, Some(code.strip_suffix(']')?.to_string())),
        None => (head, None),
    };
    Some((
        DiagnosticLevel::from_cargo(level)?,
        code,
        message.trim().to_string(),
    ))
}

/// Whether `line` belongs to the body of the diagnostic above it.
///
/// rustc indents everything below a headline — the `-->` location, the `|`
/// gutter, the notes — with one exception: a numbered source line starts with
/// its line number in the first column (`1 | pub fn f() ...`), as does the `...`
/// that stands for the lines it skipped. Blank lines separate the parts of a
/// body and are kept, then trimmed off its end.
fn is_continuation(line: &str) -> bool {
    if line.is_empty() || line.starts_with(char::is_whitespace) || line.starts_with("...") {
        return true;
    }
    let digits = line.trim_start_matches(|c: char| c.is_ascii_digit());
    digits.len() < line.len() && digits.trim_start().starts_with('|')
}

/// Whether `message` is one of the [`FAILURE_SUMMARIES`] rather than a
/// diagnostic of its own.
fn is_failure_summary(message: &str) -> bool {
    FAILURE_SUMMARIES
        .iter()
        .any(|summary| message.starts_with(summary))
}

// ── Cargo's JSON shape ───────────────────────────────────────────────

#[derive(Deserialize)]
struct RawMessage {
    reason: String,
    #[serde(default)]
    success: Option<bool>,
    #[serde(default)]
    target: Option<RawTarget>,
    #[serde(default)]
    message: Option<RawDiagnostic>,
}

#[derive(Deserialize)]
struct RawTarget {
    name: String,
}

#[derive(Deserialize)]
struct RawDiagnostic {
    level: String,
    message: String,
    #[serde(default)]
    rendered: Option<String>,
    #[serde(default)]
    code: Option<RawCode>,
    #[serde(default)]
    spans: Vec<RawSpan>,
}

#[derive(Deserialize)]
struct RawCode {
    code: String,
}

#[derive(Deserialize)]
struct RawSpan {
    file_name: String,
    line_start: u32,
    column_start: u32,
    #[serde(default)]
    is_primary: bool,
}

impl RawSpan {
    fn location(&self) -> String {
        format!(
            "{}:{}:{}",
            self.file_name, self.line_start, self.column_start
        )
    }
}

#[cfg(test)]
mod tests;
