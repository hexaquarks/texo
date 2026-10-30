use super::{test_support::application, *};
use crate::history::{self, RevisionKind};
use ratatui::{Terminal, backend::TestBackend};

const SCREEN: Rect = Rect::new(0, 0, 140, 45);

fn press(app: &mut App, code: KeyCode) {
    app.handle_event(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)), SCREEN);
}

#[test]
fn browsing_and_paste_leave_working_copy_cursor_and_undo_intact() -> anyhow::Result<()> {
    let (_directory, mut app) = application()?;
    app.session.editor.text.insert_str("Unsaved ");
    let source = app.session.editor.source();
    let cursor = app.session.editor.text.cursor();
    let scroll = app.session.editor.text.scroll_offset();
    app.open_history();
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Enter);
    app.session.history.view.as_mut().unwrap().job = None;
    app.focus = Focus::Editor;
    press(&mut app, KeyCode::Char('x'));
    app.handle_event(Event::Paste("Do not insert".into()), SCREEN);
    app.handle_prefix_action(KeyCode::Char('w'), SCREEN);
    press(&mut app, KeyCode::Char('c'));

    let mut terminal = Terminal::new(TestBackend::new(SCREEN.width, SCREEN.height))?;
    terminal.draw(|frame| app.draw(frame))?;
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("read-only"));
    assert!(text.contains("Original document"));
    assert!(!text.contains("Unsaved Original document"));
    assert!(!text.contains("Compare"));
    assert_eq!(
        std::fs::read_to_string(&app.session.editor.path)?,
        "Original document"
    );
    assert_eq!(app.session.editor.source(), source);
    assert_eq!(app.session.editor.text.cursor(), cursor);
    assert_eq!(app.session.editor.text.scroll_offset(), scroll);
    assert!(app.session.editor.is_modified());

    press(&mut app, KeyCode::Esc);
    assert!(app.session.history.view.is_none());
    assert!(app.session.editor.text.undo());
    assert_eq!(app.session.editor.source(), "Original document");
    Ok(())
}

#[test]
fn restore_recovers_unsaved_work_and_refuses_pending_proposals_or_history_failure()
-> anyhow::Result<()> {
    let (directory, mut app) = application()?;
    app.open_history();
    let revision = app.session.history.revisions[0].clone();
    app.session.editor.text.insert_str("Saved ");
    app.session.editor.save()?;
    app.session.editor.text.insert_str("Unsaved ");
    let unsaved = app.session.editor.source();

    let edit = ProposedEdit {
        source_before: unsaved.clone(),
        source_after: "Proposal".into(),
    };
    app.session.proposal_review = Some(ProposalReview::new(&edit));
    assert!(app.restore_history_revision(&revision).is_err());
    assert_eq!(app.session.editor.source(), unsaved);
    app.session.proposal_review = None;

    let journal = directory
        .path()
        .join(history::DIRECTORY)
        .join("resume.tex.json");
    let valid = std::fs::read(&journal)?;
    std::fs::write(&journal, "broken")?;
    assert!(app.restore_history_revision(&revision).is_err());
    assert_eq!(app.session.editor.source(), unsaved);
    assert_eq!(
        std::fs::read_to_string(&app.session.editor.path)?,
        "Saved Original document"
    );
    std::fs::write(&journal, valid)?;

    app.restore_history_revision(&revision)?;
    app.session.job = None;
    assert_eq!(app.session.editor.source(), revision.source);
    assert!(!app.session.editor.is_modified());
    let entries = history::revisions(&app.session.editor.path)?;
    assert_eq!(entries.len(), 4);
    assert_eq!(entries[2].source, unsaved);
    assert_eq!(entries[2].kind, RevisionKind::Recovery);
    assert_eq!(entries[3].kind, RevisionKind::Restore { from: revision.id });
    Ok(())
}

#[test]
fn save_during_review_records_only_live_text_and_accepted_edits_wait_for_save() -> anyhow::Result<()>
{
    let (_directory, mut app) = application()?;
    app.session.editor.text.insert_str("Manual ");
    let before = app.session.editor.source();
    let edit = ProposedEdit {
        source_before: before.clone(),
        source_after: "Accepted later".into(),
    };
    app.session.proposal_review = Some(ProposalReview::new(&edit));
    app.session.show_proposed = true;
    app.handle_prefix_action(KeyCode::Char('w'), SCREEN);
    let entries = history::revisions(&app.session.editor.path)?;
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[1].source, before);
    assert_eq!(std::fs::read_to_string(&app.session.editor.path)?, before);

    app.session.proposal_review = None;
    assert!(app.apply_edit(&edit));
    assert!(app.session.editor.is_modified());
    assert_eq!(history::revisions(&app.session.editor.path)?.len(), 2);
    app.handle_prefix_action(KeyCode::Char('w'), SCREEN);
    assert_eq!(
        history::revisions(&app.session.editor.path)?[2].source,
        edit.source_after
    );
    Ok(())
}

#[test]
fn included_files_and_cached_working_copies_are_isolated() -> anyhow::Result<()> {
    let workspace = tempfile::tempdir()?;
    let project = resume_project::create(workspace.path(), "Files")?;
    let other = project.root.join("experience.tex");
    std::fs::write(&other, "Experience")?;
    let editor = Editor::open(project.document())?;
    let mut app = App::with_workspace(editor, Preview::default(), workspace.path().into())?;
    app.session.editor.text.insert_str("Main unsaved ");
    let main = app.session.editor.source();
    app.open_document(other.clone());
    app.session.job = None;
    app.session.editor.text.insert_str("Second ");
    app.session.editor.save()?;
    app.open_history();
    assert_eq!(app.session.history.revisions.len(), 2);
    assert_eq!(app.session.history.revisions[0].source, "Second Experience");

    app.open_document(project.document());
    app.session.job = None;
    assert_eq!(app.session.editor.source(), main);
    assert!(app.session.editor.is_modified());
    app.open_history();
    assert_eq!(app.session.history.revisions.len(), 1);
    assert_ne!(app.session.history.revisions[0].source, "Experience");
    Ok(())
}

#[test]
#[ignore = "requires macOS, Tectonic, and Poppler"]
fn historical_builds_are_isolated_and_failure_never_shows_the_live_pdf() -> anyhow::Result<()> {
    let workspace = tempfile::tempdir()?;
    let project = resume_project::create(workspace.path(), "Historical PDF")?;
    let document = project.document();
    let mut app = App::with_workspace(
        Editor::open(document.clone())?,
        Preview::default(),
        workspace.path().into(),
    )?;
    app.start_render();
    test_support::finish_test_render(&mut app)?;
    let live_pdf = std::fs::read(document.with_extension("pdf"))?;
    let live_source = std::fs::read_to_string(&document)?;
    app.open_history();
    app.session.history.selection.select(Some(1));
    app.open_selected_revision();

    let finish = |app: &mut App| -> anyhow::Result<()> {
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        while app
            .session
            .history
            .view
            .as_ref()
            .is_some_and(|view| view.job.is_some())
        {
            anyhow::ensure!(
                std::time::Instant::now() < deadline,
                "Historical build timed out"
            );
            app.receive_history_preview();
            std::thread::sleep(Duration::from_millis(10));
        }
        Ok(())
    };
    finish(&mut app)?;
    assert!(app.active_preview().page_status().is_some());
    assert_eq!(std::fs::read(document.with_extension("pdf"))?, live_pdf);
    assert_eq!(std::fs::read_to_string(&document)?, live_source);

    // A bad historical document must leave the history preview empty, even with a valid live PDF.
    app.session.history.view.as_mut().unwrap().revision.source = "\\unknowncommand".into();
    app.build_history_preview();
    finish(&mut app)?;
    assert!(app.active_preview().page_status().is_none());
    let mut terminal = Terminal::new(TestBackend::new(SCREEN.width, SCREEN.height))?;
    terminal.draw(|frame| app.draw(frame))?;
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("Historical build failed"));
    assert_eq!(std::fs::read(document.with_extension("pdf"))?, live_pdf);
    assert_eq!(app.session.editor.source(), live_source);

    app.return_to_working_copy();
    assert!(app.active_preview().page_status().is_some());
    Ok(())
}

#[test]
#[cfg(unix)]
fn checkpoint_write_failure_aborts_restore_before_changing_either_copy() -> anyhow::Result<()> {
    let (directory, mut app) = application()?;
    app.open_history();
    let revision = app.session.history.revisions[0].clone();
    app.session.editor.text.insert_str("Unsaved ");
    let source = app.session.editor.source();
    let history_directory = directory.path().join(history::DIRECTORY);
    let permissions = std::fs::metadata(&history_directory)?.permissions();
    let mut readonly = permissions.clone();
    readonly.set_readonly(true);
    std::fs::set_permissions(&history_directory, readonly)?;

    let result = app.restore_history_revision(&revision);
    std::fs::set_permissions(&history_directory, permissions)?;
    assert!(result.is_err());
    assert_eq!(app.session.editor.source(), source);
    assert_eq!(
        std::fs::read_to_string(&app.session.editor.path)?,
        "Original document"
    );
    assert_eq!(history::revisions(&app.session.editor.path)?.len(), 1);
    Ok(())
}

#[test]
fn history_layout_handles_tiny_windows_and_footer_clicks_in_long_histories() -> anyhow::Result<()> {
    use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    let (_directory, mut app) = application()?;
    app.open_history();
    let revision = app.session.history.revisions[0].clone();
    // More two-row entries than terminal coordinates can represent must still be safe to browse.
    app.session.history.revisions = vec![revision; usize::from(u16::MAX)];
    let layout = app.workspace_layout(SCREEN);
    app.handle_event(
        Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: layout.files.x + 2,
            row: layout.files.bottom() - 2,
            modifiers: KeyModifiers::NONE,
        }),
        SCREEN,
    );
    assert!(app.session.history.view.is_none());
    app.session.history.revisions.truncate(1);

    for (width, height) in [(80, 24), (40, 12), (20, 5), (1, 1)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height))?;
        terminal.draw(|frame| app.draw(frame))?;
        app.session.history.selection.select(Some(1));
        app.handle_history_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));
        terminal.draw(|frame| app.draw(frame))?;
        app.handle_history_dialog(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
    }
    Ok(())
}

#[test]
#[cfg(unix)]
fn failed_source_write_does_not_create_a_saved_revision() -> anyhow::Result<()> {
    let (directory, mut app) = application()?;
    app.open_history();
    app.session.editor.text.insert_str("Unsaved ");
    let permissions = std::fs::metadata(directory.path())?.permissions();
    let mut readonly = permissions.clone();
    readonly.set_readonly(true);
    std::fs::set_permissions(directory.path(), readonly)?;

    let result = app.session.editor.save();
    std::fs::set_permissions(directory.path(), permissions)?;
    assert!(result.is_err());
    assert!(app.session.editor.is_modified());
    assert_eq!(
        std::fs::read_to_string(&app.session.editor.path)?,
        "Original document"
    );
    assert_eq!(history::revisions(&app.session.editor.path)?.len(), 1);
    Ok(())
}

#[test]
fn permanent_history_uses_real_filename_and_pins_working_copy_when_scrolling() -> anyhow::Result<()>
{
    use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

    let directory = tempfile::tempdir()?;
    let document = directory.path().join("cv.tex");
    std::fs::write(&document, "Initial")?;
    let mut app = App::new(Editor::open(document)?, Preview::default())?;
    for number in 1..20 {
        app.session
            .editor
            .replace_source(&format!("Saved {number}"));
        app.session.editor.save()?;
    }
    app.refresh_history();
    app.session.editor.text.insert_str("Unsaved ");
    let working_source = app.session.editor.source();
    let cursor = app.session.editor.text.cursor();
    let scroll = app.session.editor.text.scroll_offset();
    app.session.history.selection.select(Some(20));

    let mut terminal = Terminal::new(TestBackend::new(SCREEN.width, SCREEN.height))?;
    terminal.draw(|frame| app.draw(frame))?;
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("History · cv.tex"));
    assert!(text.contains("cv.tex · Working copy"));
    assert!(text.contains("Unsaved changes"));
    assert!(text.contains("cv.tex · r1"));
    assert!(!text.contains("Files") && !text.contains("Compare"));
    assert!(app.session.history.selection.offset() > 0);

    let content = Projects::history_content_area(app.workspace_layout(SCREEN).files);
    let click = |row| {
        Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: content.x,
            row,
            modifiers: KeyModifiers::NONE,
        })
    };
    let offset = app.session.history.selection.offset();
    let expected = app.session.history.revisions[offset].id;
    app.handle_event(click(content.y + 2), SCREEN);
    assert_eq!(
        app.session.history.view.as_ref().unwrap().revision.id,
        expected
    );
    app.session.history.view.as_mut().unwrap().job = None;
    app.handle_event(click(content.y), SCREEN);
    assert!(app.session.history.view.is_none());
    assert!(app.focus == Focus::Editor);
    assert_eq!(app.session.editor.source(), working_source);
    assert_eq!(app.session.editor.text.cursor(), cursor);
    assert_eq!(app.session.editor.text.scroll_offset(), scroll);
    Ok(())
}

#[test]
fn selecting_a_resume_returns_to_its_unsaved_working_copy_and_history() -> anyhow::Result<()> {
    let workspace = tempfile::tempdir()?;
    let first = resume_project::create(workspace.path(), "Alpha")?;
    let second = resume_project::create(workspace.path(), "Beta")?;
    let mut app = App::with_workspace(
        Editor::open(first.document())?,
        Preview::default(),
        workspace.path().into(),
    )?;
    app.session.editor.text.insert_str("Unsaved ");
    let source = app.session.editor.source();
    let cursor = app.session.editor.text.cursor();
    let first_revision = app.session.history.revisions[0].id;
    app.open_history();
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Enter);
    app.session.history.view.as_mut().unwrap().job = None;
    press(&mut app, KeyCode::Tab);
    assert!(!app.projects.history_focused);
    press(&mut app, KeyCode::Down);
    app.session.job = None;
    assert_eq!(app.session.editor.path, second.document());
    assert!(app.session.history.view.is_none());
    assert_ne!(app.session.history.revisions[0].id, first_revision);
    press(&mut app, KeyCode::Up);
    assert_eq!(app.session.editor.path, first.document());
    assert!(app.session.history.view.is_none());
    assert_eq!(app.session.history.selection.selected(), Some(0));
    assert_eq!(app.session.history.revisions[0].id, first_revision);
    assert_eq!(app.session.editor.source(), source);
    assert_eq!(app.session.editor.text.cursor(), cursor);
    assert!(app.session.editor.is_modified());
    Ok(())
}
