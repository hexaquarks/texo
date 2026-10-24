use super::test_support::*;
use super::*;
use ratatui::{Terminal, backend::TestBackend};

#[test]
fn focus_changes_only_invalidate_preview_placement_when_layout_changes() -> anyhow::Result<()> {
    let (_directory, mut app) = application()?;
    let wide = Rect::new(0, 0, 160, 45);
    let preview_area = app.preview_area(wide);
    for focus in [
        Focus::Editor,
        Focus::Assistant,
        Focus::Preview,
        Focus::Projects,
    ] {
        app.focus = focus;
        assert_eq!(app.preview_area(wide), preview_area);
    }

    let narrow = Rect::new(0, 0, 80, 45);
    app.focus = Focus::Editor;
    assert!(app.preview_area(narrow).is_empty());
    app.focus = Focus::Preview;
    assert!(!app.preview_area(narrow).is_empty());
    Ok(())
}

#[test]
fn sidebar_navigation_and_dividers_match_the_visible_panes() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let first = resume_project::create(directory.path(), "AI engineer")?;
    let second = resume_project::create(directory.path(), "Backend")?;
    let mut app = App::with_workspace(
        Editor::open(first.document())?,
        Preview::default(),
        directory.path().to_path_buf(),
    )?;
    let area = Rect::new(0, 0, 160, 45);
    let layout = app.workspace_layout(area);
    assert_eq!(layout.projects.bottom(), layout.editor.bottom());
    assert_eq!(layout.files.y, layout.assistant.y);

    app.focus = Focus::Projects;
    app.handle_event(
        Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
        area,
    );
    app.session.job = None;
    assert_eq!(app.session.editor.path, second.document());
    app.handle_event(
        Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
        area,
    );
    assert!(app.projects.files_focused);
    app.handle_event(
        Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        area,
    );
    assert!(app.focus == Focus::Editor);

    let before = app.session.editor.source();
    app.handle_event(
        Event::Key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE)),
        area,
    );
    assert!(app.project_dialog.is_none());
    assert_ne!(app.session.editor.source(), before);
    Ok(())
}

#[test]
fn pane_selection_highlights_only_the_selected_border() -> anyhow::Result<()> {
    let (_directory, mut app) = application()?;
    let area = Rect::new(0, 0, 160, 50);
    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height))?;
    let layout = WorkspaceLayout::new(area, false, true, app.focus);
    let panes = [layout.editor, layout.assistant, layout.preview];
    assert_eq!(layout.preview.x, layout.editor.right());
    assert_eq!(layout.assistant.y, layout.editor.bottom());

    for (selected, pane) in panes.iter().enumerate() {
        // Border clicks select the pane without activating its content controls.
        app.handle_event(
            Event::Mouse(event::MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: pane.x,
                row: pane.y,
                modifiers: KeyModifiers::NONE,
            }),
            area,
        );
        terminal.draw(|frame| app.draw(frame))?;
        let buffer = terminal.backend().buffer();
        let border_row = layout.editor.y + 1;
        assert_eq!(
            buffer[(layout.editor.right() - 1, border_row)].symbol(),
            "│"
        );
        assert_eq!(buffer[(layout.preview.x, border_row)].symbol(), "│");

        for (index, pane) in panes.iter().enumerate() {
            let color = if index == selected {
                theme::ACCENT
            } else {
                theme::DIVIDER
            };
            for position in [
                (pane.x, pane.y),
                (pane.right() - 1, pane.y),
                (pane.x, pane.bottom() - 1),
                (pane.right() - 1, pane.bottom() - 1),
            ] {
                assert_eq!(buffer[position].fg, color);
            }
        }
    }

    app.focus = Focus::Editor;
    app.handle_key(KeyEvent::new(KeyCode::F(6), KeyModifiers::NONE));
    terminal.draw(|frame| app.draw(frame))?;
    assert_eq!(
        terminal.backend().buffer()[(layout.assistant.x, layout.assistant.y)].fg,
        theme::ACCENT
    );
    assert_eq!(
        terminal.backend().buffer()[(layout.editor.x, layout.editor.y)].fg,
        theme::DIVIDER
    );
    Ok(())
}

#[test]
fn narrow_layout_shows_each_focused_pane_and_tolerates_tiny_sizes() -> anyhow::Result<()> {
    let (_directory, mut app) = application()?;
    for (width, height) in [(1, 1), (20, 5), (40, 12)] {
        for focus in [
            Focus::Editor,
            Focus::Assistant,
            Focus::Projects,
            Focus::Preview,
        ] {
            app.focus = focus;
            let area = Rect::new(0, 0, width, height);
            let layout = WorkspaceLayout::new(area, false, true, focus);
            let visible = match focus {
                Focus::Editor => layout.editor,
                Focus::Assistant => layout.assistant,
                Focus::Projects => layout.projects,
                Focus::Preview => layout.preview,
            };
            assert_eq!(visible.height, area.height.saturating_sub(1));
            assert_eq!(visible.width, area.width);
            let mut terminal = Terminal::new(TestBackend::new(width, height))?;
            terminal.draw(|frame| app.draw(frame))?;
        }
    }
    app.focus = Focus::Editor;
    for expected in [
        Focus::Assistant,
        Focus::Preview,
        Focus::Projects,
        Focus::Editor,
    ] {
        app.handle_key(KeyEvent::new(KeyCode::F(6), KeyModifiers::NONE));
        assert!(app.focus == expected);
    }
    Ok(())
}

#[test]
fn layout_renders_both_panes_and_handles_small_terminals() -> anyhow::Result<()> {
    let (_directory, mut app) = application()?;
    let mut terminal = Terminal::new(TestBackend::new(100, 30))?;
    terminal.draw(|frame| app.draw(frame))?;
    let buffer = terminal.backend().buffer();
    let text: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
    assert!(text.contains("RESUMES") && text.contains("FILES"));
    assert!(text.contains("texo / chat"));
    assert!(text.contains("Original document"));

    let mut small_terminal = Terminal::new(TestBackend::new(30, 8))?;
    small_terminal.draw(|frame| app.draw(frame))?;
    let text: String = small_terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("Original document"));
    assert!(!text.contains("Resize to at least"));
    Ok(())
}
