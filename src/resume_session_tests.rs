use super::test_support::*;
use super::*;

#[test]
fn switching_projects_preserves_unsaved_undo_chat_and_pending_review() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let first = resume_project::create(directory.path(), "General")?;
    let second = resume_project::create(directory.path(), "Backend")?;
    let mut app = App::with_workspace(
        Editor::open(first.document())?,
        Preview::default(),
        directory.path().to_path_buf(),
    )?;
    app.session.editor.text.insert_str("Unsaved edit ");
    let source = app.session.editor.source();
    app.session
        .assistant
        .input
        .insert_str("First project's draft");
    app.session.proposal_review = Some(ProposalReview::new(&ProposedEdit {
        source_before: source.clone(),
        source_after: "Proposed first project".into(),
    }));
    app.session.rendered_source = Some("First build".into());

    app.projects.select_path(&second.document());
    app.activate_projects();
    assert_eq!(app.session.editor.path, second.document());
    assert!(app.session.proposal_review.is_none());
    assert_eq!(app.session.assistant.input.lines(), &[""]);
    assert!(app.session.rendered_source.is_none());
    app.session.job = None;
    app.session.editor.text.insert_str("Second edit ");

    app.open_document(first.document());
    assert_eq!(app.session.editor.source(), source);
    assert!(app.session.proposal_review.is_some());
    assert_eq!(
        app.session.assistant.input.lines(),
        &["First project's draft"]
    );
    app.session.editor.text.undo();
    assert_eq!(app.session.editor.source(), crate::editor::STARTER_DOCUMENT);
    app.open_document(second.document());
    assert!(app.session.editor.source().starts_with("Second edit"));
    Ok(())
}

#[test]
fn cached_resumes_do_not_restore_old_workspace_settings_or_navigation() -> anyhow::Result<()> {
    let (directory, mut app) = application()?;
    let first = app.session.editor.path.clone();
    let second = directory.path().join("second.tex");

    // Unsaved buffers avoid starting compilation; this test exercises session ownership.
    app.session.editor.text.insert_str("First draft ");
    app.open_document(second.clone());
    app.session.assistant.input.insert_str("Second prompt");

    app.settings_path = directory.path().join("custom-config.toml");
    app.prefix = Prefix::new("ctrl+a")?;
    app.projects_visible = false;
    app.projects.history_focused = true;
    app.focus = Focus::Assistant;

    for path in [first.clone(), second.clone(), first, second] {
        app.open_document(path.clone());

        assert_eq!(app.session.editor.path, path);
        assert_eq!(app.sessions.len(), 1);
        assert!(!app.sessions.contains_key(&path));
        assert_eq!(
            app.settings_path,
            directory.path().join("custom-config.toml")
        );
        assert_eq!(app.prefix.label(), Prefix::new("ctrl+a")?.label());
        assert!(!app.projects_visible);
        assert!(app.projects.history_focused);
        assert!(app.focus == Focus::Assistant);
    }

    assert_eq!(app.session.assistant.input.lines(), &["Second prompt"]);
    Ok(())
}

#[test]
fn unrecognized_directories_and_loose_files_are_not_resume_projects() -> anyhow::Result<()> {
    let (directory, mut app) = application()?;
    std::fs::create_dir(directory.path().join("Empty"))?;
    app.refresh_projects();
    assert!(app.projects.catalog.projects.is_empty());
    assert_eq!(app.projects.catalog.loose_files.len(), 1);
    app.activate_projects();
    assert!(app.session.editor.source().contains("Original document"));
    Ok(())
}

#[test]
#[ignore = "requires Tectonic and Poppler"]
fn project_selection_replaces_editor_and_preview_during_a_build() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let first_project = resume_project::create(directory.path(), "AI engineering")?.root;
    let second_project = resume_project::create(directory.path(), "Backend")?.root;
    let first = first_project.join("main.tex");
    let second = second_project.join("main.tex");
    std::fs::write(
        &first,
        "\\documentclass{article}\n\\begin{document}First resume\\end{document}\n",
    )?;
    let second_source = include_str!("../tests/fixtures/two-pages.tex");
    std::fs::write(&second, second_source)?;
    let editor = Editor::open(first.clone())?;
    let mut app = App::with_workspace(editor, Preview::default(), directory.path().to_path_buf())?;
    app.start_render();
    finish_test_render(&mut app)?;
    assert_eq!(app.session.preview.page_status().as_deref(), Some("1 / 1"));

    app.session
        .assistant
        .input
        .insert_str("Old project question");
    app.start_render();
    assert!(app.session.job.is_some());
    app.projects.select_path(&second);
    app.activate_projects();
    assert_eq!(app.session.editor.path, second);
    assert_eq!(app.session.editor.source(), second_source);
    assert!(app.session.preview.page_status().is_none());
    assert!(app.session.rendered_pdf.is_none());
    assert_eq!(app.session.assistant.input.lines(), &[""]);
    assert!(app.session.job.is_some());
    finish_test_render(&mut app)?;
    assert_eq!(app.session.preview.page_status().as_deref(), Some("1 / 2"));
    assert_eq!(app.session.rendered_source.as_deref(), Some(second_source));
    assert_eq!(app.session.rendered_pdf, Some(second.with_extension("pdf")));

    app.open_document(first.clone());
    finish_test_render(&mut app)?;
    assert_eq!(app.session.preview.page_status().as_deref(), Some("1 / 1"));
    assert_eq!(app.session.rendered_pdf, Some(first.with_extension("pdf")));
    assert_eq!(
        app.session.assistant.input.lines(),
        &["Old project question"]
    );
    Ok(())
}

#[test]
fn late_assistant_reply_stays_with_its_resume_and_rejects_newer_edits() -> anyhow::Result<()> {
    use crate::assistant_job::{AssistantReply, CompletedTask};

    let directory = tempfile::tempdir()?;
    let first = resume_project::create(directory.path(), "First")?;
    let second = resume_project::create(directory.path(), "Second")?;
    let mut app = App::with_workspace(
        Editor::open(first.document())?,
        Preview::default(),
        directory.path().to_path_buf(),
    )?;
    let sender = app
        .session
        .assistant
        .pending_request(app.session.editor.source());
    app.session.editor.replace_source("Newer local edit");
    app.open_document(second.document());
    app.session.job = None;
    sender.send(Ok(CompletedTask {
        reply: AssistantReply {
            reply: "First resume revised".into(),
            proposed_tex: Some("Old response".into()),
        },
        build: None,
    }))?;

    app.receive_assistant();
    assert!(app.session.proposal_review.is_none());
    assert_eq!(app.session.editor.source(), crate::editor::STARTER_DOCUMENT);

    app.open_document(first.document());
    app.receive_assistant();
    assert!(app.session.proposal_review.is_none());
    assert_eq!(app.session.editor.source(), "Newer local edit");
    assert!(app.session.status.contains("stale"));
    Ok(())
}

#[test]
fn empty_workspaces_require_project_creation_and_missing_sources_require_confirmation()
-> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let loose = directory.path().join("main.tex");
    std::fs::write(&loose, "Untouched legacy source")?;
    let mut app = App::with_workspace(
        Editor::placeholder(loose.clone())?,
        Preview::default(),
        directory.path().to_path_buf(),
    )?;
    app.session.empty_workspace = true;
    let area = Rect::new(0, 0, 160, 45);
    app.handle_event(Event::Paste("Invisible text".into()), area);
    assert_eq!(app.session.editor.source(), crate::editor::STARTER_DOCUMENT);

    app.handle_control_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
    assert!(matches!(
        app.project_dialog.as_ref().unwrap().action,
        Action::NewResume
    ));
    app.handle_project_dialog(
        Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        area,
    );
    app.session.job = None;
    assert!(!app.session.empty_workspace);
    assert_ne!(app.session.editor.path, loose);
    assert_eq!(std::fs::read_to_string(&loose)?, "Untouched legacy source");

    app.session.editor.text.insert_str("Unsaved ");
    let source = app.session.editor.source();
    std::fs::remove_file(&app.session.editor.path)?;
    app.handle_control_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
    assert!(matches!(
        app.project_dialog.as_ref().unwrap().action,
        Action::RecoverSource
    ));
    assert!(!app.session.editor.path.exists());

    app.handle_project_dialog(
        Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        area,
    );
    assert_eq!(app.session.editor.source(), source);
    assert!(!app.session.editor.path.exists());
    app.open_project_dialog(Action::RecoverSource);
    app.handle_project_dialog(
        Event::Key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE)),
        area,
    );
    app.handle_project_dialog(
        Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        area,
    );
    app.session.job = None;
    assert_eq!(std::fs::read_to_string(&app.session.editor.path)?, source);
    Ok(())
}

#[test]
fn ambiguous_import_requires_selection_and_preserves_originals() -> anyhow::Result<()> {
    let (_directory, mut app) = application()?;
    let source = tempfile::tempdir()?;
    for name in ["first.tex", "second.tex"] {
        std::fs::write(source.path().join(name), crate::editor::STARTER_DOCUMENT)?;
    }
    let mut dialog = ProjectDialog::new(Action::ImportResume, None, None);
    dialog.source.insert_str(source.path().to_string_lossy());
    assert!(!app.finish_project_dialog(&mut dialog)?);
    assert!(app.projects.catalog.projects.is_empty());
    assert_eq!(dialog.plan.as_ref().unwrap().candidates.len(), 2);

    app.project_dialog = Some(dialog);
    let area = Rect::new(0, 0, 100, 30);
    for character in ['j', 'k', 'j'] {
        app.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE)),
            area,
        );
    }
    assert_eq!(
        app.project_dialog.as_ref().unwrap().selection.selected(),
        Some(1)
    );
    app.handle_event(
        Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        area,
    );
    assert!(app.project_dialog.is_none());
    app.session.job = None;
    assert_eq!(app.session.editor.path.file_name().unwrap(), "second.tex");
    assert_eq!(
        std::fs::read_to_string(source.path().join("second.tex"))?,
        crate::editor::STARTER_DOCUMENT
    );
    Ok(())
}

#[test]
fn switching_resumes_keeps_search_selection_cursor_and_undo() -> anyhow::Result<()> {
    use ratatui::{Terminal, backend::TestBackend};
    use ratatui_textarea::CursorMove;

    let (directory, mut app) = application()?;
    let first = app.session.editor.path.clone();
    app.session
        .editor
        .replace_source("Résumé 李\nSecond line\n");
    app.session.editor.text.move_cursor(CursorMove::Jump(0, 0));
    app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL));
    let area = Rect::new(0, 0, 160, 45);
    app.handle_event(Event::Paste("李".into()), area);
    let cursor = app.session.editor.text.cursor();
    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height))?;
    terminal.draw(|frame| app.draw(frame))?;
    app.open_document(directory.path().join("second.tex"));
    app.session.editor.text.insert_str("Second draft");
    app.open_document(first.clone());
    assert_eq!(app.session.editor.text.cursor(), cursor);
    assert!(
        app.session
            .editor
            .text
            .search_pattern()
            .unwrap()
            .is_match("李")
    );
    terminal.draw(|frame| app.draw(frame))?;
    let visible: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(visible.contains("Find:"));
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    app.session.editor.text.start_selection();
    app.session.editor.text.move_cursor(CursorMove::End);
    let selection = app.session.editor.text.selection_range();
    app.open_document(directory.path().join("second.tex"));
    app.open_document(first);
    assert_eq!(app.session.editor.text.selection_range(), selection);
    app.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL));
    app.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL));
    assert_eq!(app.session.editor.source(), "Original document");
    Ok(())
}

#[test]
fn bracketed_paste_inserts_shortcut_characters_as_one_literal_multiline_edit() -> anyhow::Result<()>
{
    let (_directory, mut app) = application()?;
    app.session.editor.text.select_all();
    let pasted = "?\nctrl+x s\n\\section{Résumé 李}\n\n";
    app.handle_event(Event::Paste(pasted.into()), Rect::new(0, 0, 160, 45));
    assert_eq!(app.session.editor.source(), pasted);
    assert!(app.settings.is_none());
    assert!(!app.prefix.armed);
    app.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL));
    app.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL));
    assert_eq!(app.session.editor.source(), "Original document");
    Ok(())
}

#[test]
fn concurrent_assistant_requests_remain_owned_by_their_resumes() -> anyhow::Result<()> {
    use crate::assistant_job::{AssistantReply, CompletedTask};

    let (directory, mut app) = application()?;
    let first = app.session.editor.path.clone();
    let second = directory.path().join("second.tex");
    let first_source = app.session.editor.source();
    let first_reply = app.session.assistant.pending_request(first_source.clone());

    app.open_document(second.clone());
    app.session.job = None;
    let second_source = app.session.editor.source();
    let second_reply = app.session.assistant.pending_request(second_source.clone());
    app.session.assistant.input.insert_str("Another request");
    app.send_message();
    assert!(app.session.status.contains("cancel the current request"));
    assert_eq!(app.session.assistant.input.lines(), &["Another request"]);

    // Complete the second request first, while the first resume is visible.
    app.open_document(first.clone());
    second_reply.send(Ok(CompletedTask {
        reply: AssistantReply {
            reply: "Second answer".into(),
            proposed_tex: None,
        },
        build: None,
    }))?;
    app.receive_assistant();
    assert!(app.session.assistant.is_running());
    assert_eq!(app.session.editor.source(), first_source);

    app.open_document(second.clone());
    app.receive_assistant();
    assert!(!app.session.assistant.is_running());
    assert_eq!(app.session.editor.source(), second_source);
    assert!(app.sessions[&first].assistant.is_running());

    // Cancelling A must neither cancel B nor let A's old response enter a new request.
    let second_followup = app.session.assistant.pending_request(second_source.clone());
    app.open_document(first.clone());
    app.session.assistant.cancel();
    let first_followup = app.session.assistant.pending_request(first_source.clone());
    assert!(
        first_reply
            .send(Err(anyhow::anyhow!("Cancelled response")))
            .is_err()
    );
    assert!(app.sessions[&second].assistant.is_running());

    first_followup.send(Ok(CompletedTask {
        reply: AssistantReply {
            reply: "New first answer".into(),
            proposed_tex: Some("Old first edit".into()),
        },
        build: None,
    }))?;
    app.session.editor.replace_source("Newer first edit");
    app.open_document(second);
    second_followup.send(Ok(CompletedTask {
        reply: AssistantReply {
            reply: "New second answer".into(),
            proposed_tex: None,
        },
        build: None,
    }))?;
    app.receive_assistant();
    assert_eq!(app.session.editor.source(), second_source);

    app.open_document(first);
    app.receive_assistant();
    assert_eq!(app.session.editor.source(), "Newer first edit");
    assert!(app.session.status.contains("stale"));
    assert!(app.session.proposal_review.is_none());
    Ok(())
}

#[test]
fn background_build_failure_updates_only_its_own_resume() -> anyhow::Result<()> {
    use crate::{projects::BuildState, render_job::RenderJob};

    let (directory, mut app) = application()?;
    let first = app.session.editor.path.clone();
    let (job, completed) = RenderJob::pending();
    app.session.building_source = app.session.editor.source();
    app.session.job = Some(job);
    app.open_document(directory.path().join("second.tex"));
    app.session.job = None;
    app.session.status = "Second resume status".into();
    let source = app.session.editor.source();
    assert_eq!(app.sessions[&first].build_state(), BuildState::Building);

    completed.send(Err(anyhow::anyhow!("First resume build failed")))?;
    app.receive_background_builds();
    assert_eq!(app.sessions[&first].build_state(), BuildState::Failed);
    assert_eq!(app.session.status, "Second resume status");
    assert_eq!(app.session.editor.source(), source);
    assert!(app.session.diagnostics.is_none());

    app.open_document(first);
    assert!(
        app.session
            .diagnostics
            .as_deref()
            .unwrap()
            .contains("First resume build failed")
    );
    Ok(())
}

#[test]
#[ignore = "requires Tectonic and Poppler"]
fn background_builds_finish_without_reopening_and_reject_newer_edits() -> anyhow::Result<()> {
    use crate::projects::BuildState;

    const BUILD_TIMEOUT: Duration = Duration::from_secs(30);
    const BUILD_POLL_INTERVAL: Duration = Duration::from_millis(20);
    const SWITCH_ROUND_TRIPS: usize = 5;

    let directory = tempfile::tempdir()?;
    let first = resume_project::create(directory.path(), "First")?.document();
    let second = resume_project::create(directory.path(), "Second")?.document();
    let first_source = r"\documentclass{article}\begin{document}First\end{document}";
    let second_source = include_str!("../tests/fixtures/two-pages.tex");
    std::fs::write(&first, first_source)?;
    std::fs::write(&second, second_source)?;
    let mut app = App::with_workspace(
        Editor::open(first.clone())?,
        Preview::default(),
        directory.path().to_path_buf(),
    )?;
    app.start_render();
    app.open_document(second.clone());

    for _ in 0..SWITCH_ROUND_TRIPS {
        app.open_document(first.clone());
        assert!(app.session.job.is_some());
        app.open_document(second.clone());
        assert!(app.session.job.is_some());
        assert_eq!(app.sessions.len(), 1);
    }

    let deadline = std::time::Instant::now() + BUILD_TIMEOUT;
    while app.session.job.is_some() || app.sessions[&first].job.is_some() {
        anyhow::ensure!(std::time::Instant::now() < deadline, "Builds timed out");
        app.receive_render();
        app.receive_background_builds();
        std::thread::sleep(BUILD_POLL_INTERVAL);
    }

    assert_eq!(app.sessions[&first].build_state(), BuildState::Ready);
    assert_eq!(
        app.sessions[&first].preview.page_status().as_deref(),
        Some("1 / 1")
    );
    assert_eq!(app.session.build_state(), BuildState::Ready);
    assert_eq!(app.session.preview.page_status().as_deref(), Some("1 / 2"));
    assert!(first.with_extension("pdf").is_file());
    let original_pdf = std::fs::read(first.with_extension("pdf"))?;

    for _ in 0..SWITCH_ROUND_TRIPS {
        app.open_document(first.clone());
        assert!(app.session.job.is_none());
        assert_eq!(app.session.rendered_source.as_deref(), Some(first_source));
        app.open_document(second.clone());
        assert!(app.session.job.is_none());
        assert_eq!(app.session.rendered_source.as_deref(), Some(second_source));
    }

    app.open_document(first.clone());
    app.start_render();
    app.session
        .editor
        .replace_source("Newer unsaved first resume");
    app.open_document(second);
    let deadline = std::time::Instant::now() + BUILD_TIMEOUT;
    while app.sessions[&first].job.is_some() {
        anyhow::ensure!(std::time::Instant::now() < deadline, "Rebuild timed out");
        app.receive_background_builds();
        std::thread::sleep(BUILD_POLL_INTERVAL);
    }

    assert_eq!(app.sessions[&first].build_state(), BuildState::Empty);
    assert!(app.sessions[&first].status.contains("outdated"));
    assert_eq!(std::fs::read(first.with_extension("pdf"))?, original_pdf);
    assert_eq!(app.session.editor.source(), second_source);
    assert_eq!(app.session.build_state(), BuildState::Ready);
    Ok(())
}
