use super::*;
use crate::commands::run::framed_block::{DIVIDER_COLOR, MIN_WIDTH, PANEL_BACKGROUND};

/// A `compiler-message` line as cargo writes it.
fn compiler_message(level: &str, message: &str, code: Option<&str>, rendered: &str) -> String {
    let code = match code {
        Some(code) => format!(r#""code":{{"code":"{code}","explanation":null}}"#),
        None => r#""code":null"#.to_string(),
    };
    format!(
        r#"{{"reason":"compiler-message","package_id":"path+file:///app#app@0.1.0","target":{{"name":"app","kind":["bin"]}},"message":{{"level":"{level}","message":"{message}",{code},"spans":[{{"file_name":"src/main.rs","line_start":2,"column_start":18,"is_primary":true}}],"rendered":"{rendered}"}}}}"#
    )
}

/// Parse `line` and unwrap it as a diagnostic.
fn diagnostic(line: &str) -> Diagnostic {
    match CargoMessage::parse(line) {
        Some(CargoMessage::Diagnostic(diagnostic)) => diagnostic,
        other => panic!("expected a diagnostic, got {other:?}"),
    }
}

/// An error carrying `message`, as if cargo had rendered it.
fn error_with(message: &str, code: &str) -> Diagnostic {
    Diagnostic {
        level: DiagnosticLevel::Error,
        message: message.to_string(),
        code: Some(code.to_string()),
        location: Some("src/main.rs:2:18".to_string()),
        rendered: Some(format!("error[{code}]: {message}\n --> src/main.rs:2:18\n")),
    }
}

// ── Parsing ──────────────────────────────────────────────────────

#[test]
fn parse_reads_an_error_diagnostic() {
    let error = diagnostic(&compiler_message(
        "error",
        "mismatched types",
        Some("E0308"),
        "rendered",
    ));

    assert_eq!(error.level, DiagnosticLevel::Error);
    assert!(error.is_error());
    assert_eq!(error.message, "mismatched types");
    assert_eq!(error.code.as_deref(), Some("E0308"));
    assert_eq!(error.location.as_deref(), Some("src/main.rs:2:18"));
}

#[test]
fn parse_reads_a_warning_diagnostic() {
    let warning = diagnostic(&compiler_message("warning", "unused variable", None, "w"));

    assert_eq!(warning.level, DiagnosticLevel::Warning);
    assert!(!warning.is_error());
    assert!(warning.code.is_none());
}

#[test]
fn parse_reads_a_failure_note_without_making_it_an_error() {
    let note = diagnostic(&compiler_message(
        "failure-note",
        "Some errors have detailed explanations.",
        None,
        "note",
    ));

    assert_eq!(note.level, DiagnosticLevel::FailureNote);
    assert!(!note.is_error());
}

#[test]
fn parse_reads_an_artifact() {
    let line = r#"{"reason":"compiler-artifact","target":{"name":"aimer_app","kind":["lib"]},"filenames":[]}"#;

    assert_eq!(
        CargoMessage::parse(line),
        Some(CargoMessage::Artifact {
            target: "aimer_app".to_string()
        })
    );
}

#[test]
fn parse_reads_the_build_outcome() {
    assert_eq!(
        CargoMessage::parse(r#"{"reason":"build-finished","success":false}"#),
        Some(CargoMessage::Finished { success: false })
    );
    assert_eq!(
        CargoMessage::parse(r#"{"reason":"build-finished","success":true}"#),
        Some(CargoMessage::Finished { success: true })
    );
}

#[test]
fn parse_maps_unhandled_reasons_to_other() {
    let line = r#"{"reason":"build-script-executed","package_id":"x","linked_libs":[]}"#;
    assert_eq!(CargoMessage::parse(line), Some(CargoMessage::Other));
}

#[test]
fn parse_rejects_output_that_is_not_a_cargo_message() {
    assert!(CargoMessage::parse("").is_none());
    assert!(CargoMessage::parse("   ").is_none());
    assert!(CargoMessage::parse("Compiling app v0.1.0").is_none());
    assert!(CargoMessage::parse("[1, 2, 3]").is_none());
    assert!(CargoMessage::parse(r#"{"level":"info","message":"app log"}"#).is_none());
    assert!(CargoMessage::parse(r#"{"reason":"compiler-message"}"#).is_none());
    assert!(CargoMessage::parse(r#"{"reason":"broken"#).is_none());
}

#[test]
fn parse_rejects_an_unknown_diagnostic_level() {
    let line = compiler_message("gossip", "who knows", None, "x");
    assert!(CargoMessage::parse(&line).is_none());
}

#[test]
fn parse_prefers_the_primary_span_for_the_location() {
    let line = r#"{"reason":"compiler-message","message":{"level":"error","message":"m","spans":[{"file_name":"src/a.rs","line_start":1,"column_start":1,"is_primary":false},{"file_name":"src/b.rs","line_start":7,"column_start":3,"is_primary":true}],"rendered":"r"}}"#;

    assert_eq!(diagnostic(line).location.as_deref(), Some("src/b.rs:7:3"));
}

#[test]
fn parse_leaves_the_location_out_when_there_is_no_span() {
    let line = r#"{"reason":"compiler-message","message":{"level":"error","message":"m","spans":[],"rendered":"r"}}"#;
    assert!(diagnostic(line).location.is_none());
}

// ── Rendering ────────────────────────────────────────────────────

#[test]
fn render_replays_cargos_own_output() {
    let line = compiler_message(
        "error",
        "mismatched types",
        Some("E0308"),
        "error[E0308]: mismatched types\\n --> src/main.rs:2:18\\n",
    );

    assert_eq!(
        diagnostic(&line).render(),
        "error[E0308]: mismatched types\n --> src/main.rs:2:18"
    );
}

#[test]
fn render_keeps_the_colors_cargo_produced() {
    let line = compiler_message("error", "boom", None, "\\u001b[91merror\\u001b[0m: boom");
    let rendered = diagnostic(&line).render();

    assert!(rendered.contains('\u{1b}'));
    assert_eq!(strip_ansi(&rendered), "error: boom");
}

#[test]
fn render_falls_back_to_the_headline_without_a_rendering() {
    let line = r#"{"reason":"compiler-message","message":{"level":"error","message":"mismatched types","code":{"code":"E0308"},"spans":[]}}"#;
    assert_eq!(diagnostic(line).render(), "error[E0308]: mismatched types");
}

#[test]
fn render_falls_back_when_the_rendering_is_blank() {
    let line = compiler_message("warning", "unused", None, "   ");
    assert_eq!(diagnostic(&line).render(), "warning: unused");
}

#[test]
fn render_lines_splits_the_rendering_into_console_lines() {
    let line = compiler_message("error", "m", None, "first\\nsecond\\nthird");
    assert_eq!(
        diagnostic(&line).render_lines(),
        vec!["first", "second", "third"]
    );
}

// ── ErrorReport ──────────────────────────────────────────────────

/// The report block as plain text lines, at a fixed width so the assertions
/// don't depend on the terminal the tests happen to run in.
fn report_lines(report: &ErrorReport) -> Vec<String> {
    report
        .lines_with_width(MIN_WIDTH)
        .iter()
        .map(|l| strip_ansi(l))
        .collect()
}

/// The report block at `width`, as plain text lines.
fn report_lines_at(report: &ErrorReport, width: usize) -> Vec<String> {
    report
        .lines_with_width(width)
        .iter()
        .map(|l| strip_ansi(l))
        .collect()
}

#[test]
fn error_report_starts_empty() {
    let report = ErrorReport::new();
    assert!(report.is_empty());
    assert!(report.lines().is_empty());
}

#[test]
fn error_report_ignores_everything_that_is_not_an_error() {
    let mut report = ErrorReport::new();
    for level in [
        DiagnosticLevel::Warning,
        DiagnosticLevel::Note,
        DiagnosticLevel::Help,
        DiagnosticLevel::FailureNote,
    ] {
        let mut diagnostic = error_with("m", "E0001");
        diagnostic.level = level;
        report.record(&diagnostic);
    }

    assert!(report.is_empty());
    assert!(report.lines().is_empty());
}

#[test]
fn error_report_of_one_error_is_framed_and_unseparated() {
    let mut report = ErrorReport::new();
    report.record(&error_with("mismatched types", "E0308"));

    let lines = report_lines(&report);
    assert!(!report.is_empty());
    assert!(lines[1].contains("Compile Error"));
    assert!(lines[1].starts_with('='));
    // The rendering cargo produced is replayed as is, padded to the panel.
    assert!(
        lines
            .iter()
            .any(|l| l.trim_end() == "error[E0308]: mismatched types")
    );
    assert!(lines.last().unwrap().contains("1 error"));
    // A single error needs no rule.
    assert!(!lines.iter().any(|l| l.starts_with('~')));
}

#[test]
fn error_report_does_not_label_its_errors() {
    // Cargo's own rendering states level, code and location already.
    let mut report = ErrorReport::new();
    report.record(&error_with("mismatched types", "E0308"));
    report.record(&error_with("cannot find function", "E0425"));

    assert!(!report_lines(&report).iter().any(|l| l.contains("Error 1")));
}

#[test]
fn error_report_separates_several_errors_with_a_wave_rule() {
    let mut report = ErrorReport::new();
    report.record(&error_with("mismatched types", "E0308"));
    report.record(&error_with("cannot find function", "E0425"));

    let lines = report_lines(&report);
    assert_eq!(
        lines.iter().filter(|l| l.starts_with("~~~")).count(),
        1,
        "one rule between two errors"
    );
    assert!(lines.last().unwrap().contains("2 errors"));
}

#[test]
fn error_report_rule_is_dimmer_than_the_headers() {
    let mut report = ErrorReport::new();
    report.record(&error_with("a", "E0001"));
    report.record(&error_with("b", "E0002"));

    let (r, g, b) = DIVIDER_COLOR;
    let rule = report
        .lines()
        .into_iter()
        .find(|l| strip_ansi(l).starts_with('~'))
        .expect("a wave rule");
    assert!(rule.contains(&format!("38;2;{r};{g};{b}")), "{rule:?}");
}

#[test]
fn error_report_paints_every_error_line_on_the_panel() {
    let mut report = ErrorReport::new();
    report.record(&error_with("mismatched types", "E0308"));

    let (r, g, b) = PANEL_BACKGROUND;
    let background = format!("48;2;{r};{g};{b}");
    let panelled = report
        .lines()
        .into_iter()
        .filter(|l| l.contains(&background))
        .count();
    // Both rendered lines plus the blank row above and below.
    assert_eq!(panelled, 4);
}

#[test]
fn error_report_rearms_the_panel_after_a_reset() {
    // Cargo resets colors mid-line; without re-arming, the background would
    // stop at the first reset.
    let mut error = error_with("m", "E0001");
    error.rendered = Some("\x1b[31merror\x1b[0m: m".to_string());
    let mut report = ErrorReport::new();
    report.record(&error);

    let (r, g, b) = PANEL_BACKGROUND;
    let background = format!("\x1b[48;2;{r};{g};{b}m");
    let line = report
        .lines()
        .into_iter()
        .find(|l| strip_ansi(l).starts_with("error"))
        .expect("the rendered line");
    assert_eq!(line.matches(&background).count(), 2, "{line:?}");
}

#[test]
fn error_report_pads_every_panel_line_to_one_width() {
    let mut error = error_with("m", "E0001");
    error.rendered = Some("short\na much longer rendered line".to_string());
    let mut report = ErrorReport::new();
    report.record(&error);

    let widths: Vec<usize> = report_lines(&report)
        .iter()
        .filter(|l| !l.is_empty() && !l.starts_with('='))
        .map(|l| l.chars().count())
        .collect();
    assert!(widths.iter().all(|w| *w == MIN_WIDTH), "{widths:?}");
}

#[test]
fn error_report_fills_the_width_it_is_given() {
    // The block spans the whole Build Logs pane rather than a fixed 58
    // columns, so every rule and panel row grows with the width.
    let mut report = ErrorReport::new();
    report.record(&error_with("a", "E0001"));
    report.record(&error_with("b", "E0002"));

    let width = 120;
    for line in report_lines_at(&report, width)
        .iter()
        .filter(|l| !l.is_empty())
    {
        assert_eq!(line.chars().count(), width, "{line:?}");
    }
}

#[test]
fn error_report_falls_back_to_the_minimum_width_when_none_is_known() {
    let mut report = ErrorReport::new();
    report.record(&error_with("m", "E0001"));

    for line in report_lines_at(&report, 0).iter().filter(|l| !l.is_empty()) {
        assert_eq!(line.chars().count(), MIN_WIDTH, "{line:?}");
    }
}

#[test]
fn error_report_fits_a_pane_narrower_than_the_minimum_width() {
    // A block wider than the pane wraps and tears, which is worse than a
    // narrow one, so a narrow pane is honoured as given.
    let mut report = ErrorReport::new();
    report.record(&error_with("mismatched types", "E0308"));

    for line in report_lines_at(&report, 30)
        .iter()
        .filter(|l| !l.is_empty())
    {
        assert_eq!(line.chars().count(), 30, "{line:?}");
    }
}

#[test]
fn error_report_wraps_a_rendering_wider_than_the_panel() {
    // A row wider than the pane would be wrapped by the console, which
    // tears the panel background apart; the report wraps it itself instead.
    let long = "x".repeat(MIN_WIDTH + 20);
    let mut error = error_with("m", "E0001");
    error.rendered = Some(long.clone());
    let mut report = ErrorReport::new();
    report.record(&error);

    let lines = report_lines(&report);
    assert!(lines.iter().any(|l| l == &"x".repeat(MIN_WIDTH)));
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with(&"x".repeat(20)) && l.trim_end() == "x".repeat(20))
    );
}

#[test]
fn error_report_never_exceeds_the_width_it_is_given() {
    // Nothing may be wider than the pane, or the console wraps it and the
    // panel loses its shape — see the `x` rendering below.
    let mut error = error_with("m", "E0001");
    error.rendered = Some(format!("{}\nshort", "x".repeat(200)));
    let mut report = ErrorReport::new();
    report.record(&error);

    for width in [MIN_WIDTH, 80, 120] {
        for line in report_lines_at(&report, width) {
            assert!(line.chars().count() <= width, "{width}: {line:?}");
        }
    }
}

#[test]
fn error_report_keeps_the_panel_and_the_colors_across_a_wrap() {
    // Both halves of a wrapped row must stay on the panel, and the color
    // cargo opened before the break must be re-armed after it.
    let mut error = error_with("m", "E0001");
    error.rendered = Some(format!("\x1b[31m{}\x1b[0m", "x".repeat(MIN_WIDTH + 10)));
    let mut report = ErrorReport::new();
    report.record(&error);

    let (r, g, b) = PANEL_BACKGROUND;
    let background = format!("\x1b[48;2;{r};{g};{b}m");
    let wrapped: Vec<String> = report
        .lines_with_width(MIN_WIDTH)
        .into_iter()
        .filter(|l| strip_ansi(l).trim_end().starts_with('x'))
        .collect();

    assert_eq!(wrapped.len(), 2, "{wrapped:?}");
    for line in &wrapped {
        assert!(line.contains(&background), "{line:?}");
        assert!(line.contains("\x1b[31m"), "{line:?}");
    }
}

#[test]
fn error_report_keeps_the_order_the_errors_arrived_in() {
    let mut report = ErrorReport::new();
    report.record(&error_with("first", "E0001"));
    report.record(&error_with("second", "E0002"));
    report.record(&error_with("third", "E0003"));

    let lines = report_lines(&report);
    let position = |needle: &str| {
        lines
            .iter()
            .position(|l| l.contains(needle))
            .expect("error listed")
    };
    assert!(position("first") < position("second"));
    assert!(position("second") < position("third"));
    assert!(lines.last().unwrap().contains("3 errors"));
}

#[test]
fn error_report_opens_with_a_blank_line() {
    // The block must not stick to the build output above it.
    let mut report = ErrorReport::new();
    report.record(&error_with("m", "E0001"));
    assert_eq!(report.lines()[0], "");
}

#[test]
fn error_report_rules_are_all_the_same_width() {
    let mut report = ErrorReport::new();
    report.record(&error_with("a", "E0001"));
    report.record(&error_with("b", "E0002"));

    for line in report_lines(&report)
        .iter()
        .filter(|l| l.starts_with('=') || l.starts_with('~'))
    {
        assert_eq!(line.chars().count(), MIN_WIDTH, "{line:?}");
    }
}

// ── Diagnostics scraped from rendered output ─────────────────────

/// The report [`RenderedDiagnostics`] builds from `output`, one line at a
/// time as a reader thread would feed it.
fn scraped(output: &str) -> ErrorReport {
    let mut scraper = RenderedDiagnostics::new();
    for line in output.lines() {
        scraper.push(line);
    }
    scraper.finish()
}

/// The rendering `cargo build` writes for a type error, as `cargo ndk` and
/// `wasm-pack` let it through: plain text, on stderr, framed by the status
/// lines of the build.
const RENDERED_BUILD: &str = "\
   Compiling demo v0.1.0 (/tmp/demo)
error[E0308]: mismatched types
 --> src/lib.rs:1:21
  |
1 | pub fn f() -> i32 { \"oops\" }
  |               ---   ^^^^^^ expected `i32`, found `&str`
  |

For more information about this error, try `rustc --explain E0308`.
error: could not compile `demo` (lib) due to 1 previous error
";

#[test]
fn rendered_output_without_diagnostics_reports_nothing() {
    assert!(scraped("   Compiling demo v0.1.0\n    Finished in 1s\n").is_empty());
}

#[test]
fn rendered_errors_are_collected_from_plain_output() {
    let report = scraped(RENDERED_BUILD);
    let lines = report_lines(&report);

    assert!(!report.is_empty());
    assert!(lines.iter().any(|l| l.contains("Compile Error")));
    assert!(lines.iter().any(|l| l.contains("mismatched types")));
    assert!(lines.last().unwrap().contains("1 error"));
}

#[test]
fn a_scraped_error_keeps_the_whole_rendering() {
    let report = scraped(RENDERED_BUILD);
    // Wide enough that no row of the rendering is wrapped, so each line can
    // be matched whole.
    let lines = report_lines_at(&report, 200);

    // The carets and the source excerpt are what make a diagnostic
    // readable, so the block replays every line rustc printed.
    assert!(lines.iter().any(|l| l.contains("--> src/lib.rs:1:21")));
    assert!(lines.iter().any(|l| l.contains("expected `i32`, found")));
    // ... and nothing that came before or after it.
    assert!(!lines.iter().any(|l| l.contains("Compiling demo")));
    assert!(!lines.iter().any(|l| l.contains("rustc --explain")));
}

#[test]
fn a_numbered_source_line_does_not_end_a_diagnostic() {
    // `1 | ...` is the one part of a body rustc does not indent, so a naive
    // "indented means continuation" rule would cut every excerpt off.
    let report = scraped(RENDERED_BUILD);
    let lines = report_lines_at(&report, 200);

    assert!(lines.iter().any(|l| l.contains("pub fn f() -> i32")));
    assert!(lines.iter().any(|l| l.contains("^^^^^^")));
}

#[test]
fn the_cargo_summary_is_not_reported_as_an_error_of_its_own() {
    // `could not compile` / `aborting due to` only restate the failure;
    // cargo's JSON output doesn't carry them as diagnostics either.
    let report = scraped(
        "error: aborting due to 2 previous errors\nerror: could not compile `demo` (lib) due to 2 previous errors\n",
    );
    assert!(report.is_empty());
}

#[test]
fn scraped_warnings_are_not_errors() {
    let report = scraped(
        "warning: unused variable: `x`\n --> src/lib.rs:2:9\n  |\n\nerror: expected `;`\n --> src/lib.rs:3:1\n",
    );
    let lines = report_lines(&report);

    assert!(lines.iter().any(|l| l.contains("expected `;`")));
    assert!(!lines.iter().any(|l| l.contains("unused variable")));
    assert!(lines.last().unwrap().contains("1 error"));
}

#[test]
fn every_scraped_error_is_reported_in_order() {
    let report = scraped(
        "error[E0308]: first\n --> src/a.rs:1:1\n\nerror[E0425]: second\n --> src/b.rs:2:2\n\nerror: could not compile `demo` (lib) due to 2 previous errors\n",
    );
    let lines = report_lines(&report);
    let position = |needle: &str| {
        lines
            .iter()
            .position(|l| l.contains(needle))
            .expect("error listed")
    };

    assert!(position("first") < position("second"));
    assert!(lines.last().unwrap().contains("2 errors"));
}

#[test]
fn a_scraped_error_at_the_end_of_the_output_is_not_lost() {
    // Nothing follows the last diagnostic, so it is only complete once the
    // stream ends.
    let report = scraped("error[E0433]: cannot find crate\n --> src/lib.rs:1:5\n");
    assert!(!report.is_empty());
    assert!(
        report_lines(&report)
            .iter()
            .any(|l| l.contains("cannot find crate"))
    );
}

/// The stderr of a real failing `cargo ndk -t arm64-v8a build --lib`, kept
/// verbatim: it is the output this collector exists for, down to the notes
/// `cargo ndk` adds of its own after cargo has given up.
const CARGO_NDK_STDERR: &str = "\
    Building arm64-v8a (aarch64-linux-android)
   Compiling ndkerr v0.1.0 (/private/tmp/ndkerr)
error[E0308]: mismatched types
 --> src/lib.rs:1:21
  |
1 | pub fn f() -> i32 { \"oops\" }
  |               ---   ^^^^^^ expected `i32`, found `&str`
  |               |
  |               expected `i32` because of return type

error[E0425]: cannot find function `missing` in this scope
 --> src/lib.rs:2:14
  |
2 | pub fn g() { missing(); }
  |              ^^^^^^^ not found in this scope

Some errors have detailed explanations: E0308, E0425.
For more information about an error, try `rustc --explain E0308`.
error: could not compile `ndkerr` (lib) due to 2 previous errors
note: If the build failed due to a missing target, you can run this command:
note: 
note:     rustup target install aarch64-linux-android
";

#[test]
fn a_failing_cargo_ndk_build_reports_exactly_its_errors() {
    let report = scraped(CARGO_NDK_STDERR);
    let lines = report_lines_at(&report, 200);

    assert!(lines.iter().any(|l| l.contains("mismatched types")));
    assert!(
        lines
            .iter()
            .any(|l| l.contains("cannot find function `missing`"))
    );
    assert!(lines.last().unwrap().contains("2 errors"));
    // Cargo's trailing hints and cargo ndk's own notes are not errors.
    assert!(!lines.iter().any(|l| l.contains("detailed explanations")));
    assert!(!lines.iter().any(|l| l.contains("rustup target install")));
}

#[test]
fn scraped_errors_survive_the_colors_cargo_paints_them_with() {
    // Cargo colors its rendering when it thinks it writes to a terminal;
    // the level, the code and the message all sit behind escapes then.
    let report = scraped("\x1b[1m\x1b[31merror[E0308]\x1b[0m: mismatched types\n");
    assert!(!report.is_empty());
    assert!(
        report_lines(&report)
            .iter()
            .any(|l| l.contains("mismatched types"))
    );
}
