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
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(text.contains(&format!("{} u Restore", app.prefix.label())));
        assert!(!text.contains("Maximized"));
        assert!(!text.contains("Split"));

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
    assert!(
        text.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .contains(&format!("{} u Restore", app.prefix.label()))
    );
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
        let button = Assistant::maximize_button(
            pane_content(layout.assistant),
            app.session.chat_maximized,
            &app.prefix.label(),
        );
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
    assert_eq!(app.session.preview.zoom_percent(), 120);
    assert_eq!(app.session.proposed_preview.zoom_percent(), 100);
    app.handle_event(
        wheel(
            MouseEventKind::ScrollDown,
            KeyModifiers::NONE,
            preview.x + 5,
        ),
        area,
    );
    assert_eq!(app.session.preview.zoom_percent(), 120);
    app.handle_event(
        wheel(MouseEventKind::ScrollUp, KeyModifiers::CONTROL, 0),
        area,
    );
    assert_eq!(app.session.preview.zoom_percent(), 120);

    app.focus = Focus::Preview;
    for (key, expected) in [('+', 140), ('-', 120), ('0', 100)] {
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

#[test]
fn clicking_the_composer_right_edge_keeps_typing_and_enter_still_submits_commands()
-> anyhow::Result<()> {
    let (_directory, mut app) = application()?;
    let area = Rect::new(0, 0, 160, 45);
    app.session.assistant.input.insert_str("/settings");
    let source = app.session.editor.source();
    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height))?;
    terminal.draw(|frame| app.draw(frame))?;
    let layout = app.workspace_layout(area);
    let chat = app.assistant_body(pane_content(layout.assistant));
    let input = app.session.assistant.input_area(chat);

    app.handle_event(
        Event::Mouse(event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: input.right() - 2,
            row: input.y + 1,
            modifiers: KeyModifiers::NONE,
        }),
        area,
    );

    assert!(app.settings.is_none());
    assert_eq!(app.session.assistant.input.lines(), &["/settings"]);
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(app.settings.is_some());
    assert_eq!(app.session.editor.source(), source);
    Ok(())
}

#[test]
#[ignore = "requires Poppler"]
fn pdf_drag_captures_motion_until_release_and_excludes_header_and_dialogs() -> anyhow::Result<()> {
    use super::workspace_view::below_header;
    use ratatui::layout::Position;

    let (_directory, mut app) = application()?;
    app.session.preview.set_pages(texo::pdf::PdfPages::render(
        std::path::Path::new("tests/fixtures/two-pages.pdf"),
        &std::sync::atomic::AtomicBool::new(false),
    )?)?;
    let area = Rect::new(0, 0, 160, 50);
    let body = below_header(pane_content(app.preview_area(area)), 1);
    let pointer = Position::new(body.x + 8, body.y + 6);
    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height))?;
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while app.session.preview.is_preparing() {
        terminal.draw(|frame| app.draw(frame))?;
        anyhow::ensure!(
            std::time::Instant::now() < deadline,
            "Preview did not become ready"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    let mouse = |kind, point: Position| {
        Event::Mouse(event::MouseEvent {
            kind,
            column: point.x,
            row: point.y,
            modifiers: KeyModifiers::NONE,
        })
    };

    app.handle_event(
        mouse(MouseEventKind::Down(MouseButton::Left), pointer),
        area,
    );
    assert!(app.focus == Focus::Preview);
    assert!(app.session.preview.drag_to(pointer));
    let source = app.session.editor.source();
    app.handle_event(
        mouse(
            MouseEventKind::Drag(MouseButton::Left),
            Position::new(30, 10),
        ),
        area,
    );
    assert_eq!(app.session.editor.source(), source);
    assert!(app.focus == Focus::Preview);
    app.handle_event(
        mouse(MouseEventKind::Up(MouseButton::Left), Position::new(30, 10)),
        area,
    );
    assert!(!app.session.preview.drag_to(pointer));

    let header = Position::new(body.x + 5, body.y - 1);
    app.handle_event(mouse(MouseEventKind::Down(MouseButton::Left), header), area);
    assert!(!app.session.preview.drag_to(pointer));

    app.handle_event(
        mouse(MouseEventKind::Down(MouseButton::Left), pointer),
        area,
    );
    app.handle_event(Event::Resize(120, 40), area);
    assert!(!app.session.preview.drag_to(pointer));

    app.handle_event(
        mouse(MouseEventKind::Down(MouseButton::Left), pointer),
        area,
    );
    app.handle_event(
        Event::Key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE)),
        area,
    );
    assert!(app.settings.is_some());
    assert!(!app.session.preview.drag_to(pointer));
    Ok(())
}

#[test]
fn only_the_visible_chat_maximize_control_handles_clicks() -> anyhow::Result<()> {
    let (_directory, mut app) = application()?;
    let area = Rect::new(0, 0, 160, 45);

    for prefix in ["ctrl+x", "alt+a"] {
        app.prefix = crate::keybindings::Prefix::new(prefix)?;

        for maximized in [false, true] {
            app.focus = Focus::Assistant;
            app.session.chat_maximized = maximized;
            let layout = app.workspace_layout(area);
            let body = app.assistant_body(pane_content(layout.assistant));
            let button = Assistant::maximize_button(body, maximized, &app.prefix.label());

            // The title, the gap before the shortcut, and pane padding must not toggle.
            for column in [body.x, button.x - 1, button.right(), body.right() - 1] {
                app.handle_event(
                    Event::Mouse(event::MouseEvent {
                        kind: MouseEventKind::Down(MouseButton::Left),
                        column,
                        row: button.y,
                        modifiers: KeyModifiers::NONE,
                    }),
                    area,
                );
                assert_eq!(app.session.chat_maximized, maximized);
            }

            // Both the shortcut keycap and the trailing action text are clickable.
            for column in [button.x, button.right() - 1] {
                app.session.chat_maximized = maximized;
                app.handle_event(
                    Event::Mouse(event::MouseEvent {
                        kind: MouseEventKind::Down(MouseButton::Left),
                        column,
                        row: button.y,
                        modifiers: KeyModifiers::NONE,
                    }),
                    area,
                );
                assert_eq!(app.session.chat_maximized, !maximized);
            }
        }
    }
    Ok(())
}

fn pane_prefix(app: &mut App, character: char, modifiers: KeyModifiers, area: Rect) {
    app.handle_event(
        Event::Key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL)),
        area,
    );
    app.handle_event(
        Event::Key(KeyEvent::new(KeyCode::Char(character), modifiers)),
        area,
    );
}

#[test]
fn directional_prefix_selects_neighbors_without_editing_chat() -> anyhow::Result<()> {
    let (_directory, mut app) = application()?;
    let area = Rect::new(0, 0, 160, 45);
    app.focus = Focus::Assistant;
    app.session.assistant.input.insert_str("Keep this prompt");
    let source = app.session.editor.source();

    for (character, expected) in [
        ('k', Focus::Editor),
        ('j', Focus::Assistant),
        ('l', Focus::Preview),
        ('l', Focus::Preview),
        ('h', Focus::Editor),
        ('h', Focus::Projects),
        ('h', Focus::Projects),
        ('l', Focus::Editor),
        ('j', Focus::Assistant),
    ] {
        pane_prefix(&mut app, character, KeyModifiers::NONE, area);
        assert!(app.focus == expected, "prefix {character}");
    }

    assert_eq!(app.session.assistant.input.lines(), &["Keep this prompt"]);
    assert_eq!(app.session.editor.source(), source);

    app.session.chat_maximized = true;
    pane_prefix(&mut app, 'k', KeyModifiers::NONE, area);
    assert!(app.focus == Focus::Editor);
    assert!(!app.session.chat_maximized);
    Ok(())
}

#[test]
fn directional_resize_grows_chat_in_pixels_and_preserves_focus_and_text() -> anyhow::Result<()> {
    let (_directory, mut app) = application()?;
    let area = Rect::new(0, 0, 160, 45);
    app.focus = Focus::Assistant;
    app.session.assistant.input.insert_str("Keep this prompt");
    let (cell_width, cell_height) = app.session.preview.cell_size_pixels();
    let horizontal_step = super::pane_navigation::PANE_RESIZE_PIXELS.div_ceil(cell_width.max(1));
    let vertical_step = super::pane_navigation::PANE_RESIZE_PIXELS.div_ceil(cell_height.max(1));

    for modifiers in [KeyModifiers::NONE, KeyModifiers::SHIFT] {
        let before = app.workspace_layout(area);
        pane_prefix(&mut app, 'L', modifiers, area);
        let after = app.workspace_layout(area);
        assert_eq!(
            after.assistant.width,
            before.assistant.width + horizontal_step
        );
        assert_eq!(after.preview.width, before.preview.width - horizontal_step);

        let before = after;
        pane_prefix(&mut app, 'H', modifiers, area);
        let after = app.workspace_layout(area);
        assert_eq!(
            after.assistant.width,
            before.assistant.width + horizontal_step
        );
        assert_eq!(
            after.projects.width,
            before.projects.width - horizontal_step
        );

        let before = after;
        pane_prefix(&mut app, 'K', modifiers, area);
        let after = app.workspace_layout(area);
        assert_eq!(
            after.assistant.height,
            before.assistant.height + vertical_step
        );
        assert_eq!(after.editor.height, before.editor.height - vertical_step);

        pane_prefix(&mut app, 'J', modifiers, area);
        assert_eq!(app.workspace_layout(area).assistant, after.assistant);
        assert!(app.focus == Focus::Assistant);
        assert_eq!(app.session.assistant.input.lines(), &["Keep this prompt"]);
    }

    app.focus = Focus::Editor;
    let before = app.workspace_layout(area);
    pane_prefix(&mut app, 'J', KeyModifiers::SHIFT, area);
    assert_eq!(
        app.workspace_layout(area).editor.height,
        before.editor.height + vertical_step
    );
    Ok(())
}

#[test]
fn pane_bindings_respect_hidden_panes_and_small_terminal_limits() -> anyhow::Result<()> {
    let (_directory, mut app) = application()?;
    let area = Rect::new(0, 0, 160, 45);
    app.projects_visible = false;
    app.focus = Focus::Editor;
    pane_prefix(&mut app, 'h', KeyModifiers::NONE, area);
    assert!(app.focus == Focus::Editor);

    app.focus = Focus::Assistant;
    for _ in 0..100 {
        pane_prefix(&mut app, 'L', KeyModifiers::SHIFT, area);
        pane_prefix(&mut app, 'K', KeyModifiers::SHIFT, area);
    }
    let layout = app.workspace_layout(area);
    assert_eq!(layout.preview.width, workspace_view::MIN_PANE_WIDTH);
    assert_eq!(layout.editor.height, workspace_view::MIN_EDITOR_HEIGHT);

    for (width, height) in [(80, 30), (100, 21), (1, 1), (160, 45)] {
        let area = Rect::new(0, 0, width, height);
        for character in ['h', 'j', 'k', 'l', 'H', 'J', 'K', 'L'] {
            pane_prefix(&mut app, character, KeyModifiers::NONE, area);
            let mut terminal = Terminal::new(TestBackend::new(width, height))?;
            terminal.draw(|frame| app.draw(frame))?;
        }
    }
    Ok(())
}

#[test]
fn custom_prefix_controls_panes_from_chat_in_a_narrow_layout() -> anyhow::Result<()> {
    let (_directory, mut app) = application()?;
    let area = Rect::new(0, 0, 80, 30);
    app.prefix = crate::keybindings::Prefix::new("alt+a")?;
    app.focus = Focus::Assistant;
    app.session.assistant.input.insert_str("Keep this prompt");
    let before = app.workspace_layout(area);

    for character in ['K', 'k', 'j'] {
        app.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::ALT)),
            area,
        );
        let modifiers = if character.is_ascii_uppercase() {
            KeyModifiers::SHIFT
        } else {
            KeyModifiers::NONE
        };
        app.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Char(character), modifiers)),
            area,
        );
        if character == 'k' {
            assert!(app.focus == Focus::Editor);
        } else {
            assert!(app.focus == Focus::Assistant);
        }
    }

    assert!(app.workspace_layout(area).assistant.height > before.assistant.height);
    assert_eq!(app.session.assistant.input.lines(), &["Keep this prompt"]);
    Ok(())
}
