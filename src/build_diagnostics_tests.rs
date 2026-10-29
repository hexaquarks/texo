use super::{App, Focus, test_support::application};
use ratatui::{Terminal, backend::TestBackend, layout::Rect};

fn fail_build(app: &mut App) {
    app.session.building_source = app.session.editor.source();
    app.report_build_error(
        anyhow::anyhow!("error: .texo-agent-test.tex:1: ! Undefined control sequence."),
        false,
    );
}

#[test]
fn build_error_preserves_focus_and_prepares_unsent_chat_without_erasing_input() -> anyhow::Result<()>
{
    let (_directory, mut app) = application()?;
    app.focus = Focus::Preview;
    fail_build(&mut app);
    assert!(app.focus == Focus::Preview);

    app.session.assistant.input.insert_str("Existing question");
    app.session.assistant.input.select_all();
    app.explain_problem();
    let prompt = app.session.assistant.input.lines().join("\n");
    assert!(prompt.starts_with("Existing question\n\n"));
    assert!(prompt.contains("resume.tex:1"));
    assert!(app.focus == Focus::Assistant);
    assert!(!app.session.assistant.is_running());
    Ok(())
}

#[test]
fn stale_errors_cannot_move_cursor_or_prepare_a_fix() -> anyhow::Result<()> {
    let (_directory, mut app) = application()?;
    fail_build(&mut app);
    app.session.editor.replace_source("Changed source");
    app.focus = Focus::Preview;
    app.go_to_problem();
    app.explain_problem();
    assert!(app.focus == Focus::Preview);
    assert!(app.session.assistant.input.lines()[0].is_empty());
    assert!(app.session.status.contains("Rebuild"));
    Ok(())
}

#[test]
fn problems_render_markers_without_repeating_them_below_the_document() -> anyhow::Result<()> {
    let (_directory, mut app) = application()?;
    fail_build(&mut app);
    let mut terminal = Terminal::new(TestBackend::new(120, 35))?;
    terminal.draw(|frame| app.draw(frame))?;
    let buffer = terminal.backend().buffer();
    let markers = buffer
        .content
        .iter()
        .filter(|cell| cell.symbol() == "!")
        .count();
    assert_eq!(markers, 1);

    let text = buffer
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(text.contains("Problems"));
    assert!(text.contains("resume.tex:1"));
    assert!(text.contains("Ask AI"));

    app.handle_prefix_action(
        ratatui::crossterm::event::KeyCode::Char('E'),
        Rect::new(0, 0, 120, 35),
    );
    assert!(app.session.details_open);
    Ok(())
}

#[test]
fn late_build_failure_keeps_the_snapshot_that_actually_failed() -> anyhow::Result<()> {
    let (_directory, mut app) = application()?;
    app.session.building_source = app.session.editor.source();
    app.session
        .editor
        .replace_source("New source while building");
    app.report_build_error(
        anyhow::anyhow!("error: .texo-agent-test.tex:1: broken"),
        false,
    );
    assert!(!app.diagnostics_current());
    app.recover_resource();
    assert!(app.session.job.is_none());
    Ok(())
}

#[test]
fn problem_navigation_wraps_and_does_not_jump_to_dependency_line_numbers() -> anyhow::Result<()> {
    let (_directory, mut app) = application()?;
    app.session.building_source = app.session.editor.source();
    app.report_build_error(
        anyhow::anyhow!(concat!(
            "error: .texo-agent-test.tex:1: broken command\n",
            "error: package.sty:42: broken package",
        )),
        false,
    );
    app.focus = Focus::Preview;
    app.select_problem(true);
    assert!(app.focus == Focus::Preview);
    app.session.editor.text.select_all();
    app.select_problem(true);
    assert!(app.focus == Focus::Editor);
    assert!(app.session.editor.text.selection_range().is_none());
    assert_eq!(app.session.build_diagnostics.as_ref().unwrap().selected, 0);
    Ok(())
}

#[test]
fn summary_stays_inside_editor_without_shrinking_chat_or_preview() -> anyhow::Result<()> {
    let (_directory, mut app) = application()?;
    let area = Rect::new(0, 0, 160, 45);
    let before = app.workspace_layout(area);
    fail_build(&mut app);
    let after = app.workspace_layout(area);

    assert_eq!(before.preview, after.preview);
    assert_eq!(before.assistant, after.assistant);
    assert_eq!(
        after.editor.intersection(after.diagnostics),
        after.diagnostics
    );
    assert!(!app.editor_text_area(&after).intersects(after.diagnostics));
    Ok(())
}

#[test]
fn hiding_summary_preserves_markers_and_successful_build_cleanup_removes_them() -> anyhow::Result<()>
{
    let (_directory, mut app) = application()?;
    fail_build(&mut app);
    app.toggle_problem_summary();
    let mut terminal = Terminal::new(TestBackend::new(120, 35))?;
    terminal.draw(|frame| app.draw(frame))?;
    assert!(
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .any(|cell| cell.symbol() == "!")
    );
    assert!(
        app.workspace_layout(Rect::new(0, 0, 120, 35))
            .diagnostics
            .is_empty()
    );

    app.clear_diagnostics();
    terminal.draw(|frame| app.draw(frame))?;
    assert!(
        !terminal
            .backend()
            .buffer()
            .content
            .iter()
            .any(|cell| cell.symbol() == "!")
    );
    Ok(())
}

#[test]
fn details_modal_owns_input_clamps_scroll_and_preserves_underlying_layout() -> anyhow::Result<()> {
    use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

    let (_directory, mut app) = application()?;
    let area = Rect::new(0, 0, 120, 35);
    let before = app.workspace_layout(area);
    app.report_error(anyhow::anyhow!("{}", "Diagnostic line\n".repeat(100)));
    assert!(app.diagnostic_details_open());
    assert_eq!(app.workspace_layout(area).preview, before.preview);
    assert!(app.workspace_layout(area).diagnostics.is_empty());
    let source = app.session.editor.source();
    let key = |code| Event::Key(KeyEvent::new(code, KeyModifiers::NONE));

    app.handle_event(key(KeyCode::Char('x')), area);
    app.handle_event(Event::Paste("unexpected edit".into()), area);
    app.handle_event(key(KeyCode::End), area);
    let end = app.session.diagnostic_scroll;
    assert!(end > 0);
    app.handle_event(key(KeyCode::PageDown), area);
    assert_eq!(app.session.diagnostic_scroll, end);
    app.handle_event(key(KeyCode::Home), area);
    assert_eq!(app.session.diagnostic_scroll, 0);
    assert_eq!(app.session.editor.source(), source);

    app.handle_event(key(KeyCode::Esc), area);
    assert!(!app.diagnostic_details_open());
    assert!(app.session.diagnostics.is_some());
    Ok(())
}
