use super::*;
use crate::commands::run::cargo_message::CargoMessage;
use crate::commands::run::utilities::{LogLevel, LogRecord};

// ── strip_ansi ───────────────────────────────────────────────────

#[test]
fn strip_ansi_plain_text() {
    assert_eq!(strip_ansi("hello world"), "hello world");
}

#[test]
fn strip_ansi_empty_string() {
    assert_eq!(strip_ansi(""), "");
}

#[test]
fn strip_ansi_sgr_color_codes() {
    assert_eq!(strip_ansi("\x1b[31mError\x1b[0m"), "Error");
}

#[test]
fn strip_ansi_bold_and_combined() {
    assert_eq!(strip_ansi("\x1b[1;32mOK\x1b[0m"), "OK");
}

#[test]
fn strip_ansi_preserves_newlines_and_tabs() {
    assert_eq!(strip_ansi("line1\nline2\ttab"), "line1\nline2\ttab");
}

#[test]
fn strip_ansi_strips_control_chars() {
    assert_eq!(strip_ansi("a\x07b"), "ab");
}

#[test]
fn strip_ansi_cursor_movement_sequences() {
    assert_eq!(strip_ansi("\x1b[2J\x1b[HReady"), "Ready");
}

#[test]
fn strip_ansi_multiple_sequences() {
    assert_eq!(
        strip_ansi("\x1b[31mred\x1b[0m normal \x1b[34mblue\x1b[0m"),
        "red normal blue"
    );
}

// ── ConsoleType::next ────────────────────────────────────────────

#[test]
fn console_type_next_cycles() {
    assert_eq!(ConsoleType::App.next(), ConsoleType::Build);
    assert_eq!(ConsoleType::Build.next(), ConsoleType::Inspector);
    assert_eq!(ConsoleType::Inspector.next(), ConsoleType::ResourceMonitor);
    assert_eq!(ConsoleType::ResourceMonitor.next(), ConsoleType::App);
}

#[test]
fn console_type_next_full_cycle() {
    let mut pane = ConsoleType::App;
    for _ in 0..4 {
        pane = pane.next();
    }
    assert_eq!(pane, ConsoleType::App);
}

// ── ScrollablePane ───────────────────────────────────────────────

#[test]
fn scrollable_pane_starts_at_zero() {
    let pane = ScrollablePane::new();
    assert_eq!(pane.scroll, 0);
}

#[test]
fn scrollable_pane_scroll_up() {
    let mut pane = ScrollablePane::new();
    pane.scroll_up(5);
    assert_eq!(pane.scroll, 5);
    pane.scroll_up(3);
    assert_eq!(pane.scroll, 8);
}

#[test]
fn scrollable_pane_scroll_down_saturates() {
    let mut pane = ScrollablePane::new();
    pane.scroll_up(5);
    pane.scroll_down(3);
    assert_eq!(pane.scroll, 2);
}

#[test]
fn scrollable_pane_scroll_down_does_not_underflow() {
    let mut pane = ScrollablePane::new();
    pane.scroll_down(10);
    assert_eq!(pane.scroll, 0);
}

#[test]
fn scrollable_pane_reset() {
    let mut pane = ScrollablePane::new();
    pane.scroll_up(100);
    pane.reset();
    assert_eq!(pane.scroll, 0);
}

#[test]
fn scrollable_pane_scroll_up_large_values() {
    let mut pane = ScrollablePane::new();
    pane.scroll_up(u16::MAX);
    assert_eq!(pane.scroll, u16::MAX);
}

// ── Status equality ──────────────────────────────────────────────

#[test]
fn status_eq_variants() {
    assert_eq!(Status::Running, Status::Running);
    assert_eq!(Status::Launching, Status::Launching);
    assert_eq!(Status::Error, Status::Error);
    assert_eq!(Status::Idling, Status::Idling);
}

#[test]
fn status_eq_with_payload() {
    assert_eq!(Status::Compiling(50), Status::Compiling(50));
    assert_eq!(Status::Fetching(10), Status::Fetching(10));
    assert_eq!(Status::Building(75), Status::Building(75));
}

#[test]
fn status_ne_different_variants() {
    assert_ne!(Status::Running, Status::Error);
    assert_ne!(Status::Compiling(0), Status::Fetching(0));
}

#[test]
fn status_ne_different_payload() {
    assert_ne!(Status::Compiling(10), Status::Compiling(90));
}

#[test]
fn status_clone() {
    let s = Status::Compiling(42);
    let s2 = s.clone();
    assert_eq!(s, s2);
}

// ── AppState transitions ─────────────────────────────────────────

#[test]
fn app_state_defaults() {
    let state = AppState::new();
    assert_eq!(state.pane, ConsoleType::App);
    assert_eq!(state.status, Status::Compiling(0));
    assert!(state.build_logs.is_empty());
    assert!(state.app_logs.is_empty());
    assert!(!state.inspector_full_tree);
    assert_eq!(state.inspector_cursor, 0);
}

#[test]
fn app_state_apply_status_error_focuses_build() {
    let mut state = AppState::new();
    state.apply_status(Status::Error);
    assert_eq!(state.pane, ConsoleType::Build);
    assert_eq!(state.status, Status::Error);
}

#[test]
fn app_state_apply_status_running_focuses_app() {
    let mut state = AppState::new();
    state.pane = ConsoleType::Build;
    state.apply_status(Status::Running);
    assert_eq!(state.pane, ConsoleType::App);
    assert_eq!(state.status, Status::Running);
}

#[test]
fn app_state_apply_status_other_keeps_pane() {
    let mut state = AppState::new();
    state.pane = ConsoleType::Inspector;
    state.apply_status(Status::Compiling(50));
    assert_eq!(state.pane, ConsoleType::Inspector);
    assert_eq!(state.status, Status::Compiling(50));
}

#[test]
fn app_state_next_pane_cycles() {
    let mut state = AppState::new();
    state.next_pane();
    assert_eq!(state.pane, ConsoleType::Build);
    state.next_pane();
    assert_eq!(state.pane, ConsoleType::Inspector);
    state.next_pane();
    assert_eq!(state.pane, ConsoleType::ResourceMonitor);
    state.next_pane();
    assert_eq!(state.pane, ConsoleType::App);
}

#[test]
fn app_state_push_build_log_strips_cr() {
    let mut state = AppState::new();
    state.push_build_log("hello\rworld".to_string());
    assert_eq!(state.build_log_text(), "helloworld");
}

#[test]
fn app_state_push_app_log_keeps_the_line_verbatim() {
    // Styling happens in the reader threads, so the event loop must not
    // touch the line it is given.
    let mut state = AppState::new();
    state.push_app_log(StyledLog::plain("[INFO] already styled"));
    assert_eq!(state.app_log_lines(), vec!["[INFO] already styled"]);
}

#[test]
fn app_state_push_app_log_strips_cr() {
    let mut state = AppState::new();
    state.push_app_log(StyledLog::plain("hello\rworld"));
    assert_eq!(state.app_log_lines(), vec!["helloworld"]);
}

// ── The recovered panic block ────────────────────────────────────

/// The report of a widget panic the running app recovered from.
fn panic_report() -> PanicReport {
    let record = LogRecord {
        level: LogLevel::Error,
        message: concat!(
            "Widget `HttpRequestButton` panicked during build: boom\n",
            "\n",
            "at jaime/src/http_request_button.rs:117:67\n",
            "\n",
            "        let panic: Option<i32> = Option::None.unwrap();\n",
            "                                 ^^^^^^^^^^^^^^^^^^^^^",
        )
        .to_string(),
        file: None,
        line: None,
    };
    PanicReport::of(&record).expect("a panic report")
}

#[test]
fn app_state_push_app_panic_shows_the_whole_block() {
    let mut state = AppState::new();
    state.set_app_width(80);
    state.push_app_panic(panic_report());

    let rows = row_texts(state.app_logs.rows());
    assert!(rows.iter().any(|row| row.contains("Panicked: Widget")));
    assert!(rows.iter().any(|row| row.contains("^^^^^^^^^^")));
    assert_eq!(widest_row(state.app_logs.rows()), 80);
}

#[test]
fn app_state_relays_the_panic_block_out_when_the_pane_is_resized() {
    let mut state = AppState::new();
    state.set_app_width(80);
    state.push_app_panic(panic_report());

    state.set_app_width(120);

    assert_eq!(widest_row(state.app_logs.rows()), 120);
}

#[test]
fn app_state_leaves_ordinary_app_lines_unresized() {
    // Only a block depends on the width, so resizing must not touch the
    // pane's ordinary lines.
    let mut state = AppState::new();
    state.push_app_log(StyledLog::plain("[INFO] ready"));

    state.set_app_width(120);

    assert_eq!(state.app_log_lines(), vec!["[INFO] ready"]);
}

#[test]
fn app_state_toggle_log_source_keeps_the_panic_block_intact() {
    // A panic block states where it came from itself, so the toggle that
    // hides the log call location has nothing to hide in it.
    let mut state = AppState::new();
    state.set_app_width(80);
    state.push_app_panic(panic_report());

    let before = state.app_log_lines();
    state.toggle_log_source();

    assert_eq!(state.app_log_lines(), before);
}

// ── Source location toggle ───────────────────────────────────────

#[test]
fn app_state_hides_the_log_source_by_default() {
    assert!(!AppState::new().show_log_source);
}

#[test]
fn app_state_toggle_log_source_flips_the_flag() {
    let mut state = AppState::new();
    state.toggle_log_source();
    assert!(state.show_log_source);
    state.toggle_log_source();
    assert!(!state.show_log_source);
}

#[test]
fn app_state_toggle_log_source_hides_and_restores_the_location() {
    let mut state = AppState::new();
    state.push_app_log(StyledLog::with_location(
        "[INFO] ready",
        " (src/main.rs:12)",
    ));

    assert_eq!(state.app_log_lines(), vec!["[INFO] ready"]);
    state.toggle_log_source();
    assert_eq!(state.app_log_lines(), vec!["[INFO] ready (src/main.rs:12)"]);
    state.toggle_log_source();
    assert_eq!(state.app_log_lines(), vec!["[INFO] ready"]);
}

#[test]
fn app_state_toggle_log_source_leaves_plain_output_alone() {
    let mut state = AppState::new();
    state.push_app_log(StyledLog::plain("raw println output"));

    let shown = state.app_log_lines();
    state.toggle_log_source();
    assert_eq!(state.app_log_lines(), shown);
}

#[test]
fn app_state_toggle_log_source_applies_to_every_line() {
    let mut state = AppState::new();
    for i in 0..3 {
        state.push_app_log(StyledLog::with_location(
            format!("[INFO] {i}"),
            format!(" (src/main.rs:{i})"),
        ));
    }

    assert_eq!(
        state.app_log_lines(),
        vec!["[INFO] 0", "[INFO] 1", "[INFO] 2"]
    );
    state.toggle_log_source();
    assert_eq!(
        state.app_log_lines(),
        vec![
            "[INFO] 0 (src/main.rs:0)",
            "[INFO] 1 (src/main.rs:1)",
            "[INFO] 2 (src/main.rs:2)"
        ]
    );
}

#[test]
fn app_state_app_log_text_follows_the_toggle() {
    let mut state = AppState::new();
    state.push_app_log(StyledLog::with_location("[INFO] a", " (f:1)"));
    state.push_app_log(StyledLog::plain("b"));

    assert_eq!(state.app_log_text(), "[INFO] a\nb");
    state.toggle_log_source();
    assert_eq!(state.app_log_text(), "[INFO] a (f:1)\nb");
}

/// Plain text of the rows a pane hands the renderer.
fn row_texts(rows: &[ratatui::text::Line<'static>]) -> Vec<String> {
    rows.iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect::<String>()
        })
        .collect()
}

#[test]
fn app_state_caches_the_rendered_rows_of_each_pushed_line() {
    let mut state = AppState::new();
    state.push_build_log("\x1b[31mred\x1b[0m".to_string());
    state.push_app_log(StyledLog::plain("hello"));

    assert_eq!(row_texts(state.build_logs.rows()), vec!["red"]);
    assert_eq!(row_texts(state.app_logs.rows()), vec!["hello"]);
}

#[test]
fn app_state_toggle_log_source_updates_the_cached_rows() {
    let mut state = AppState::new();
    state.push_app_log(StyledLog::with_location("[INFO] a", " (f:1)"));

    assert_eq!(row_texts(state.app_logs.rows()), vec!["[INFO] a"]);
    state.toggle_log_source();
    assert_eq!(row_texts(state.app_logs.rows()), vec!["[INFO] a (f:1)"]);
    state.toggle_log_source();
    assert_eq!(row_texts(state.app_logs.rows()), vec!["[INFO] a"]);
}

#[test]
fn app_state_pushes_after_a_toggle_honour_it() {
    let mut state = AppState::new();
    state.toggle_log_source();
    state.push_app_log(StyledLog::with_location("[INFO] a", " (f:1)"));

    assert_eq!(row_texts(state.app_logs.rows()), vec!["[INFO] a (f:1)"]);
}

// ── The compile error report ─────────────────────────────────────

/// A report holding the one error `rendered` describes.
fn report_of(rendered: &str) -> ErrorReport {
    let line = format!(
        r#"{{"reason":"compiler-message","message":{{"level":"error","message":"m","spans":[],"rendered":"{rendered}"}}}}"#
    );
    let mut report = ErrorReport::new();
    match CargoMessage::parse(&line) {
        Some(CargoMessage::Diagnostic(diagnostic)) => report.record(&diagnostic),
        other => panic!("expected a diagnostic, got {other:?}"),
    }
    report
}

/// The width of the widest row a pane hands the renderer.
fn widest_row(rows: &[ratatui::text::Line<'static>]) -> usize {
    row_texts(rows)
        .iter()
        .map(|row| row.chars().count())
        .max()
        .unwrap_or(0)
}

#[test]
fn app_state_lays_the_build_report_out_for_the_pane_width() {
    // A block wider than the pane is wrapped by the renderer, which tears
    // its panels apart, so it is laid out for the pane it lands in.
    let mut state = AppState::new();
    state.set_build_width(80);
    state.push_build_report(report_of("error: mismatched types"));

    let rows = row_texts(state.build_logs.rows());
    assert!(rows.iter().any(|row| row.contains("Compile Error")));
    assert_eq!(widest_row(state.build_logs.rows()), 80);
}

#[test]
fn app_state_lays_the_build_report_out_again_when_the_pane_is_resized() {
    let mut state = AppState::new();
    state.set_build_width(80);
    state.push_build_report(report_of("error: mismatched types"));

    state.set_build_width(120);
    assert_eq!(widest_row(state.build_logs.rows()), 120);

    state.set_build_width(64);
    assert_eq!(widest_row(state.build_logs.rows()), 64);
}

#[test]
fn app_state_leaves_plain_build_lines_alone_when_the_pane_is_resized() {
    let mut state = AppState::new();
    state.push_build_log("a".to_string());
    state.push_build_report(report_of("error: boom"));
    state.set_build_width(90);

    assert_eq!(row_texts(state.build_logs.rows())[0], "a");
}

#[test]
fn app_state_build_log_text_includes_the_report() {
    let mut state = AppState::new();
    state.push_build_log("a".to_string());
    state.push_build_report(report_of("error: boom"));

    assert!(state.build_log_text().starts_with("a\n"));
    assert!(strip_ansi(&state.build_log_text()).contains("error: boom"));
}

#[test]
fn app_state_build_log_text_joins_every_line() {
    let mut state = AppState::new();
    state.push_build_log("a".to_string());
    state.push_build_log("b".to_string());

    assert_eq!(state.build_log_text(), "a\nb");
}

#[test]
fn app_state_push_logs_cap_at_max() {
    let mut state = AppState::new();
    for i in 0..(MAX_LINES + 10) {
        state.push_build_log(format!("line {i}"));
    }
    assert!(state.build_logs.len() <= MAX_LINES);
    // Oldest lines dropped; last line preserved.
    assert_eq!(
        state.build_logs.entries().last().unwrap().render(0),
        format!("line {}", MAX_LINES + 9)
    );
}

#[test]
fn app_state_clear_build_and_app() {
    let mut state = AppState::new();
    state.push_build_log("b".to_string());
    state.push_app_log(StyledLog::plain("a"));
    state.build_pane.scroll_up(5);
    state.app_pane.scroll_up(5);

    state.clear_build();
    assert!(state.build_logs.is_empty());
    assert_eq!(state.build_pane.scroll, 0);

    state.clear_app();
    assert!(state.app_logs.is_empty());
    assert_eq!(state.app_pane.scroll, 0);
}

// ── Selection / yank ─────────────────────────────────────────────

/// Build an `AppState` whose most-recent pane view exposes `logical` lines,
/// so `selected_text` has something to slice.
fn state_with(logical: &[&str]) -> AppState {
    let mut state = AppState::new();
    state.last_view = Some(PaneView {
        x: 0,
        y: 0,
        height: 0,
        visible_rows: Vec::new(),
        logical: logical.iter().map(|s| s.to_string()).collect(),
    });
    state
}

#[test]
fn selection_defaults_off() {
    let state = AppState::new();
    assert!(!state.selection_mode);
    assert!(state.selection.is_none());
    assert!(!state.selecting);
    assert!(state.last_view.is_none());
}

#[test]
fn selection_ordered_normalizes_endpoints() {
    let backwards = Selection {
        anchor: (2, 5),
        cursor: (1, 3),
    };
    assert_eq!(backwards.ordered(), ((1, 3), (2, 5)));
    let forwards = Selection {
        anchor: (0, 0),
        cursor: (0, 4),
    };
    assert_eq!(forwards.ordered(), ((0, 0), (0, 4)));
}

#[test]
fn selected_text_single_line_inclusive() {
    let mut state = state_with(&["hello world"]);
    state.selection = Some(Selection {
        anchor: (0, 0),
        cursor: (0, 4),
    });
    assert_eq!(state.selected_text().as_deref(), Some("hello"));
}

#[test]
fn selected_text_is_order_independent() {
    let mut state = state_with(&["hello world"]);
    state.selection = Some(Selection {
        anchor: (0, 4),
        cursor: (0, 0),
    });
    assert_eq!(state.selected_text().as_deref(), Some("hello"));
}

#[test]
fn selected_text_multi_line() {
    let mut state = state_with(&["foo", "bar", "baz"]);
    // From col 1 of "foo" through col 1 of "baz", whole middle line.
    state.selection = Some(Selection {
        anchor: (0, 1),
        cursor: (2, 1),
    });
    assert_eq!(state.selected_text().as_deref(), Some("oo\nbar\nba"));
}

#[test]
fn selected_text_clamps_column_past_eol() {
    let mut state = state_with(&["ab"]);
    state.selection = Some(Selection {
        anchor: (0, 0),
        cursor: (0, 99),
    });
    assert_eq!(state.selected_text().as_deref(), Some("ab"));
}

#[test]
fn selected_text_none_without_selection() {
    let state = state_with(&["x"]);
    assert_eq!(state.selected_text(), None);
}

#[test]
fn selected_text_none_without_view() {
    let mut state = AppState::new();
    state.selection = Some(Selection {
        anchor: (0, 0),
        cursor: (0, 0),
    });
    assert_eq!(state.selected_text(), None);
}

#[test]
fn clear_selection_resets_state() {
    let mut state = state_with(&["x"]);
    state.selection = Some(Selection {
        anchor: (0, 0),
        cursor: (0, 0),
    });
    state.selecting = true;
    state.clear_selection();
    assert!(state.selection.is_none());
    assert!(!state.selecting);
}
