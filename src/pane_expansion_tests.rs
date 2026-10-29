use super::test_support::application;
use super::*;
use ratatui::{Terminal, backend::TestBackend};

fn prefix(app: &mut App, character: char, area: Rect) {
    app.handle_event(
        Event::Key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL)),
        area,
    );
    app.handle_event(
        Event::Key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE)),
        area,
    );
}

#[test]
fn workspace_expansion_round_trips_each_pane_without_changing_dividers() -> anyhow::Result<()> {
    let (_directory, mut app) = application()?;
    app.pane_sizes.sidebar = Some(30);
    app.pane_sizes.preview = Some(48);
    app.pane_sizes.editor = Some(18);
    let sizes = app.pane_sizes;

    for visible in [true, false] {
        app.projects_visible = visible;
        for focus in [Focus::Editor, Focus::Assistant, Focus::Preview] {
            for (width, height) in [(160, 45), (80, 30), (30, 10), (1, 1)] {
                let area = Rect::new(0, 0, width, height);
                app.focus = focus;
                let original = app.workspace_layout(area);
                prefix(&mut app, 'z', area);
                let expanded = app.workspace_layout(area);

                assert_eq!(expanded.projects, original.projects);
                assert_eq!(expanded.files, original.files);
                assert_eq!(expanded.status, original.status);
                let original_content = [original.editor, original.assistant, original.preview]
                    .into_iter()
                    .filter(|pane| !pane.is_empty())
                    .reduce(|area, pane| area.union(pane))
                    .unwrap_or_default();
                let selected = match focus {
                    Focus::Editor => expanded.editor,
                    Focus::Assistant => expanded.assistant,
                    Focus::Preview => expanded.preview,
                    Focus::Projects => unreachable!(),
                };
                assert_eq!(selected, original_content);

                let content = [expanded.editor, expanded.assistant, expanded.preview];
                assert!(content.iter().filter(|pane| !pane.is_empty()).count() <= 1);
                for pane in content {
                    assert!(pane.right() <= area.right());
                    assert!(pane.bottom() <= expanded.status.y);
                }

                let mut terminal = Terminal::new(TestBackend::new(width, height))?;
                terminal.draw(|frame| app.draw(frame))?;
                prefix(&mut app, 'z', area);
                let restored = app.workspace_layout(area);
                assert_eq!(restored.editor, original.editor);
                assert_eq!(restored.assistant, original.assistant);
                assert_eq!(restored.preview, original.preview);
                assert!(app.pane_sizes == sizes);
            }
        }
    }
    Ok(())
}

#[test]
fn workspace_expansion_restores_column_state_and_build_reveals_pdf() -> anyhow::Result<()> {
    let (_directory, mut app) = application()?;
    let area = Rect::new(0, 0, 160, 45);
    prefix(&mut app, 'u', area);
    let column = app.workspace_layout(area);
    prefix(&mut app, 'z', area);
    assert!(app.workspace_layout(area).preview.is_empty());
    prefix(&mut app, 'z', area);
    assert_eq!(app.workspace_layout(area).editor, column.editor);

    prefix(&mut app, 'z', area);
    // Exercise restoration before build validation without launching a compiler.
    app.session.empty_workspace = true;
    prefix(&mut app, 'b', area);
    assert!(app.workspace_expansion.is_none());
    assert!(app.session.column_expansion == Some(Focus::Editor));
    assert_eq!(app.workspace_layout(area).preview, column.preview);
    Ok(())
}

#[test]
fn navigation_and_review_actions_reveal_adjacent_panes() -> anyhow::Result<()> {
    let (_directory, mut app) = application()?;
    let area = Rect::new(0, 0, 160, 45);
    prefix(&mut app, 'u', area);
    prefix(&mut app, 'z', area);
    prefix(&mut app, 'j', area);
    assert!(app.focus == Focus::Assistant);
    assert!(app.workspace_expansion.is_none());
    assert!(app.session.column_expansion.is_none());

    prefix(&mut app, 'z', area);
    prefix(&mut app, 'r', area);
    assert!(app.focus == Focus::Projects);
    assert!(app.workspace_expansion.is_none());

    app.focus = Focus::Editor;
    prefix(&mut app, 'u', area);
    prefix(&mut app, 'z', area);
    app.perform_action(Action::ReviewEdit);
    assert!(app.workspace_expansion.is_none());
    assert!(app.session.column_expansion.is_none());
    Ok(())
}

#[test]
fn details_overlay_preserves_expansion_until_error_navigation() -> anyhow::Result<()> {
    let (_directory, mut app) = application()?;
    let area = Rect::new(0, 0, 160, 45);
    app.session.diagnostics = Some("Compiler details".into());
    app.session.summary_hidden = true;
    prefix(&mut app, 'z', area);
    prefix(&mut app, 'E', area);
    assert!(app.diagnostic_details_open());
    assert!(app.workspace_expansion == Some(Focus::Editor));
    app.handle_event(
        Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        area,
    );
    assert!(!app.diagnostic_details_open());
    assert!(app.workspace_expansion == Some(Focus::Editor));
    app.go_to_problem();
    assert!(app.workspace_expansion.is_none());
    Ok(())
}

#[test]
fn build_from_chat_restores_workspace_and_failure_reveals_source() -> anyhow::Result<()> {
    let (_directory, mut app) = application()?;
    let area = Rect::new(0, 0, 160, 45);
    app.focus = Focus::Assistant;
    prefix(&mut app, 'u', area);
    prefix(&mut app, 'z', area);
    app.session.empty_workspace = true;
    prefix(&mut app, 'b', area);
    assert!(app.workspace_expansion.is_none());
    assert!(app.session.column_expansion == Some(Focus::Assistant));
    assert!(!app.workspace_layout(area).preview.is_empty());

    app.report_build_error(anyhow::anyhow!("Undefined control sequence"), false);
    assert!(app.session.column_expansion.is_none());
    assert!(!app.workspace_layout(area).editor.is_empty());
    assert!(!app.workspace_layout(area).diagnostics.is_empty());
    Ok(())
}
