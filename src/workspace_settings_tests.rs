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
    let dialog = app
        .settings
        .as_ref()
        .expect("Settings stays open after switching folders");
    assert_eq!(
        dialog.value(),
        crate::workspace::display_path(&target.canonicalize()?)
    );
    assert_eq!(dialog.prefix_label, Prefix::new("ctrl+a")?.label());
    assert_eq!(dialog.notice, "Configuration reloaded.");

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
    app.apply_settings()?;
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
    app.apply_settings()?;

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
    assert!(Settings::load(&app.settings_path)?.resume_folder.is_some());
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
        assert!(app.apply_settings().is_err());
        assert_eq!(app.session.editor.source(), source);
        assert_eq!(app.projects.root(), root);
        assert!(app.settings.is_some());
        assert!(app.session.proposal_review.is_some());
    }
    app.handle_settings_event(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
    assert!(app.settings.is_none());
    assert_eq!(app.session.editor.source(), source);
    Ok(())
}

#[test]
fn settings_shortcut_command_and_footer_are_available() -> anyhow::Result<()> {
    let (_directory, mut app) = application()?;
    let area = Rect::new(0, 0, 160, 45);
    app.handle_event(
        Event::Key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE)),
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
    assert!(text.contains("Settings") && text.contains("Resume folder"));
    app.handle_settings_event(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
    app.session.assistant.input.insert_str("/set");
    app.session
        .assistant
        .complete_command(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert_eq!(app.session.assistant.input.lines()[0], "/settings");
    app.send_message();
    assert!(app.settings.is_some());
    app.settings = None;
    app.perform_action(Action::Settings);
    assert!(app.settings.is_some());
    app.settings = None;
    terminal.draw(|frame| app.draw(frame))?;
    let footer: String = (0..area.width)
        .map(|x| terminal.backend().buffer()[(x, area.height - 1)].symbol())
        .collect();
    let footer = footer.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(footer.contains("? Settings"));
    let settings_key = (0..area.width)
        .find(|x| terminal.backend().buffer()[(*x, area.height - 1)].symbol() == "?")
        .unwrap();
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

    let layout = WorkspaceLayout::new(area, false, true, app.focus);
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
    app.apply_settings()?;

    assert!(app.session.proposed_job.is_none());
    assert_eq!(app.session.editor.source(), source);
    let mut terminal = Terminal::new(TestBackend::new(160, 45))?;
    wait_for_visible_pdf(&mut app, &mut terminal)?;
    assert!(project.document().with_extension("pdf").is_file());
    assert!(!directory.path().join("resume.pdf").exists());
    Ok(())
}
