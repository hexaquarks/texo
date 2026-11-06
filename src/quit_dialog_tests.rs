use super::super::{Focus, test_support::application};
use super::*;
use crate::assistant_job::{AssistantReply, CompletedTask};
use ratatui::{
    Terminal,
    backend::TestBackend,
    crossterm::event::{KeyEvent, KeyModifiers, MouseEvent},
};

const SCREEN: Rect = Rect::new(0, 0, 160, 45);

fn key(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

#[test]
fn every_quit_shortcut_checks_unsaved_work() -> anyhow::Result<()> {
    for (focus, shortcut) in [
        (
            Focus::Editor,
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL),
        ),
        (
            Focus::Projects,
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
        ),
        (
            Focus::Preview,
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
        ),
    ] {
        let (_directory, mut app) = application()?;
        app.focus = focus;
        assert!(app.handle_event(Event::Key(shortcut), SCREEN));

        app.session.editor.text.insert_str("Unsaved ");
        let source = app.session.editor.source();
        assert!(!app.handle_event(Event::Key(shortcut), SCREEN));
        assert!(app.quit_dialog.is_some());
        assert!(!app.handle_event(Event::Key(shortcut), SCREEN));
        assert!(!app.handle_event(key(KeyCode::Enter), SCREEN));
        assert!(app.quit_dialog.is_none());
        assert_eq!(app.session.editor.source(), source);
    }

    let (_directory, mut app) = application()?;
    app.session.editor.text.insert_str("Unsaved ");
    assert!(!app.handle_event(
        Event::Key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL)),
        SCREEN,
    ));
    assert!(!app.handle_event(key(KeyCode::Char('q')), SCREEN));
    assert!(app.quit_dialog.is_some());
    Ok(())
}

#[test]
fn quit_includes_cached_resumes_ai_work_and_chat_drafts() -> anyhow::Result<()> {
    let (directory, mut app) = application()?;
    let first = app.session.editor.path.clone();
    app.session.editor.text.insert_str("Unsaved first ");
    let source = app.session.editor.source();
    let _reply = app.session.assistant.pending_request(source.clone());
    app.session.assistant.input.insert_str("Unsent follow-up");
    let second = directory.path().join("second.tex");
    std::fs::write(&second, "Second resume")?;
    app.open_document(second);
    app.session.job = None;

    assert!(!app.request_quit());
    let losses = &app.quit_dialog.as_ref().unwrap().losses;
    assert!(losses.iter().any(|loss| loss.contains("unsaved text")));
    assert!(
        losses
            .iter()
            .any(|loss| loss.contains("pending AI work or proposals"))
    );
    assert!(losses.iter().any(|loss| loss.contains("chat")));
    assert!(!app.handle_event(key(KeyCode::Esc), SCREEN));
    assert!(app.quit_dialog.is_none());
    assert!(app.sessions[&first].assistant.is_running());
    assert_eq!(app.sessions[&first].editor.source(), source);
    assert_eq!(std::fs::read_to_string(first)?, "Original document");
    Ok(())
}

#[test]
fn a_late_ai_reply_requires_a_fresh_quit_confirmation() -> anyhow::Result<()> {
    let (_directory, mut app) = application()?;
    let sender = app
        .session
        .assistant
        .pending_request(app.session.editor.source());
    assert!(!app.request_quit());
    assert!(!app.handle_event(key(KeyCode::Right), SCREEN));

    sender.send(Ok(CompletedTask {
        reply: AssistantReply {
            reply: "Finished answer".into(),
            proposed_tex: None,
        },
        build: None,
    }))?;
    app.receive_assistant();
    assert!(!app.handle_event(key(KeyCode::Enter), SCREEN));
    assert!(app.quit_dialog.as_ref().unwrap().focus == ButtonFocus::Cancel);
    assert!(!app.handle_event(key(KeyCode::Right), SCREEN));
    assert!(app.handle_event(key(KeyCode::Enter), SCREEN));
    Ok(())
}

#[test]
fn proposals_and_unapplied_settings_require_confirmation() -> anyhow::Result<()> {
    use crate::{assistant::ProposedEdit, proposal::ProposalReview};

    let (_directory, mut app) = application()?;
    let original = app.session.editor.source();
    app.session.proposal_review = Some(ProposalReview::new(&ProposedEdit {
        source_before: original.clone(),
        source_after: "Proposed edit".into(),
    }));
    assert!(!app.request_quit());
    assert!(!app.handle_event(key(KeyCode::Esc), SCREEN));
    assert!(app.session.proposal_review.is_some());
    assert_eq!(app.session.editor.source(), original);

    app.session.proposal_review = None;
    app.open_settings();
    app.settings.as_mut().unwrap().toggle_mode();
    assert!(!app.request_quit());
    assert!(
        app.quit_dialog
            .as_ref()
            .unwrap()
            .losses
            .contains(&"unapplied settings".into())
    );
    assert!(!app.handle_event(key(KeyCode::Esc), SCREEN));
    assert!(app.settings.as_ref().unwrap().has_staged_preferences());
    Ok(())
}

#[test]
fn quit_dialog_draws_and_mouse_confirmation_does_not_save_changes() -> anyhow::Result<()> {
    let (_directory, mut app) = application()?;
    app.session.editor.text.insert_str("Unsaved ");
    assert!(!app.request_quit());

    for (width, height) in [(160, 45), (80, 24), (40, 18), (12, 8)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height))?;
        terminal.draw(|frame| app.draw(frame))?;
        if width >= 40 {
            let screen: String = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect();
            assert!(screen.contains("Quit Texo?"));
            assert!(screen.contains("Quit & discard"));
            assert!(screen.contains("Cancel"));
        }
    }

    let quit = app.quit_dialog.as_ref().unwrap().layout(SCREEN).buttons[1];
    assert!(app.handle_event(
        Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: quit.x,
            row: quit.y,
            modifiers: KeyModifiers::NONE,
        }),
        SCREEN
    ));
    assert_eq!(
        std::fs::read_to_string(&app.session.editor.path)?,
        "Original document"
    );
    Ok(())
}

#[test]
fn quit_names_the_unsaved_resume_after_switching_to_a_clean_one() -> anyhow::Result<()> {
    use crate::{editor::Editor, resume_project};
    use texo::preview::Preview;

    let directory = tempfile::tempdir()?;
    let first = resume_project::create(directory.path(), "Backend resume")?;
    let second = resume_project::create(directory.path(), "Research resume")?;
    let mut app = App::with_workspace(
        Editor::open(first.document())?,
        Preview::default(),
        directory.path().to_path_buf(),
    )?;
    app.session.editor.text.insert_str("Unsaved ");
    app.open_document(second.document());
    app.session.job = None;

    assert!(!app.request_quit());
    let message = app.quit_dialog.as_ref().unwrap().message();
    assert!(message.contains("Backend resume"));
    assert!(message.contains("unsaved text"));
    assert!(!message.contains("Research resume"));
    assert!(!app.handle_event(key(KeyCode::Esc), SCREEN));

    app.session
        .assistant
        .input
        .insert_str("Unsent research prompt");
    assert!(!app.request_quit());
    let losses = &app.quit_dialog.as_ref().unwrap().losses;
    assert_eq!(losses.len(), 2);
    assert!(
        losses
            .iter()
            .any(|loss| loss.contains("Backend resume") && loss.contains("unsaved text"))
    );
    assert!(
        losses
            .iter()
            .any(|loss| loss.contains("Research resume") && loss.contains("chat"))
    );
    Ok(())
}

#[test]
fn long_quit_lists_scroll_without_hiding_the_confirmation_buttons() -> anyhow::Result<()> {
    const RESUME_COUNT: usize = 20;
    let losses = (0..RESUME_COUNT)
        .map(|index| format!("Resume {index}: unsaved text"))
        .collect();
    let mut dialog = QuitDialog::new(losses);
    let screen = Rect::new(0, 0, 60, 18);
    let mut terminal = Terminal::new(TestBackend::new(screen.width, screen.height))?;
    terminal.draw(|frame| dialog.draw(frame))?;
    assert!(dialog.max_scroll(screen) > 0);

    for _ in 0..RESUME_COUNT {
        dialog.handle(key(KeyCode::PageDown), screen);
    }
    terminal.draw(|frame| dialog.draw(frame))?;
    let visible: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(visible.contains("Resume 19: unsaved text"));
    assert!(visible.contains("Quit & discard"));
    assert!(visible.contains("Cancel"));
    assert_eq!(dialog.scroll, dialog.max_scroll(screen));
    Ok(())
}
