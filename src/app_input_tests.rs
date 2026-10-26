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
        ..Settings::default()
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
fn application_actions_bypass_editor_shortcuts_and_use_the_configured_prefix() -> anyhow::Result<()>
{
    let (_directory, mut app) = application()?;
    let area = Rect::new(0, 0, 160, 45);
    let source = app.session.editor.source();
    app.perform_action(Action::FocusAssistant);
    assert!(app.focus == Focus::Assistant);
    app.perform_action(Action::FocusAssistant);
    assert!(app.focus == Focus::Editor);
    app.session.rendered_source = Some(source.clone());
    app.session.rendered_pdf = Some(app.session.editor.path.with_extension("pdf"));
    app.perform_action(Action::Export);
    assert!(app.export.is_some());
    app.export = None;
    app.session.editor.text.insert_str("Saved through prefix ");
    app.prefix = Prefix::new("ctrl+s")?;
    for (code, modifiers) in [
        (KeyCode::Char('s'), KeyModifiers::CONTROL),
        (KeyCode::Char('w'), KeyModifiers::NONE),
    ] {
        app.handle_event(Event::Key(KeyEvent::new(code, modifiers)), area);
    }
    assert!(!app.session.editor.is_modified());
    assert_eq!(
        std::fs::read_to_string(&app.session.editor.path)?,
        app.session.editor.source()
    );
    assert!(app.session.editor.source().ends_with(&source));
    Ok(())
}

#[test]
fn assistant_focus_keeps_chat_input_out_of_the_document() -> anyhow::Result<()> {
    let (_directory, mut app) = application()?;
    let original = app.session.editor.source();
    app.handle_prefix_action(KeyCode::Char('c'), Rect::new(0, 0, 160, 50));
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
    assert!(preparing.contains("Loading PDF…"));
    assert!(!preparing.contains("Preparing PDF"));
    assert!(!preparing.contains("Updating…"));

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
fn resize_moves_dividers_like_tmux_from_either_side() -> anyhow::Result<()> {
    let (_directory, mut app) = application()?;
    let area = Rect::new(0, 0, 160, 80);
    let step = super::pane_navigation::PANE_RESIZE_CELLS;
    app.session.assistant.input.insert_str("Keep this prompt");

    for modifiers in [KeyModifiers::NONE, KeyModifiers::SHIFT] {
        for focus in [Focus::Editor, Focus::Assistant, Focus::Preview] {
            app.focus = focus;
            app.pane_sizes = PaneSizes::default();
            let before = app.workspace_layout(area);

            pane_prefix(&mut app, 'L', modifiers, area);
            let after = app.workspace_layout(area);
            assert_eq!(after.preview.x, before.preview.x + step);
            assert_eq!(after.preview.width, before.preview.width - step);
            assert_eq!(after.projects, before.projects);

            pane_prefix(&mut app, 'H', modifiers, area);
            assert_eq!(app.workspace_layout(area).preview, before.preview);
            assert!(app.focus == focus);
        }

        for focus in [Focus::Editor, Focus::Assistant] {
            app.focus = focus;
            app.pane_sizes = PaneSizes::default();
            let before = app.workspace_layout(area);

            pane_prefix(&mut app, 'J', modifiers, area);
            let after = app.workspace_layout(area);
            assert_eq!(after.editor.height, before.editor.height + step);
            assert_eq!(after.assistant.height, before.assistant.height - step);

            pane_prefix(&mut app, 'K', modifiers, area);
            assert_eq!(app.workspace_layout(area).assistant, before.assistant);
            assert!(app.focus == focus);
        }

        app.focus = Focus::Projects;
        app.pane_sizes = PaneSizes::default();
        let before = app.workspace_layout(area);
        pane_prefix(&mut app, 'H', modifiers, area);
        assert_eq!(
            app.workspace_layout(area).projects.width,
            before.projects.width - step
        );
        pane_prefix(&mut app, 'L', modifiers, area);
        assert_eq!(app.workspace_layout(area).projects, before.projects);
    }

    assert_eq!(app.session.assistant.input.lines(), &["Keep this prompt"]);
    Ok(())
}

#[test]
fn resize_can_repeat_without_prefix_and_typing_ends_the_repeat_sequence() -> anyhow::Result<()> {
    let (_directory, mut app) = application()?;
    let area = Rect::new(0, 0, 160, 45);
    app.focus = Focus::Assistant;
    let before = app.workspace_layout(area);
    pane_prefix(&mut app, 'L', KeyModifiers::SHIFT, area);
    app.handle_event(
        Event::Key(KeyEvent::new(KeyCode::Char('L'), KeyModifiers::SHIFT)),
        area,
    );
    assert_eq!(
        app.workspace_layout(area).preview.width,
        before.preview.width - 2 * super::pane_navigation::PANE_RESIZE_CELLS
    );
    assert_eq!(app.session.assistant.input.lines(), &[""]);

    for character in ['a', 'L'] {
        app.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE)),
            area,
        );
    }
    assert_eq!(app.session.assistant.input.lines(), &["aL"]);
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

#[test]
fn pane_sizes_restore_in_a_new_instance_without_changing_config() -> anyhow::Result<()> {
    let (directory, mut app) = application()?;
    let path = directory.path().join("state/layout.json");
    let config = directory.path().join("config.toml");
    Settings::default().save(&config)?;
    let original_config = std::fs::read(&config)?;
    app.settings_path = config.clone();
    app.restore_layout(path.clone());
    assert!(!path.exists());
    let area = Rect::new(0, 0, 160, 80);

    app.focus = Focus::Projects;
    pane_prefix(&mut app, 'L', KeyModifiers::SHIFT, area);
    app.focus = Focus::Assistant;
    pane_prefix(&mut app, 'L', KeyModifiers::SHIFT, area);
    pane_prefix(&mut app, 'J', KeyModifiers::SHIFT, area);
    let expected = app.workspace_layout(area);
    drop(app);

    let (_other_directory, mut restored) = application()?;
    restored.restore_layout(path);
    let actual = restored.workspace_layout(area);
    assert_eq!(actual.projects, expected.projects);
    assert_eq!(actual.editor, expected.editor);
    assert_eq!(actual.assistant, expected.assistant);
    assert_eq!(actual.preview, expected.preview);
    assert_eq!(std::fs::read(config)?, original_config);
    Ok(())
}

#[test]
fn restored_sizes_fit_smaller_terminals_without_overwriting_saved_dimensions() -> anyhow::Result<()>
{
    let (directory, mut app) = application()?;
    let path = directory.path().join("layout.json");
    let sizes = PaneSizes {
        sidebar: Some(u16::MAX),
        preview: Some(u16::MAX),
        editor: Some(u16::MAX),
    };
    sizes.save(&path)?;
    app.restore_layout(path.clone());

    for (width, height) in [(160, 45), (100, 21), (80, 30), (1, 1)] {
        let area = Rect::new(0, 0, width, height);
        let layout = app.workspace_layout(area);
        for pane in [
            layout.projects,
            layout.files,
            layout.editor,
            layout.assistant,
            layout.preview,
        ] {
            assert!(pane.right() <= area.right());
            assert!(pane.bottom() <= area.bottom());
        }

        if width >= 100 {
            assert!(layout.editor.width >= workspace_view::MIN_PANE_WIDTH);
            assert!(layout.preview.width >= workspace_view::MIN_PANE_WIDTH);
            assert!(layout.editor.height >= workspace_view::MIN_EDITOR_HEIGHT);
            assert!(layout.assistant.height >= workspace_view::MIN_CHAT_HEIGHT);
        }

        let mut terminal = Terminal::new(TestBackend::new(width, height))?;
        terminal.draw(|frame| app.draw(frame))?;
    }

    assert!(PaneSizes::load(&path) == sizes);
    Ok(())
}

#[test]
fn focused_editor_owns_typing_selection_navigation_and_history() -> anyhow::Result<()> {
    let (_directory, mut app) = application()?;
    let area = Rect::new(0, 0, 160, 45);
    app.session.diagnostics = Some("Build log".into());
    let key = |app: &mut App, code, modifiers| {
        app.handle_event(Event::Key(KeyEvent::new(code, modifiers)), area);
    };

    key(&mut app, KeyCode::Char('?'), KeyModifiers::NONE);
    assert!(app.settings.is_none());
    assert!(app.session.editor.source().starts_with('?'));
    key(&mut app, KeyCode::End, KeyModifiers::NONE);
    key(
        &mut app,
        KeyCode::Left,
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    );
    assert!(app.session.editor.text.is_selecting());
    assert!(app.focus == Focus::Editor);

    key(&mut app, KeyCode::Char('a'), KeyModifiers::CONTROL);
    key(&mut app, KeyCode::Char('X'), KeyModifiers::SHIFT);
    assert_eq!(app.session.editor.source(), "X");
    key(&mut app, KeyCode::Char('z'), KeyModifiers::CONTROL);
    key(&mut app, KeyCode::Char('z'), KeyModifiers::CONTROL);
    assert!(app.session.editor.source().starts_with('?'));
    key(
        &mut app,
        KeyCode::Char('z'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    );
    key(&mut app, KeyCode::Char('y'), KeyModifiers::CONTROL);
    assert_eq!(app.session.editor.source(), "X");

    app.prefix = Prefix::new("ctrl+a")?;
    key(
        &mut app,
        KeyCode::Char('A'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    );
    assert!(app.session.editor.text.is_selecting());
    key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert!(!app.session.editor.text.is_selecting());
    assert!(app.session.diagnostics.is_some());

    app.session.editor.replace_source(&"line\n".repeat(100));
    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height))?;
    terminal.draw(|frame| app.draw(frame))?;
    key(&mut app, KeyCode::Home, KeyModifiers::CONTROL);
    key(&mut app, KeyCode::PageDown, KeyModifiers::NONE);
    assert!(app.session.editor.text.cursor().0 > 0);
    assert_eq!(app.session.diagnostic_scroll, 0);
    key(
        &mut app,
        KeyCode::End,
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    );
    assert_eq!(app.session.editor.text.cursor().0, 100);

    let source = app.session.editor.source();
    app.focus = Focus::Assistant;
    key(&mut app, KeyCode::Char('z'), KeyModifiers::CONTROL);
    assert_eq!(app.session.editor.source(), source);
    Ok(())
}

#[test]
fn proposal_review_blocks_editor_commands_and_paste() -> anyhow::Result<()> {
    let (_directory, mut app) = application()?;
    let source = app.session.editor.source();
    app.session.proposal_review = Some(ProposalReview::new(&ProposedEdit {
        source_before: source.clone(),
        source_after: "Proposed".into(),
    }));
    let area = Rect::new(0, 0, 160, 45);

    for event in [
        Event::Key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL)),
        Event::Key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE)),
        Event::Key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL)),
        Event::Paste("Do not insert".into()),
    ] {
        app.handle_event(event, area);
    }

    assert_eq!(app.session.editor.source(), source);
    assert!(!app.session.editor.text.is_selecting());
    app.handle_prefix_action(KeyCode::Char('d'), area);
    assert!(app.session.proposal_review.is_none());
    assert_eq!(app.session.editor.source(), source);
    Ok(())
}

#[test]
fn editor_mouse_drag_requires_an_editor_press_and_releases_outside_the_pane() -> anyhow::Result<()>
{
    let (_directory, mut app) = application()?;
    app.session
        .editor
        .replace_source(&"Résumé 李\ttext\n".repeat(30));
    app.session
        .editor
        .text
        .move_cursor(ratatui_textarea::CursorMove::Top);
    let area = Rect::new(0, 0, 160, 45);
    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height))?;
    terminal.draw(|frame| app.draw(frame))?;
    let editor = pane_content(app.workspace_layout(area).editor);
    let mouse = |kind, column, row| {
        Event::Mouse(event::MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        })
    };
    let column = editor.x + theme::PANEL_PADDING + app.session.editor.text.line_number_width();
    let row = editor.y + 1;
    app.focus = Focus::Assistant;
    let cursor = app.session.editor.text.cursor();
    app.handle_event(mouse(MouseEventKind::ScrollDown, column, row), area);
    app.handle_event(
        mouse(MouseEventKind::Drag(MouseButton::Left), column + 6, row),
        area,
    );
    assert_eq!(app.session.editor.text.cursor(), cursor);
    assert!(!app.session.editor.text.is_selecting());

    app.handle_event(
        mouse(MouseEventKind::Down(MouseButton::Left), column, row),
        area,
    );
    app.handle_event(
        mouse(MouseEventKind::Drag(MouseButton::Left), column + 6, row),
        area,
    );
    assert!(app.session.editor.is_dragging());
    app.handle_event(mouse(MouseEventKind::Up(MouseButton::Left), 0, 0), area);
    assert!(!app.session.editor.is_dragging());
    app.session.editor.text.copy();
    assert_eq!(app.session.editor.text.yank_text(), "Résumé");
    Ok(())
}

#[test]
fn vim_pending_navigation_clears_on_focus_prefix_document_and_review_changes() -> anyhow::Result<()>
{
    use crate::config::{EditingMode, EditorPreferences};
    use ratatui_textarea::CursorMove;

    let (directory, mut app) = application()?;
    app.apply_editor_preferences(EditorPreferences {
        mode: EditingMode::Vim,
        ..EditorPreferences::default()
    });
    app.session.editor.replace_source("First\nSecond\nThird\n");
    let area = Rect::new(0, 0, 160, 45);
    let key = |app: &mut App, code| {
        app.handle_event(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)), area);
    };
    let first = app.session.editor.path.clone();
    app.session.editor.text.move_cursor(CursorMove::Jump(2, 0));
    key(&mut app, KeyCode::Char('g'));
    app.handle_event(Event::FocusLost, area);
    key(&mut app, KeyCode::Char('g'));
    assert_eq!(app.session.editor.text.cursor().0, 2);
    key(&mut app, KeyCode::Esc);

    key(&mut app, KeyCode::Char('g'));
    pane_prefix(&mut app, 'c', KeyModifiers::NONE, area);
    assert!(app.focus == Focus::Assistant);
    key(&mut app, KeyCode::Char('i'));
    assert_eq!(app.session.assistant.input.lines(), &["i"]);
    app.focus = Focus::Editor;
    key(&mut app, KeyCode::Char('g'));
    assert_eq!(app.session.editor.text.cursor().0, 2);

    app.open_document(directory.path().join("second.tex"));
    app.open_document(first);
    key(&mut app, KeyCode::Char('g'));
    assert_eq!(app.session.editor.text.cursor().0, 2);
    key(&mut app, KeyCode::Char('g'));
    assert_eq!(app.session.editor.text.cursor().0, 0);

    key(&mut app, KeyCode::Char('g'));
    let before = app.session.editor.source();
    app.apply_edit(&ProposedEdit {
        source_before: before.clone(),
        source_after: format!("{before}Accepted\n"),
    });
    app.session.editor.text.move_cursor(CursorMove::Jump(2, 0));
    key(&mut app, KeyCode::Char('g'));
    assert_eq!(app.session.editor.text.cursor().0, 2);
    Ok(())
}

#[test]
fn vim_operators_clear_across_resume_switches_and_pending_ai_reviews() -> anyhow::Result<()> {
    use crate::config::{EditingMode, EditorPreferences};
    use ratatui_textarea::CursorMove;

    let (directory, mut app) = application()?;
    app.apply_editor_preferences(EditorPreferences {
        mode: EditingMode::Vim,
        ..EditorPreferences::default()
    });
    let first = app.session.editor.path.clone();
    let original = app.session.editor.source();
    let area = Rect::new(0, 0, 160, 45);
    let key = |app: &mut App, character| {
        app.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE)),
            area,
        );
    };
    key(&mut app, 'd');
    app.open_document(directory.path().join("second.tex"));
    app.open_document(first);
    key(&mut app, 'w');
    assert_eq!(app.session.editor.source(), original);
    assert!(!app.session.editor.text.is_selecting());

    app.session.editor.text.move_cursor(CursorMove::Jump(0, 0));
    key(&mut app, 'c');
    assert!(app.apply_edit(&ProposedEdit {
        source_before: original.clone(),
        source_after: "Accepted résumé\n".into(),
    }));
    key(&mut app, 'w');
    assert_eq!(app.session.editor.source(), "Accepted résumé\n");
    app.session.proposal_review = Some(ProposalReview::new(&ProposedEdit {
        source_before: app.session.editor.source(),
        source_after: "Pending revision\n".into(),
    }));
    for character in ['d', 'd', 'u', 'p', 'i'] {
        key(&mut app, character);
    }
    assert_eq!(app.session.editor.source(), "Accepted résumé\n");
    pane_prefix(&mut app, 'd', KeyModifiers::NONE, area);
    assert!(app.session.proposal_review.is_none());
    key(&mut app, 'u');
    key(&mut app, 'u');
    assert_eq!(app.session.editor.source(), original);
    Ok(())
}

#[test]
fn editor_terminal_cursor_yields_to_dialogs_and_other_panes() -> anyhow::Result<()> {
    use crate::config::{EditingMode, EditorPreferences};
    use ratatui::crossterm::cursor::SetCursorStyle;

    let (_directory, mut app) = application()?;
    app.apply_editor_preferences(EditorPreferences {
        mode: EditingMode::Vim,
        ..EditorPreferences::default()
    });
    let mut terminal = Terminal::new(TestBackend::new(160, 45))?;
    terminal.draw(|frame| app.draw(frame))?;
    assert!(terminal.backend().cursor_visible());
    assert_eq!(app.terminal_cursor_style(), SetCursorStyle::SteadyBlock);

    app.open_settings();
    terminal.draw(|frame| app.draw(frame))?;
    assert!(!terminal.backend().cursor_visible());
    assert_eq!(
        app.terminal_cursor_style(),
        SetCursorStyle::DefaultUserShape
    );
    app.settings = None;
    app.focus = Focus::Assistant;
    terminal.draw(|frame| app.draw(frame))?;
    assert!(!terminal.backend().cursor_visible());
    assert_eq!(
        app.terminal_cursor_style(),
        SetCursorStyle::DefaultUserShape
    );

    app.focus = Focus::Editor;
    app.session.proposal_review = Some(ProposalReview::new(&ProposedEdit {
        source_before: app.session.editor.source(),
        source_after: "Pending proposal".into(),
    }));
    terminal.draw(|frame| app.draw(frame))?;
    assert!(!terminal.backend().cursor_visible());
    assert_eq!(
        app.terminal_cursor_style(),
        SetCursorStyle::DefaultUserShape
    );
    Ok(())
}

#[test]
fn vim_objects_and_jumps_respect_focus_resume_sessions_prefixes_and_ai_review() -> anyhow::Result<()>
{
    use crate::config::{EditingMode, EditorPreferences};
    use ratatui_textarea::CursorMove;

    let (directory, mut app) = application()?;
    app.apply_editor_preferences(EditorPreferences {
        mode: EditingMode::Vim,
        ..EditorPreferences::default()
    });
    let source = "first\nsecond\nthird\nfourth\n";
    app.session.editor.replace_source(source);
    app.session.editor.text.move_cursor(CursorMove::Jump(2, 1));
    let first = app.session.editor.path.clone();
    let area = Rect::new(0, 0, 160, 45);
    let character = |app: &mut App, character, modifiers| {
        app.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Char(character), modifiers)),
            area,
        );
    };
    for key in ['G', 'g', 'g'] {
        character(&mut app, key, KeyModifiers::NONE);
    }
    assert_eq!(app.session.editor.text.cursor(), (0, 0));

    app.open_document(directory.path().join("second.tex"));
    character(&mut app, 'o', KeyModifiers::CONTROL);
    app.open_document(first);
    app.session.proposal_review = Some(ProposalReview::new(&ProposedEdit {
        source_before: source.into(),
        source_after: "Pending proposal".into(),
    }));
    for key in ['c', 'i', 'w'] {
        character(&mut app, key, KeyModifiers::NONE);
    }
    character(&mut app, 'o', KeyModifiers::CONTROL);
    assert_eq!(app.session.editor.source(), source);
    assert_eq!(app.session.editor.text.cursor(), (0, 0));
    pane_prefix(&mut app, 'd', KeyModifiers::NONE, area);
    character(&mut app, 'o', KeyModifiers::CONTROL);
    assert_eq!(app.session.editor.text.cursor(), (3, 0));
    character(&mut app, 'o', KeyModifiers::CONTROL);
    assert_eq!(app.session.editor.text.cursor(), (2, 1));

    character(&mut app, 'c', KeyModifiers::NONE);
    character(&mut app, 'i', KeyModifiers::NONE);
    app.handle_event(Event::FocusLost, area);
    character(&mut app, 'w', KeyModifiers::NONE);
    assert_eq!(app.session.editor.source(), source);
    app.focus = Focus::Assistant;
    for key in ['c', 'i', 'w'] {
        character(&mut app, key, KeyModifiers::NONE);
    }
    assert_eq!(app.session.assistant.input.lines(), &["ciw"]);
    assert_eq!(app.session.editor.source(), source);

    app.focus = Focus::Editor;
    app.prefix = Prefix::new("ctrl+i")?;
    let cursor = app.session.editor.text.cursor();
    app.handle_event(
        Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
        area,
    );
    assert!(app.prefix.armed);
    assert_eq!(app.session.editor.text.cursor(), cursor);
    character(&mut app, 'c', KeyModifiers::NONE);
    assert!(app.focus == Focus::Assistant);
    Ok(())
}

#[test]
fn vim_line_selection_and_pending_commands_remain_scoped_to_the_resume() -> anyhow::Result<()> {
    use crate::config::{EditingMode, EditorPreferences};
    use ratatui_textarea::CursorMove;

    let (directory, mut app) = application()?;
    app.apply_editor_preferences(EditorPreferences {
        mode: EditingMode::Vim,
        ..EditorPreferences::default()
    });
    let source = "first\nsecond\nthird\nfourth\n";
    app.session.editor.replace_source(source);
    app.session.editor.text.move_cursor(CursorMove::Jump(1, 3));
    let first = app.session.editor.path.clone();
    let area = Rect::new(0, 0, 100, 30);
    let key = |app: &mut App, character| {
        app.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE)),
            area,
        );
    };
    key(&mut app, 'V');
    key(&mut app, 'k');
    let selection = app.session.editor.text.selection_range();

    app.open_document(directory.path().join("second.tex"));
    app.open_document(first);
    assert_eq!(app.session.editor.text.selection_range(), selection);
    key(&mut app, 'd');
    assert_eq!(app.session.editor.source(), "third\nfourth\n");
    key(&mut app, 'u');
    assert_eq!(app.session.editor.source(), source);

    for character in ['2', 'd', 'f'] {
        key(&mut app, character);
    }
    app.handle_event(Event::FocusLost, area);
    key(&mut app, 'l');
    assert_eq!(app.session.editor.source(), source);
    assert!(!app.session.editor.text.is_selecting());

    app.session.proposal_review = Some(ProposalReview::new(&ProposedEdit {
        source_before: source.into(),
        source_after: "Pending AI proposal".into(),
    }));
    for character in ['.', 'V', 'k', 'd'] {
        key(&mut app, character);
    }
    assert_eq!(app.session.editor.source(), source);
    assert!(app.session.proposal_review.is_some());

    app.focus = Focus::Assistant;
    key(&mut app, '.');
    assert_eq!(app.session.assistant.input.lines(), &["."]);
    assert_eq!(app.session.editor.source(), source);
    Ok(())
}
