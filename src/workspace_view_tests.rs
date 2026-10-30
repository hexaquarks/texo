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
    assert!(app.projects.history_focused);
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
    let layout = WorkspaceLayout::new(area, true, app.focus);
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
    app.handle_prefix_action(KeyCode::Char('c'), Rect::new(0, 0, 160, 45));
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
            let layout = WorkspaceLayout::new(area, true, focus);
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
        app.handle_prefix_action(KeyCode::Char('c'), Rect::new(0, 0, 160, 45));
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
    assert!(text.contains("RESUMES") && text.contains("History"));
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

#[test]
#[ignore = "requires Poppler (pdfinfo and pdftoppm) on PATH"]
fn settings_do_not_consume_the_first_kitty_upload() -> anyhow::Result<()> {
    use ratatui::{backend::Backend, buffer::Buffer};
    use ratatui_image::picker::{Picker, ProtocolType};
    use std::sync::atomic::AtomicBool;

    const COVERED_FRAME_DURATION: Duration = Duration::from_secs(2);
    const PREVIEW_READY_TIMEOUT: Duration = Duration::from_secs(5);
    const WORKER_POLL_INTERVAL: Duration = Duration::from_millis(5);
    const KITTY_IMAGE_PLACEHOLDER: char = '\u{10eeee}';

    let (_directory, mut app) = application()?;
    let mut picker = Picker::halfblocks();
    picker.set_protocol_type(ProtocolType::Kitty);
    app.session.preview = Preview::with_picker(picker);
    app.session.preview.set_pages(texo::pdf::PdfPages::render(
        std::path::Path::new("tests/fixtures/two-pages.pdf"),
        &AtomicBool::new(false),
    )?)?;
    app.focus = Focus::Preview;
    app.open_settings();

    // This small screen is fully covered by settings. Give the background image
    // worker time to finish while the dialog owns the frame.
    let mut terminal = Terminal::new(TestBackend::new(40, 16))?;
    let covered_until = std::time::Instant::now() + COVERED_FRAME_DURATION;
    while std::time::Instant::now() < covered_until {
        terminal.draw(|frame| app.draw(frame))?;
        std::thread::sleep(WORKER_POLL_INTERVAL);
    }

    app.settings = None;
    let deadline = std::time::Instant::now() + PREVIEW_READY_TIMEOUT;
    loop {
        terminal.draw(|frame| app.draw(frame))?;
        let frame: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();

        if frame.contains(KITTY_IMAGE_PLACEHOLDER) {
            let buffer = terminal.backend().buffer();
            let blank = Buffer::empty(buffer.area);
            let mut bytes = Vec::new();
            ratatui::backend::CrosstermBackend::new(&mut bytes)
                .draw(blank.diff(buffer).into_iter())?;
            let wire = String::from_utf8(bytes)?;

            assert!(
                wire.contains("a=T"),
                "The first visible image must upload its pixels, not only placeholders"
            );
            break;
        }

        anyhow::ensure!(
            std::time::Instant::now() < deadline,
            "Preview did not appear"
        );
        std::thread::sleep(WORKER_POLL_INTERVAL);
    }

    Ok(())
}

#[test]
fn focused_pdf_keeps_zoom_and_fit_hints_in_split_and_full_width_layouts() -> anyhow::Result<()> {
    let (_directory, mut app) = application()?;
    app.focus = Focus::Preview;

    for width in [80, 120, 160, 240] {
        let mut terminal = Terminal::new(TestBackend::new(width, 45))?;
        let area = Rect::new(0, 0, width, 45);
        let header = pane_content(app.preview_area(area));
        terminal.draw(|frame| app.draw(frame))?;
        let text: String = (header.x..header.right())
            .map(|x| terminal.backend().buffer()[(x, header.y)].symbol())
            .collect();

        assert!(
            text.contains("+/-"),
            "Missing zoom controls at {width}: {text}"
        );
        assert!(
            text.contains("fit"),
            "Missing fit control at {width}: {text}"
        );
        assert!(
            text.contains("100%"),
            "Missing zoom level at {width}: {text}"
        );
    }
    Ok(())
}

#[test]
fn review_keeps_operation_errors_visible_and_prefix_help_accessible() -> anyhow::Result<()> {
    let (_directory, mut app) = application()?;
    app.session.proposal_review = Some(ProposalReview::new(&ProposedEdit {
        source_before: app.session.editor.source(),
        source_after: "Pending draft".into(),
    }));
    app.session.status = "Accept or discard the proposal before exporting.".into();
    let mut terminal = Terminal::new(TestBackend::new(160, 45))?;

    terminal.draw(|frame| app.draw(frame))?;
    let screen: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(screen.contains("Unapproved changes"));
    assert!(screen.contains(&app.session.status));

    app.prefix.armed = true;
    terminal.draw(|frame| app.draw(frame))?;
    let screen: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(screen.contains("Shortcuts"));
    assert!(!screen.contains("Select pane"));
    assert!(screen.contains(&app.session.status));
    Ok(())
}
