use super::*;

fn device() -> Device {
    Device {
        name: "macOS Desktop".to_string(),
        target: Targets::Macos,
        id: "local".to_string(),
    }
}

#[test]
fn collapsed_successful_stages_hide_details() {
    let mut session = InlineSession::new(&device());
    let id = session.stages.start(StageKind::Compile, Instant::now());
    session
        .stages
        .append_detail(id, "cargo check output")
        .unwrap();
    session.stages.finish(id, Instant::now()).unwrap();

    colored::control::set_override(true);
    let lines = render_lines(&session, 120);
    let expected_label = "Compile".bright_cyan().bold().to_string();
    colored::control::unset_override();

    assert!(
        lines
            .iter()
            .any(|line| strip_ansi(line).contains("✓ Compile"))
    );
    assert!(lines.iter().any(|line| line.contains(&expected_label)));
    assert!(!lines.iter().any(|line| line.contains("cargo check output")));
}

#[test]
fn enter_expansion_reveals_retained_stage_details() {
    let mut session = InlineSession::new(&device());
    let id = session.stages.start(StageKind::Compile, Instant::now());
    session
        .stages
        .append_detail(id, "cargo check output")
        .unwrap();
    session.stages.finish(id, Instant::now()).unwrap();
    session.stages.toggle_selected();

    let lines = render_lines(&session, 120);
    assert!(lines.iter().any(|line| line.contains("cargo check output")));
}

#[test]
fn failed_stage_is_visible_and_expanded() {
    let mut session = InlineSession::new(&device());
    let id = session.stages.start(StageKind::Compile, Instant::now());
    session.stages.append_detail(id, "error: failed").unwrap();
    session.stages.fail(id, Instant::now()).unwrap();

    let lines = render_lines(&session, 120);
    assert!(
        lines
            .iter()
            .any(|line| strip_ansi(line).contains("✗ Compile"))
    );
    assert!(lines.iter().any(|line| line.contains("error: failed")));
}

#[test]
fn status_transitions_create_the_expected_execution_stages() {
    let mut session = InlineSession::new(&device());
    session.apply_status(Status::Compiling(10));
    session.apply_status(Status::Building(50));
    session.apply_status(Status::Launching);
    session.apply_status(Status::Running);

    let labels: Vec<String> = session
        .stages
        .stages()
        .iter()
        .map(|stage| stage.kind().label().to_owned())
        .collect();
    assert_eq!(labels, ["Compile", "Assemble", "Launch", "Application"]);
    assert_eq!(session.stages.active(), session.stages.selected());
}

#[test]
fn application_log_details_are_visible_without_expansion() {
    let mut session = InlineSession::new(&device());
    session.apply_status(Status::Running);
    session.apply_event(RunnerEvent::AppLog("window ready".into()));

    let mut output = Vec::new();
    let mut renderer = InlineRenderer::new(&mut output);
    renderer.render(&session, 120).unwrap();

    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("window ready"));
    assert!(output.contains("\r\x1b[2Kwindow ready\n"));
    assert!(!output.contains("\r\x1b[2K    window ready\n"));
}

#[test]
fn build_lock_wait_is_rendered_until_compile_progress_arrives() {
    let mut session = InlineSession::new(&device());
    session.apply_event(RunnerEvent::BuildLog(
        "Blocking waiting for file lock on build directory".into(),
    ));

    let mut output = Vec::new();
    colored::control::set_override(true);
    {
        let mut renderer = InlineRenderer::new(&mut output);
        renderer.render(&session, 120).unwrap();
    }
    let first_frame_end = output.len();

    session.apply_status(Status::Compiling(1));
    {
        let mut renderer = InlineRenderer::new(&mut output);
        renderer.render(&session, 120).unwrap();
    }

    let expected_status = "Blocking waiting for file lock on build directory"
        .yellow()
        .bold()
        .to_string();
    colored::control::unset_override();
    let first_frame_raw = String::from_utf8_lossy(&output[..first_frame_end]);
    let first_frame = strip_ansi(&String::from_utf8_lossy(&output[..first_frame_end]));
    let second_frame = strip_ansi(&String::from_utf8_lossy(&output[first_frame_end..]));
    assert!(first_frame.contains("⠋ Blocking waiting for file lock on build directory"));
    assert!(first_frame_raw.contains(&expected_status));
    assert!(second_frame.contains("⠋ Compile"));
    assert!(!second_frame.contains("Blocking waiting for file lock"));
}

#[test]
fn active_stage_status_uses_a_spinner_and_only_stage_timing() {
    let mut session = InlineSession::new(&device());
    session.apply_status(Status::Compiling(10));

    let mut output = Vec::new();
    let mut renderer = InlineRenderer::new(&mut output);
    colored::control::set_override(true);
    renderer.render(&session, 120).unwrap();

    let output = String::from_utf8(output).unwrap();
    let expected_label = "Compile".bright_cyan().bold().to_string();
    let expected_navbar = "  ↑/↓ select · r restart · ⏎ expand · ⇧Q quit"
        .dimmed()
        .to_string();
    colored::control::unset_override();

    assert!(output.contains(&format!("\r\x1b[2K  ⠋ {}", expected_label)));
    assert!(!output.contains("▸ □ Compile"));
    assert!(!output.contains("lines"));
    assert!(!output.contains(" · compiling"));
    assert!(output.contains(&expected_navbar));
}

#[test]
fn application_log_is_separated_from_the_live_status_row() {
    let mut session = InlineSession::new(&device());
    session.apply_status(Status::Running);
    session.apply_event(RunnerEvent::AppLog("Tab pressed".into()));
    session.apply_event(RunnerEvent::AppLog("Current route: Home".into()));

    let mut output = Vec::new();
    let mut renderer = InlineRenderer::new(&mut output);
    renderer.render(&session, 80).unwrap();
    renderer.render(&session, 80).unwrap();

    let output = String::from_utf8(output).unwrap();
    let output_without_style = strip_ansi(&output);
    let separator = "─".repeat(80);
    let separator_at = output_without_style
        .find(&separator)
        .expect("application separator");
    assert_eq!(output_without_style.matches(&separator).count(), 2);
    assert!(separator_at > output_without_style.find("Current route: Home").unwrap());
    assert!(separator_at < output_without_style.find("⠋ Application").unwrap());
}

#[test]
fn hot_reload_events_are_hidden_when_hot_reload_is_disabled() {
    let mut session = InlineSession::new(&device());
    session.apply_event(RunnerEvent::HotReload);

    assert!(session.stages.stages().is_empty());
}

#[test]
fn hot_reload_control_is_hidden_when_hot_reload_is_disabled() {
    let session = InlineSession::new(&device());
    let mut output = Vec::new();
    let mut renderer = InlineRenderer::new(&mut output);
    renderer.render(&session, 120).unwrap();

    let output = String::from_utf8(output).unwrap();
    assert!(!output.contains("hot-reload"));
    assert!(output.contains("r restart"));
}

#[test]
fn hot_reload_control_is_visible_when_hot_reload_is_enabled() {
    let session = InlineSession::new_with_hot_reload(&device(), true);
    let mut output = Vec::new();
    let mut renderer = InlineRenderer::new(&mut output);
    renderer.render(&session, 120).unwrap();

    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("^R hot-reload"));
}

#[test]
fn hot_reload_events_are_retained_when_hot_reload_is_enabled() {
    let mut session = InlineSession::new_with_hot_reload(&device(), true);
    session.apply_event(RunnerEvent::HotReload);

    assert!(
        session
            .stages
            .stages()
            .iter()
            .any(|stage| stage.kind() == &StageKind::HotReload)
    );
}

#[test]
fn repeated_frames_update_the_live_block_without_rewriting_the_transcript() {
    let session = InlineSession::new(&device());
    let mut output = Vec::new();
    let mut renderer = InlineRenderer::new(&mut output);

    renderer.render(&session, 120).unwrap();
    renderer.render(&session, 120).unwrap();

    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("\x1b[1A"));
    assert!(!output.contains("\x1b[2J"));
    assert_eq!(output.matches("◆ Aimer").count(), 1);
    assert!(output.matches("\r\x1b[2K").count() >= 2);
}

#[test]
fn narrowing_the_terminal_clears_reflowed_live_rows_before_redrawing() {
    let session = InlineSession::new(&device());
    let mut output = Vec::new();
    let mut renderer = InlineRenderer::new(&mut output);

    renderer.render(&session, 120).unwrap();
    renderer.render(&session, 40).unwrap();

    let output = String::from_utf8(output).unwrap();
    assert_eq!(output.matches("\x1b[1A").count(), 2);
}

#[test]
fn a_hot_restart_starts_the_new_transcript_after_a_separator() {
    let mut session = InlineSession::new(&device());
    session.apply_status(Status::Compiling(10));
    let mut output = Vec::new();
    let mut renderer = InlineRenderer::new(&mut output);

    renderer.render(&session, 80).unwrap();
    session.reset_for_restart();
    renderer.reset();
    renderer.render(&session, 80).unwrap();

    let output = String::from_utf8(output).unwrap();
    let output_without_style = strip_ansi(&output);
    let separator = "─".repeat(80);
    let separator_at = output_without_style
        .find(&separator)
        .expect("restart separator");
    assert_eq!(output_without_style.matches(&separator).count(), 1);
    assert!(separator_at < output_without_style.rfind("status: compiling").unwrap());
}

#[test]
fn separators_are_dimmed() {
    let mut session = InlineSession::new(&device());
    session.apply_status(Status::Running);
    session.apply_event(RunnerEvent::AppLog("window ready".into()));

    let mut output = Vec::new();
    let mut renderer = InlineRenderer::new(&mut output);
    colored::control::set_override(true);
    renderer.render(&session, 80).unwrap();
    session.reset_for_restart();
    renderer.reset();
    renderer.render(&session, 80).unwrap();

    let output = String::from_utf8(output).unwrap();
    let expected_separator = "─".repeat(80).dimmed().to_string();
    colored::control::unset_override();

    assert_eq!(output.matches(&expected_separator).count(), 2);
}

#[test]
fn completed_stage_summary_is_emitted_to_scrollback_once() {
    let mut session = InlineSession::new(&device());
    let id = session.stages.start(StageKind::Compile, Instant::now());
    session.stages.finish(id, Instant::now()).unwrap();

    let mut output = Vec::new();
    let mut renderer = InlineRenderer::new(&mut output);
    renderer.render(&session, 120).unwrap();
    renderer.render(&session, 120).unwrap();

    let output = String::from_utf8(output).unwrap();
    assert_eq!(strip_ansi(&output).matches("✓ Compile").count(), 1);
}

#[test]
fn failed_stage_summary_and_details_are_appended_once() {
    let mut session = InlineSession::new(&device());
    let id = session.stages.start(StageKind::Compile, Instant::now());
    session.stages.append_detail(id, "error: failed").unwrap();
    session.stages.fail(id, Instant::now()).unwrap();

    let mut output = Vec::new();
    let mut renderer = InlineRenderer::new(&mut output);
    renderer.render(&session, 120).unwrap();
    renderer.render(&session, 120).unwrap();

    let output = String::from_utf8(output).unwrap();
    assert_eq!(strip_ansi(&output).matches("✗ Compile").count(), 1);
    assert_eq!(output.matches("error: failed").count(), 1);
}

#[test]
fn notices_are_shown_in_the_live_region_until_they_expire() {
    let mut session = InlineSession::new(&device());
    session.notice("Hot reload requested".to_string());
    let mut output = Vec::new();
    let mut renderer = InlineRenderer::new(&mut output);

    renderer.render(&session, 120).unwrap();
    let shown = String::from_utf8(output.clone()).unwrap();
    assert!(shown.contains("Hot reload requested"));

    let expired = Instant::now() + NOTICE_VISIBLE_FOR;
    assert!(session.active_notice(expired).is_none());
}

#[test]
fn durations_switch_to_minutes_after_one_minute() {
    assert_eq!(format_duration(Duration::from_millis(250)), "250ms");
    assert_eq!(format_duration(Duration::from_millis(1_500)), "1.5s");
    assert_eq!(format_duration(Duration::from_secs(125)), "2m05s");
}

#[test]
fn styled_live_rows_are_truncated_to_the_window_without_losing_their_reset() {
    let line = "  ⠋ \x1b[1;96mCompile\x1b[0m 1.2s".to_string();

    let fitted = truncate_styled(&line, 8);
    assert_eq!(display_width(&fitted), 8);
    assert!(fitted.contains('…'));
    assert!(fitted.ends_with("\x1b[0m"));
    assert_eq!(truncate_styled(&line, 120), line);
}

#[test]
fn wide_characters_count_as_two_columns_when_sizing_rows() {
    assert_eq!(physical_rows("你好你好", 4), 2);
    assert_eq!(physical_rows("abcd", 4), 1);
    assert_eq!(display_width(&truncate_styled("你好你好", 5)), 5);
}

#[test]
fn live_rows_never_wrap_in_a_narrow_window() {
    let mut session = InlineSession::new(&device());
    session.apply_status(Status::Compiling(10));
    let mut output = Vec::new();
    let mut renderer = InlineRenderer::new(&mut output);

    renderer.render(&session, 20).unwrap();

    assert!(renderer.live_lines.iter().all(|line| physical_rows(line, 20) == 1));
}

#[test]
fn clearing_the_live_region_also_erases_everything_below_it() {
    let session = InlineSession::new(&device());
    let mut output = Vec::new();
    let mut renderer = InlineRenderer::new(&mut output);

    renderer.render(&session, 120).unwrap();
    let first_frame_end = renderer.writer.len();
    renderer.render(&session, 60).unwrap();

    let redraw = String::from_utf8_lossy(&renderer.writer[first_frame_end..]).into_owned();
    assert!(redraw.contains("\r\x1b[J"));
}

#[test]
fn build_lock_wait_names_the_cause_and_how_long_it_has_lasted() {
    let mut session = InlineSession::new(&device());
    session.apply_event(RunnerEvent::BuildLog(
        "Blocking waiting for file lock on build directory".into(),
    ));
    let mut output = Vec::new();
    let mut renderer = InlineRenderer::new(&mut output);

    renderer.render(&session, 160).unwrap();

    let frame = strip_ansi(&String::from_utf8(output).unwrap());
    assert!(frame.contains("Blocking waiting for file lock on build directory · "));
    assert!(frame.contains("another cargo process"));
}

fn stage_kinds(session: &InlineSession) -> Vec<StageKind> {
    session
        .stages
        .stages()
        .iter()
        .map(|stage| stage.kind().clone())
        .collect()
}

fn live_frame(session: &InlineSession) -> String {
    let mut output = Vec::new();
    InlineRenderer::new(&mut output)
        .render(session, 120)
        .unwrap();
    strip_ansi(&String::from_utf8(output).unwrap())
}

#[test]
fn cargo_preparation_phases_are_their_own_stages_in_order() {
    let mut session = InlineSession::new(&device());
    session.apply_status(Status::Compiling(0));
    session.apply_status(Status::Locking);
    session.apply_status(Status::Fetching(10));
    session.apply_status(Status::Downloading(0));
    session.apply_status(Status::Downloading(40));
    session.apply_status(Status::Compiling(5));

    // The empty compile stage that opened the build became the first phase.
    assert_eq!(
        stage_kinds(&session),
        vec![
            StageKind::Locking,
            StageKind::Fetching,
            StageKind::Downloading,
            StageKind::Compile
        ]
    );
    let statuses: Vec<_> = session.stages.stages().iter().map(|s| s.status()).collect();
    assert_eq!(
        statuses,
        vec![
            StageStatus::Succeeded,
            StageStatus::Succeeded,
            StageStatus::Succeeded,
            StageStatus::Running
        ]
    );
}

#[test]
fn a_build_without_preparation_phases_keeps_a_single_compile_stage() {
    let mut session = InlineSession::new(&device());
    session.apply_status(Status::Compiling(0));
    session.apply_status(Status::Compiling(30));
    session.apply_status(Status::Compiling(100));

    assert_eq!(stage_kinds(&session), vec![StageKind::Compile]);
}

#[test]
fn a_compile_that_has_made_progress_is_not_turned_into_a_phase() {
    let mut session = InlineSession::new(&device());
    session.apply_status(Status::Compiling(20));
    session.apply_status(Status::Fetching(1));

    assert_eq!(
        stage_kinds(&session),
        vec![StageKind::Compile, StageKind::Fetching]
    );
}

#[test]
fn build_output_stays_with_the_preparation_phase_it_arrived_in() {
    let mut session = InlineSession::new(&device());
    session.apply_status(Status::Compiling(0));
    session.apply_status(Status::Downloading(0));
    session.apply_event(RunnerEvent::BuildLog("Downloaded serde v1.0".into()));

    assert_eq!(stage_kinds(&session), vec![StageKind::Downloading]);
    let stage = &session.stages.stages()[0];
    assert!(
        stage
            .details()
            .iter()
            .any(|detail| detail.as_str().contains("Downloaded serde"))
    );
}

#[test]
fn a_running_stage_shows_its_percentage_instead_of_its_duration() {
    let mut session = InlineSession::new(&device());
    session.apply_status(Status::Downloading(40));

    let frame = live_frame(&session);
    assert!(frame.contains("Downloading"));
    assert!(frame.contains("40%"));
    assert!(!frame.contains("ms"), "no duration while progress is known: {frame}");
}

#[test]
fn a_stage_without_a_percentage_falls_back_to_its_duration() {
    let mut session = InlineSession::new(&device());
    session.apply_status(Status::Locking);

    let frame = live_frame(&session);
    assert!(frame.contains("Locking"));
    assert!(frame.contains("ms") || frame.contains('s'));
    assert!(!frame.contains('%'));
}

#[test]
fn a_finished_stage_reports_its_duration_not_its_percentage() {
    let mut session = InlineSession::new(&device());
    session.apply_status(Status::Downloading(100));
    session.apply_status(Status::Compiling(1));

    let frame = live_frame(&session);
    let summary = frame
        .lines()
        .find(|line| line.contains("✓ Downloading"))
        .expect("the finished phase is committed to scrollback");
    assert!(!summary.contains('%'), "{summary}");
    assert!(summary.contains("ms") || summary.contains('s'), "{summary}");
    assert!(frame.contains("Compile") && frame.contains("1%"));
}
