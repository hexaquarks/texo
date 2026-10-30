use super::test_support::*;
use super::*;
use crate::render_job::RenderJob;
use ratatui::{Terminal, backend::TestBackend};

#[test]
fn reload_shortcut_keeps_settings_open_for_same_new_and_invalid_folders() -> anyhow::Result<()> {
    let (directory, mut app) = application()?;
    app.settings_path = directory.path().join("config.toml");
    let mut settings = Settings {
        resume_folder: Some(directory.path().to_path_buf()),
        ..Settings::default()
    };
    settings.save(&app.settings_path)?;
    app.session.editor.text.insert_str("unsaved text");
    let source = app.session.editor.source();
    app.open_settings();
    let area = Rect::new(0, 0, 160, 45);
    let reload = Event::Key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));

    app.settings.as_mut().unwrap().error = "Previous error".into();
    assert!(!app.handle_event(reload.clone(), area));
    let dialog = app.settings.as_ref().expect("Settings stays open");
    assert_eq!(dialog.notice, "Configuration reloaded.");
    assert!(dialog.error.is_empty());
    assert_eq!(app.session.editor.source(), source);

    let target = directory.path().join("new resumes");
    std::fs::create_dir(&target)?;
    settings.resume_folder = Some(target.clone());
    settings.keybindings.prefix = "ctrl+a".into();
    settings.save(&app.settings_path)?;
    assert!(!app.handle_event(reload.clone(), area));
    assert_eq!(app.session.editor.source(), source);
    assert!(app.settings.as_ref().unwrap().confirmation.is_some());
    app.confirm_folder_change()?;
    app.open_settings();
    let dialog = app.settings.as_ref().unwrap();
    assert_eq!(
        dialog.value(),
        crate::workspace::display_path(&target.canonicalize()?)
    );
    assert_eq!(dialog.prefix_label, Prefix::new("ctrl+a")?.label());
    assert!(dialog.notice.is_empty());

    let saved_value = dialog.value();
    std::fs::write(&app.settings_path, "broken TOML")?;
    assert!(!app.handle_event(reload, area));
    let dialog = app.settings.as_ref().expect("Settings stays open on error");
    assert!(!dialog.error.is_empty());
    assert!(dialog.notice.is_empty());
    assert_eq!(dialog.value(), saved_value);
    assert_eq!(app.projects.root(), target.canonicalize()?);
    Ok(())
}

#[test]
fn open_config_failure_has_a_concise_error_and_preserves_the_dialog() -> anyhow::Result<()> {
    let (directory, mut app) = application()?;
    app.settings_path = directory.path().join("config.toml");
    app.open_settings();
    let error = app
        .open_config_with(|_| anyhow::bail!("Launcher details and exit status"))
        .unwrap_err();

    assert_eq!(
        error.to_string(),
        "Could not open config. Open the path above in a text editor."
    );
    assert!(app.settings.is_some());
    assert!(app.settings_path.is_file());
    Ok(())
}

#[test]
fn config_open_reload_and_folder_apply_preserve_prefix_and_reject_invalid_edits()
-> anyhow::Result<()> {
    let (directory, mut app) = application()?;
    app.settings_path = directory.path().join("config.toml");
    app.open_settings();
    app.open_config_with(|path| {
        assert_eq!(path, app.settings_path);
        assert!(path.is_file());
        Ok(())
    })?;

    let mut settings = Settings::load(&app.settings_path)?;
    settings.resume_folder = Some(directory.path().to_path_buf());
    settings.keybindings.prefix = "ctrl+a".into();
    settings.save(&app.settings_path)?;
    app.reload_settings()?;
    assert_eq!(app.prefix.label(), Prefix::new("ctrl+a")?.label());

    let area = Rect::new(0, 0, 160, 45);
    for (code, modifiers) in [
        (KeyCode::Char('a'), KeyModifiers::CONTROL),
        (KeyCode::Char('s'), KeyModifiers::NONE),
    ] {
        app.handle_event(Event::Key(KeyEvent::new(code, modifiers)), area);
    }
    assert!(app.settings.is_some());

    let target = directory.path().join("new");
    std::fs::create_dir(&target)?;
    app.settings.as_mut().unwrap().input =
        ratatui_textarea::TextArea::new(vec![target.display().to_string()]);
    app.prepare_folder_change()?;
    app.confirm_folder_change()?;
    assert_eq!(
        Settings::load(&app.settings_path)?.keybindings.prefix,
        "ctrl+a"
    );
    assert_eq!(app.prefix.label(), Prefix::new("ctrl+a")?.label());

    app.session.editor.text.insert_str("unsaved");
    let source = app.session.editor.source();
    app.open_settings();
    std::fs::write(&app.settings_path, "invalid TOML")?;
    assert!(app.reload_settings().is_err());
    assert_eq!(app.session.editor.source(), source);
    assert_eq!(app.projects.root(), target.canonicalize()?);
    assert_eq!(app.prefix.label(), Prefix::new("ctrl+a")?.label());
    // The Open action must still work so a broken file can be repaired.
    app.open_config_with(|_| Ok(()))?;
    Ok(())
}

#[test]
fn settings_switch_discards_pending_state_and_refreshes_an_empty_workspace() -> anyhow::Result<()> {
    let (directory, mut app) = application()?;
    let layout_state_path = directory.path().join("layout.json");
    app.restore_layout(layout_state_path.clone());
    app.pane_sizes = PaneSizes {
        sidebar: Some(30),
        preview: Some(65),
        editor: Some(20),
    };
    let pane_sizes = app.pane_sizes;
    let original = app.session.editor.source();
    app.session.editor.text.insert_str("unsaved");
    app.session.proposal_review = Some(ProposalReview::new(&ProposedEdit {
        source_before: app.session.editor.source(),
        source_after: "pending".into(),
    }));
    app.session.assistant.input.insert_str("old prompt");
    app.session.show_proposed = true;
    app.session.diagnostics = Some("old error".into());
    app.settings_path = directory.path().join("config/settings.json");
    let target = directory.path().join("New resumes");
    std::fs::create_dir(&target)?;
    app.open_settings();
    app.settings.as_mut().unwrap().input =
        ratatui_textarea::TextArea::new(vec![target.display().to_string()]);
    app.prepare_folder_change()?;
    app.confirm_folder_change()?;

    assert!(app.pane_sizes == pane_sizes);
    assert_eq!(app.layout_state_path, Some(layout_state_path));
    assert_eq!(app.projects.root(), target.canonicalize()?);
    assert_eq!(
        app.session.editor.path,
        target.canonicalize()?.join("main.tex")
    );
    assert!(!app.session.editor.path.exists());
    assert!(app.session.proposal_review.is_none() && app.session.proposed_build.is_none());
    assert!(app.session.job.is_none() && app.session.proposed_job.is_none());
    assert!(!app.session.show_proposed && app.session.diagnostics.is_none());
    assert!(app.session.rendered_pdf.is_none() && app.session.rendered_source.is_none());
    assert!(app.session.assistant.input.lines()[0].is_empty());
    assert_eq!(
        std::fs::read_to_string(directory.path().join("resume.tex"))?,
        original
    );
    assert_eq!(
        Settings::load(&app.settings_path)?.resume_folder,
        Some(target.canonicalize()?)
    );
    Ok(())
}

#[test]
fn unchanged_folder_preserves_unsaved_text_and_pending_review() -> anyhow::Result<()> {
    let (directory, mut app) = application()?;
    app.settings_path = directory.path().join("settings.json");
    app.session.editor.text.insert_str("unsaved");
    let source = app.session.editor.source();
    app.session.proposal_review = Some(ProposalReview::new(&ProposedEdit {
        source_before: source.clone(),
        source_after: "pending".into(),
    }));
    app.open_settings();
    app.apply_settings()?;

    assert!(app.settings.is_none());
    assert_eq!(app.session.editor.source(), source);
    assert!(app.session.proposal_review.is_some());
    assert!(Settings::load(&app.settings_path)?.resume_folder.is_none());
    Ok(())
}

#[test]
fn invalid_or_unsavable_settings_preserve_pending_work() -> anyhow::Result<()> {
    let (directory, mut app) = application()?;
    app.session.editor.text.insert_str("unsaved");
    let source = app.session.editor.source();
    app.session.proposal_review = Some(ProposalReview::new(&ProposedEdit {
        source_before: source.clone(),
        source_after: "pending".into(),
    }));
    let root = app.projects.root().to_path_buf();
    let target = directory.path().join("target");
    std::fs::create_dir(&target)?;
    app.settings_path = directory.path().join("resume.tex/settings.json");
    app.open_settings();

    for path in [
        directory.path().join("missing"),
        directory.path().join("resume.tex"),
        target,
    ] {
        app.settings.as_mut().unwrap().input =
            ratatui_textarea::TextArea::new(vec![path.display().to_string()]);
        assert!(
            app.prepare_folder_change()
                .and_then(|_| app.confirm_folder_change())
                .is_err()
        );
        assert_eq!(app.session.editor.source(), source);
        assert_eq!(app.projects.root(), root);
        assert!(app.settings.is_some());
        assert!(app.session.proposal_review.is_some());
    }
    app.handle_settings_event(
        Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        Rect::new(0, 0, 160, 45),
    );
    assert!(app.settings.is_none());
    assert_eq!(app.session.editor.source(), source);
    Ok(())
}

#[test]
fn settings_shortcut_command_and_footer_are_available() -> anyhow::Result<()> {
    let (_directory, mut app) = application()?;
    let area = Rect::new(0, 0, 160, 45);
    app.handle_event(
        Event::Key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL)),
        area,
    );
    app.handle_event(
        Event::Key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE)),
        area,
    );
    assert!(app.settings.is_some());
    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height))?;
    terminal.draw(|frame| app.draw(frame))?;
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("Settings") && text.contains("WORKSPACE"));
    app.handle_settings_event(
        Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        Rect::new(0, 0, 160, 45),
    );
    app.session.assistant.input.insert_str("/set");
    app.session
        .assistant
        .complete_command(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert_eq!(app.session.assistant.input.lines()[0], "/settings");
    app.send_message();
    assert!(app.settings.is_some());
    app.settings = None;
    app.open_settings();
    assert!(app.settings.is_some());
    app.settings = None;
    terminal.draw(|frame| app.draw(frame))?;
    let footer: String = (0..area.width)
        .map(|x| terminal.backend().buffer()[(x, area.height - 1)].symbol())
        .collect();
    let settings_key =
        footer.find(&format!("{} s", app.prefix.label())).unwrap() + app.prefix.label().len() + 1;
    let settings_key = settings_key as u16;
    let footer = footer.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(footer.contains(&format!("{} s Settings", app.prefix.label())));
    assert_eq!(
        terminal.backend().buffer()[(settings_key, area.height - 1)].bg,
        theme::KEYCAP
    );
    assert_eq!(
        terminal.backend().buffer()[(settings_key + 3, area.height - 1)].bg,
        theme::ELEVATED
    );
    assert_eq!(
        terminal.backend().buffer()[(0, area.height - 1)].bg,
        theme::ELEVATED
    );

    let layout = WorkspaceLayout::new(area, true, app.focus);
    assert_eq!(layout.preview.bottom(), layout.status.y);
    assert_eq!(layout.assistant.bottom(), layout.status.y);
    assert_eq!(layout.status.height, 1);
    Ok(())
}

#[test]
#[ignore = "requires macOS, cached Tectonic, and Poppler"]
fn switching_folders_cancels_old_builds_and_displays_the_new_pdf() -> anyhow::Result<()> {
    let (directory, mut app) = application()?;
    app.settings_path = directory.path().join("settings.json");
    let looping = r"\documentclass{article}\begin{document}\loop\iftrue\repeat\end{document}";
    app.session.job = Some(RenderJob::start(
        looping.into(),
        app.session.editor.path.clone(),
    )?);
    app.session.proposed_job = Some(RenderJob::draft(
        looping.into(),
        app.session.editor.path.clone(),
    )?);
    let target = directory.path().join("new");
    std::fs::create_dir(&target)?;
    let source = r"\documentclass{article}\begin{document}New workspace\end{document}";
    let project = resume_project::create(&target, "General")?;
    std::fs::write(project.document(), source)?;
    app.open_settings();
    app.settings.as_mut().unwrap().input =
        ratatui_textarea::TextArea::new(vec![target.display().to_string()]);
    app.prepare_folder_change()?;
    app.confirm_folder_change()?;

    assert!(app.session.proposed_job.is_none());
    assert_eq!(app.session.editor.source(), source);
    let mut terminal = Terminal::new(TestBackend::new(160, 45))?;
    wait_for_visible_pdf(&mut app, &mut terminal)?;
    assert!(project.document().with_extension("pdf").is_file());
    assert!(!directory.path().join("resume.pdf").exists());
    Ok(())
}

#[test]
fn wrap_reload_updates_cached_editors_without_resetting_history_or_proposals() -> anyhow::Result<()>
{
    use ratatui_textarea::WrapMode;

    let (directory, mut app) = application()?;
    let first = app.session.editor.path.clone();
    app.session.editor.text.insert_str("unsaved ");
    let source = app.session.editor.source();
    app.session.proposal_review = Some(ProposalReview::new(&ProposedEdit {
        source_before: source.clone(),
        source_after: "Pending proposal".into(),
    }));
    app.open_document(directory.path().join("second.tex"));
    app.settings_path = directory.path().join("config.toml");
    let mut settings = Settings {
        resume_folder: Some(directory.path().to_path_buf()),
        ..Settings::default()
    };
    assert!(settings.editor.soft_wrap);
    settings.editor.soft_wrap = false;
    settings.save(&app.settings_path)?;
    app.reload_settings()?;
    assert_eq!(app.session.editor.text.wrap_mode(), WrapMode::None);

    app.open_document(first);
    assert_eq!(app.session.editor.text.wrap_mode(), WrapMode::None);
    assert_eq!(app.session.editor.source(), source);
    assert!(app.session.proposal_review.is_some());
    assert!(app.session.editor.text.undo());
    assert_eq!(app.session.editor.source(), "Original document");
    Ok(())
}

#[test]
fn visible_mode_controls_save_or_cancel_without_losing_resume_state() -> anyhow::Result<()> {
    use crate::config::EditingMode;
    use ratatui::layout::Position;

    let (directory, mut app) = application()?;
    app.settings_path = directory.path().join("config.toml");
    let first = app.session.editor.path.clone();
    let second = directory.path().join("second.tex");
    app.session.editor.text.insert_str("Unsaved ");
    let source = app.session.editor.source();
    app.open_document(second.clone());
    app.open_document(first.clone());
    app.session.proposal_review = Some(ProposalReview::new(&ProposedEdit {
        source_before: source.clone(),
        source_after: "Pending proposal".into(),
    }));
    let area = Rect::new(0, 0, 160, 45);
    app.open_settings();
    app.handle_event(
        Event::Key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE)),
        area,
    );
    assert!(app.settings.as_ref().unwrap().editor_preferences.mode == EditingMode::Vim);
    assert!(app.editor_preferences.mode == EditingMode::Standard);
    app.handle_event(
        Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        area,
    );
    assert!(app.editor_preferences.mode == EditingMode::Standard);
    assert!(!app.settings_path.exists());

    app.open_settings();
    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height))?;
    terminal.draw(|frame| app.draw(frame))?;
    let buffer = terminal.backend().buffer();
    let vim_button = (0..area.height)
        .find_map(|row| {
            (0..area.width.saturating_sub(2)).find_map(|column| {
                let label: String = (column..column + 3)
                    .map(|x| buffer[(x, row)].symbol())
                    .collect();
                (label == "Vim").then_some(Position::new(column, row))
            })
        })
        .expect("Vim must be visible in Settings");
    app.handle_event(
        Event::Mouse(event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: vim_button.x,
            row: vim_button.y,
            modifiers: KeyModifiers::NONE,
        }),
        area,
    );
    assert!(app.settings.as_ref().unwrap().editor_preferences.mode == EditingMode::Vim);
    terminal.draw(|frame| app.draw(frame))?;
    assert_eq!(
        terminal.backend().buffer()[(vim_button.x, vim_button.y)].bg,
        theme::ACCENT
    );
    app.handle_event(
        Event::Key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::NONE)),
        area,
    );
    app.handle_event(
        Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        area,
    );

    assert!(app.settings.is_none());
    assert!(Settings::load(&app.settings_path)?.editor.mode == EditingMode::Vim);
    assert_eq!(app.session.editor.source(), source);
    assert!(app.session.proposal_review.is_some());
    app.open_document(second);
    assert!(app.sessions[&first].proposal_review.is_some());
    let second_source = app.session.editor.source();
    app.handle_event(
        Event::Key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)),
        area,
    );
    assert_eq!(app.session.editor.source(), second_source);
    app.open_document(first);
    app.discard_proposal();
    app.handle_event(
        Event::Key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::NONE)),
        area,
    );
    assert_eq!(app.session.editor.source(), "Original document");

    app.open_settings();
    assert!(app.settings.as_ref().unwrap().editor_preferences.mode == EditingMode::Vim);
    for code in [KeyCode::Left, KeyCode::BackTab, KeyCode::Enter] {
        app.handle_event(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)), area);
    }
    assert!(Settings::load(&app.settings_path)?.editor.mode == EditingMode::Standard);
    app.handle_event(
        Event::Key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE)),
        area,
    );
    assert!(app.session.editor.is_modified());
    Ok(())
}

#[test]
fn preference_apply_never_switches_workspaces_and_staged_edits_block_navigation()
-> anyhow::Result<()> {
    let (directory, mut app) = application()?;
    app.settings_path = directory.path().join("config.toml");
    let other = directory.path().join("other");
    std::fs::create_dir(&other)?;
    Settings {
        resume_folder: Some(other.clone()),
        ..Settings::default()
    }
    .save(&app.settings_path)?;
    let root = app.projects.root().to_path_buf();
    app.session.editor.text.insert_str("Unsaved ");
    let source = app.session.editor.source();
    app.open_settings();
    app.settings.as_mut().unwrap().editor_preferences.soft_wrap = false;
    assert!(app.begin_folder_change().is_err());
    assert!(app.reload_settings().is_err());
    assert!(app.editor_preferences.soft_wrap);
    assert!(!app.settings.as_ref().unwrap().editor_preferences.soft_wrap);
    app.apply_settings()?;
    assert_eq!(app.projects.root(), root);
    assert_eq!(app.session.editor.source(), source);
    assert!(!app.editor_preferences.soft_wrap);
    assert_eq!(
        Settings::load(&app.settings_path)?.resume_folder,
        Some(other)
    );
    Ok(())
}

#[test]
fn folder_confirmation_defaults_to_cancel_and_accounts_for_cached_sessions() -> anyhow::Result<()> {
    let (directory, mut app) = application()?;
    let area = Rect::new(0, 0, 100, 35);
    app.settings_path = directory.path().join("config.toml");
    app.session.editor.text.insert_str("Cached unsaved ");
    app.session.assistant.input.insert_str("Draft chat");
    app.session.proposal_review = Some(ProposalReview::new(&ProposedEdit {
        source_before: app.session.editor.source(),
        source_after: "Proposal".into(),
    }));
    app.open_document(directory.path().join("second.tex"));
    app.session.job = None;
    let root = app.projects.root().to_path_buf();
    let target = directory.path().join("target");
    std::fs::create_dir(&target)?;
    app.open_settings();
    app.begin_folder_change()?;
    app.settings.as_mut().unwrap().input =
        ratatui_textarea::TextArea::new(vec![target.display().to_string()]);
    app.prepare_folder_change()?;
    let losses = &app
        .settings
        .as_ref()
        .unwrap()
        .confirmation
        .as_ref()
        .unwrap()
        .losses;
    assert_eq!(
        losses,
        &["unsaved text", "pending AI work or proposals", "chat"]
    );
    app.handle_settings_event(
        Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        area,
    );
    assert_eq!(app.projects.root(), root);
    assert!(!app.settings.as_ref().unwrap().folder_open);
    assert!(!app.settings_path.exists());
    assert_eq!(app.sessions.len(), 1);
    Ok(())
}

#[test]
fn changed_destinations_and_new_losses_require_fresh_confirmation() -> anyhow::Result<()> {
    let (directory, mut app) = application()?;
    app.settings_path = directory.path().join("config.toml");
    let root = app.projects.root().to_path_buf();
    let first = directory.path().join("first");
    let second = directory.path().join("second");
    std::fs::create_dir(&first)?;
    std::fs::create_dir(&second)?;
    app.open_settings();
    app.begin_folder_change()?;
    app.settings.as_mut().unwrap().input =
        ratatui_textarea::TextArea::new(vec![first.display().to_string()]);
    app.prepare_folder_change()?;
    assert!(
        app.settings
            .as_ref()
            .unwrap()
            .confirmation
            .as_ref()
            .unwrap()
            .losses
            .is_empty()
    );

    app.settings.as_mut().unwrap().input =
        ratatui_textarea::TextArea::new(vec![second.display().to_string()]);
    app.confirm_folder_change()?;
    assert_eq!(app.projects.root(), root);
    assert_eq!(
        app.settings
            .as_ref()
            .unwrap()
            .confirmation
            .as_ref()
            .unwrap()
            .destination,
        second.canonicalize()?
    );
    app.session.editor.text.insert_str("New unsaved ");
    app.confirm_folder_change()?;
    assert_eq!(app.projects.root(), root);
    assert_eq!(
        app.settings
            .as_ref()
            .unwrap()
            .confirmation
            .as_ref()
            .unwrap()
            .losses,
        ["unsaved text"]
    );
    app.confirm_folder_change()?;
    assert_eq!(app.projects.root(), second.canonicalize()?);
    Ok(())
}

#[test]
fn failed_switch_save_and_same_canonical_folder_preserve_session() -> anyhow::Result<()> {
    let (directory, mut app) = application()?;
    app.settings_path = directory.path().join("config.toml");
    let root = app.projects.root().to_path_buf();
    let target = directory.path().join("target");
    std::fs::create_dir(&target)?;
    app.session.editor.text.insert_str("Unsaved ");
    let source = app.session.editor.source();
    app.open_settings();
    app.begin_folder_change()?;
    app.settings.as_mut().unwrap().input =
        ratatui_textarea::TextArea::new(vec![target.display().to_string()]);
    app.prepare_folder_change()?;
    std::fs::create_dir(&app.settings_path)?;
    assert!(app.confirm_folder_change().is_err());
    assert_eq!(app.projects.root(), root);
    assert_eq!(app.session.editor.source(), source);
    std::fs::remove_dir(&app.settings_path)?;

    app.settings.as_mut().unwrap().input =
        ratatui_textarea::TextArea::new(vec![root.join(".").display().to_string()]);
    app.prepare_folder_change()?;
    assert!(app.settings.is_none());
    assert_eq!(app.session.editor.source(), source);
    assert_eq!(app.projects.root(), root);
    Ok(())
}

#[test]
fn grouped_settings_support_keyboard_toggle_cancel_and_small_screens() -> anyhow::Result<()> {
    let (directory, mut app) = application()?;
    app.settings_path = directory.path().join("config.toml");
    app.open_settings();
    let screen = Rect::new(0, 0, 100, 35);
    for code in [KeyCode::Right, KeyCode::Tab, KeyCode::Char(' ')] {
        app.handle_settings_event(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)), screen);
    }
    assert!(!app.settings.as_ref().unwrap().editor_preferences.soft_wrap);
    assert!(app.editor_preferences.soft_wrap);
    for (width, height) in [(100, 35), (40, 12), (20, 5), (1, 1)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height))?;
        for control in [
            crate::settings::SettingsControl::Mode,
            crate::settings::SettingsControl::Apply,
        ] {
            app.settings.as_mut().unwrap().focus = control;
            terminal.draw(|frame| app.draw(frame))?;
        }
    }
    app.handle_settings_event(
        Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        screen,
    );
    assert!(app.editor_preferences.soft_wrap);
    assert!(app.editor_preferences.mode == crate::config::EditingMode::Standard);
    assert!(!app.settings_path.exists());
    Ok(())
}

#[test]
fn preference_save_failure_keeps_staged_values_and_live_work() -> anyhow::Result<()> {
    let (directory, mut app) = application()?;
    app.settings_path = directory.path().join("blocked-config");
    std::fs::create_dir(&app.settings_path)?;
    app.session.editor.text.insert_str("Unsaved ");
    let source = app.session.editor.source();
    app.open_settings();
    app.settings.as_mut().unwrap().editor_preferences.soft_wrap = false;
    assert!(app.apply_settings().is_err());
    assert!(!app.settings.as_ref().unwrap().editor_preferences.soft_wrap);
    assert!(app.editor_preferences.soft_wrap);
    assert_eq!(app.session.editor.source(), source);
    Ok(())
}

#[test]
fn editing_a_validated_destination_removes_the_confirmation() -> anyhow::Result<()> {
    let (directory, mut app) = application()?;
    app.settings_path = directory.path().join("config.toml");
    let target = directory.path().join("target");
    std::fs::create_dir(&target)?;
    app.open_settings();
    app.begin_folder_change()?;
    app.settings.as_mut().unwrap().input =
        ratatui_textarea::TextArea::new(vec![target.display().to_string()]);
    app.prepare_folder_change()?;
    assert!(app.settings.as_ref().unwrap().confirmation.is_some());
    let screen = Rect::new(0, 0, 100, 35);
    app.handle_settings_event(
        Event::Key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::NONE)),
        screen,
    );
    app.handle_settings_event(Event::Paste("changed".into()), screen);
    assert!(app.settings.as_ref().unwrap().confirmation.is_none());
    assert_eq!(app.projects.root(), directory.path());
    Ok(())
}
