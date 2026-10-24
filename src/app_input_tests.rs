use super::test_support::*;
use super::*;
use crate::assistant::Assistant;
use ratatui::{Terminal, backend::TestBackend};

#[test]
fn chat_maximizes_and_restores_without_changing_documents_or_prompt() -> anyhow::Result<()> {
    let (_directory, mut app) = application()?;
    let source = app.session.editor.source();
    app.session.assistant.input.insert_str("Keep this draft");
    let toggle = |app: &mut App, area| {
        for (code, modifiers) in [
            (KeyCode::Char('x'), KeyModifiers::CONTROL),
            (KeyCode::Char('u'), KeyModifiers::NONE),
        ] {
            app.handle_event(Event::Key(KeyEvent::new(code, modifiers)), area);
        }
    };

    toggle(&mut app, Rect::new(0, 0, 160, 45));
    assert!(!app.session.chat_maximized);
    app.focus = Focus::Assistant;

    for (width, height) in [(160, 45), (80, 30)] {
        let area = Rect::new(0, 0, width, height);
        let original = app.workspace_layout(area);
        toggle(&mut app, area);
        let expanded = app.workspace_layout(area);

        assert!(expanded.editor.is_empty());
        assert_eq!(
            expanded.assistant,
            original.assistant.union(original.editor)
        );
        assert_eq!(expanded.preview, original.preview);
        assert_eq!(expanded.projects, original.projects);

        let mut terminal = Terminal::new(TestBackend::new(width, height))?;
        terminal.draw(|frame| app.draw(frame))?;
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains(&format!("Maximized · {} u Restore", app.prefix.label())));

        for (small_width, small_height) in [(1, 1), (30, 10)] {
            let mut small_terminal = Terminal::new(TestBackend::new(small_width, small_height))?;
            small_terminal.draw(|frame| app.draw(frame))?;
        }

        toggle(&mut app, area);
        let restored = app.workspace_layout(area);
        assert_eq!(restored.editor, original.editor);
        assert_eq!(restored.assistant, original.assistant);
        assert_eq!(app.session.editor.source(), source);
        assert_eq!(app.session.assistant.input.lines()[0], "Keep this draft");
    }
    Ok(())
}

#[test]
fn chat_toggle_uses_reloaded_prefix_and_cancels_without_typing_action_keys() -> anyhow::Result<()> {
    let (directory, mut app) = application()?;
    app.settings_path = directory.path().join("config.toml");
    Settings {
        resume_folder: Some(directory.path().to_path_buf()),
        keybindings: crate::config::Keybindings {
            prefix: "ctrl+a".into(),
        },
    }
    .save(&app.settings_path)?;
    app.reload_settings()?;
    app.focus = Focus::Assistant;
    let area = Rect::new(0, 0, 160, 45);
    let prefix = Event::Key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL));
    let action = Event::Key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::NONE));

    app.handle_event(prefix.clone(), area);
    app.handle_event(
        Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        area,
    );
    assert!(!app.session.chat_maximized && !app.prefix.armed);

    app.handle_event(prefix.clone(), area);
    app.handle_event(action.clone(), area);
    assert!(app.session.chat_maximized);
    assert_eq!(app.session.assistant.input.lines()[0], "");

    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height))?;
    terminal.draw(|frame| app.draw(frame))?;
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains(&format!("{} u Restore", app.prefix.label())));
    assert!(!text.contains("Alt+Z"));

    app.handle_event(prefix, area);
    app.handle_event(action, area);
    assert!(!app.session.chat_maximized);
    app.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::ALT));
    assert!(!app.session.chat_maximized);
    Ok(())
}

#[test]
fn chat_header_click_toggles_and_editor_focus_restores_split() -> anyhow::Result<()> {
    let (_directory, mut app) = application()?;
    let area = Rect::new(0, 0, 160, 45);

    for expected in [true, false, true] {
        let layout = app.workspace_layout(area);
        let button = Assistant::maximize_button(pane_content(layout.assistant));
        app.handle_event(
            Event::Mouse(event::MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: button.right() - 1,
                row: button.y,
                modifiers: KeyModifiers::NONE,
            }),
            area,
        );

        assert_eq!(app.session.chat_maximized, expected);
        assert!(app.focus == Focus::Assistant);
    }

    app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL));
    assert!(app.focus == Focus::Editor);
    assert!(!app.session.chat_maximized);
    assert!(!app.workspace_layout(area).editor.is_empty());
    Ok(())
}

#[test]
fn login_captures_typing_paste_and_cancel_without_editing_tex() -> anyhow::Result<()> {
    let (_directory, mut app) = application()?;
    let original = app.session.editor.source();
    let area = Rect::new(0, 0, 160, 50);
    app.focus = Focus::Assistant;
    app.session.assistant.input.insert_str("/logi");
    app.handle_event(
        Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        area,
    );
    assert!(app.session.assistant.login_open());
    app.handle_event(
        Event::Paste("text should not reach the editor".into()),
        area,
    );
    app.handle_event(
        Event::Key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE)),
        area,
    );
    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height))?;
    terminal.draw(|frame| app.draw(frame))?;
    app.handle_event(
        Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        area,
    );
    assert!(!app.session.assistant.login_open());
    assert_eq!(app.session.editor.source(), original);
    assert!(!app.session.assistant.is_running());
    Ok(())
}

#[test]
fn typing_q_edits_and_ctrl_q_quits_immediately() -> anyhow::Result<()> {
    let (_directory, mut app) = application()?;
    assert!(!app.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)));
    assert!(app.session.editor.source().starts_with('q'));
    let quit = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL);
    assert!(app.handle_key(quit));
    app.actions = Some(ActionMenu::open());
    assert!(app.handle_event(Event::Key(quit), Rect::new(0, 0, 160, 50)));
    Ok(())
}

#[test]
fn export_rejects_stale_buffers_and_modal_typing_does_not_edit_source() -> anyhow::Result<()> {
    let (_directory, mut app) = application()?;
    app.session.rendered_source = Some(app.session.editor.source());
    app.session.rendered_pdf = Some(app.session.editor.path.with_extension("pdf"));
    app.open_export();
    assert!(app.export.is_some());
    let original = app.session.editor.source();
    app.handle_event(
        Event::Paste("new destination".into()),
        Rect::new(0, 0, 160, 50),
    );
    assert_eq!(app.session.editor.source(), original);
    app.handle_event(
        Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        Rect::new(0, 0, 160, 50),
    );
    app.session.editor.text.insert_str("changed");
    app.open_export();
    assert!(app.export.is_none());
    assert!(app.session.status.contains("Build the current"));
    Ok(())
}

#[test]
fn assistant_focus_keeps_chat_input_out_of_the_document() -> anyhow::Result<()> {
    let (_directory, mut app) = application()?;
    let original = app.session.editor.source();
    app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL));
    app.handle_event(
        Event::Paste("Shorten these bullets".into()),
        Rect::new(0, 0, 160, 50),
    );
    assert_eq!(app.session.editor.source(), original);
    assert_eq!(
        app.session.assistant.input.lines(),
        &["Shorten these bullets"]
    );
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT));
    assert_eq!(app.session.assistant.input.lines().len(), 2);
    app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL));
    assert!(app.focus == Focus::Editor);
    Ok(())
}

#[test]
fn export_rechecks_snapshot_before_writing() -> anyhow::Result<()> {
    let (_directory, mut app) = application()?;
    app.session.rendered_source = Some(app.session.editor.source());
    app.session.rendered_pdf = Some(app.session.editor.path.with_extension("pdf"));
    app.open_export();
    app.session.rendered_source = Some("A different completed build".into());
    app.finish_export();
    assert!(app.export.is_none());
    assert!(app.session.status.contains("PDF changed"));

    Ok(())
}

#[test]
fn narrow_workspace_keeps_chat_below_the_upper_pane() -> anyhow::Result<()> {
    let (_directory, mut app) = application()?;
    for (width, height) in [(80, 24), (90, 42)] {
        let area = Rect::new(0, 0, width, height);
        for focus in [
            Focus::Editor,
            Focus::Assistant,
            Focus::Preview,
            Focus::Projects,
        ] {
            app.focus = focus;
            let layout = WorkspaceLayout::new(area, false, true, focus);
            let upper = match focus {
                Focus::Preview => layout.preview,
                Focus::Projects => layout.projects.union(layout.files),
                _ => layout.editor,
            };
            assert_eq!(upper.bottom(), layout.assistant.y);
            assert_eq!(layout.assistant.bottom(), layout.status.y);
            assert_eq!(layout.assistant.width, area.width);
            let mut terminal = Terminal::new(TestBackend::new(width, height))?;
            terminal.draw(|frame| app.draw(frame))?;
            let text: String = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect();
            assert!(text.contains("texo / chat"));
            assert!(text.contains("Ask Texo"));
            if matches!(focus, Focus::Editor | Focus::Assistant) {
                assert!(text.contains("Original document"));
            }
        }
    }
    Ok(())
}

#[test]
#[ignore = "requires Poppler"]
fn pdf_zoom_mouse_and_keyboard_events_leave_editor_and_other_preview_untouched()
-> anyhow::Result<()> {
    let (_directory, mut app) = application()?;
    let pdf = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/two-pages.pdf");
    let pages = texo::pdf::PdfPages::render(&pdf, &std::sync::atomic::AtomicBool::new(false))?;
    app.session.preview.set_pages(pages)?;
    let area = Rect::new(0, 0, 160, 50);
    let preview = app.workspace_layout(area).preview;
    let source = app.session.editor.source();
    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height))?;
    terminal.draw(|frame| app.draw(frame))?;
    let preparing: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(preparing.contains("Updating…"));

    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while app.session.preview.is_preparing() {
        anyhow::ensure!(
            std::time::Instant::now() < deadline,
            "Preview preparation stalled"
        );
        std::thread::sleep(Duration::from_millis(5));
        terminal.draw(|frame| app.draw(frame))?;
    }
    let prepared: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(!prepared.contains("Updating…"));

    let wheel = |kind, modifiers, column| {
        Event::Mouse(event::MouseEvent {
            kind,
            column,
            row: preview.y + 5,
            modifiers,
        })
    };

    app.handle_event(
        wheel(
            MouseEventKind::ScrollUp,
            KeyModifiers::CONTROL,
            preview.x + 5,
        ),
        area,
    );
    assert_eq!(app.session.preview.zoom_percent(), 110);
    assert_eq!(app.session.proposed_preview.zoom_percent(), 100);
    app.handle_event(
        wheel(
            MouseEventKind::ScrollDown,
            KeyModifiers::NONE,
            preview.x + 5,
        ),
        area,
    );
    assert_eq!(app.session.preview.zoom_percent(), 110);
    app.handle_event(
        wheel(MouseEventKind::ScrollUp, KeyModifiers::CONTROL, 0),
        area,
    );
    assert_eq!(app.session.preview.zoom_percent(), 110);

    app.focus = Focus::Preview;
    for (key, expected) in [('+', 120), ('-', 110), ('0', 100)] {
        app.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Char(key), KeyModifiers::NONE)),
            area,
        );
        assert_eq!(app.session.preview.zoom_percent(), expected);
    }
    assert_eq!(app.session.editor.source(), source);

    app.focus = Focus::Assistant;
    app.handle_event(
        Event::Key(KeyEvent::new(KeyCode::Char('+'), KeyModifiers::NONE)),
        area,
    );
    assert_eq!(app.session.assistant.input.lines().join(""), "+");
    assert_eq!(app.session.preview.zoom_percent(), 100);
    Ok(())
}
