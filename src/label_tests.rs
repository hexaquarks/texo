use super::*;
use crate::resume_label::{LabelColor, PRESET_LABELS, ResumeLabel};
use ratatui::{Terminal, backend::TestBackend};

const SCREEN: Rect = Rect::new(0, 0, 100, 30);

fn key(app: &mut App, code: KeyCode) {
    app.handle_event(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)), SCREEN);
}

fn managed_application() -> anyhow::Result<(tempfile::TempDir, App)> {
    let directory = tempfile::tempdir()?;
    let project = resume_project::create(directory.path(), "General")?;
    let app = App::with_workspace(
        Editor::open(project.document())?,
        Preview::default(),
        directory.path().to_path_buf(),
    )?;
    Ok((directory, app))
}

#[test]
fn custom_label_dialog_preserves_unsaved_history_review_and_concurrent_rename() -> anyhow::Result<()>
{
    let (_directory, mut app) = managed_application()?;
    let root = app.session.editor.path.parent().unwrap().to_path_buf();
    app.session.editor.text.insert_str("Unsaved edit ");
    let source = app.session.editor.source();
    app.session.proposal_review = Some(ProposalReview::new(&ProposedEdit {
        source_before: source.clone(),
        source_after: "Pending proposal".into(),
    }));
    app.perform_action(Action::LabelResume);
    assert!(app.label_dialog.is_some());

    for _ in 0..=PRESET_LABELS.len() {
        key(&mut app, KeyCode::Down);
    }
    key(&mut app, KeyCode::Enter);
    app.handle_event(Event::Paste("Systems engineer".into()), SCREEN);
    key(&mut app, KeyCode::Tab);
    key(&mut app, KeyCode::Right);
    resume_project::Project::load(&root)?.rename("Renamed elsewhere")?;
    key(&mut app, KeyCode::Enter);

    let expected = Some(ResumeLabel::new("Systems engineer", LabelColor::Teal)?);
    let project = resume_project::Project::load(&root)?;
    assert_eq!(project.manifest.label, expected);
    assert_eq!(project.manifest.name, "Renamed elsewhere");
    assert!(app.label_dialog.is_none());
    assert_eq!(app.session.editor.source(), source);
    assert!(app.session.proposal_review.is_some());
    app.session.editor.text.undo();
    assert_eq!(app.session.editor.source(), crate::editor::STARTER_DOCUMENT);

    // A fresh application must recover both the custom name and its color.
    let mut reopened = App::with_workspace(
        Editor::open(project.document())?,
        Preview::default(),
        app.projects.root().to_path_buf(),
    )?;
    reopened.open_label_dialog();
    key(&mut reopened, KeyCode::Enter);
    assert!(reopened.label_dialog.is_none());
    assert_eq!(
        resume_project::Project::load(&root)?.manifest.label,
        expected
    );
    Ok(())
}

#[test]
fn label_shortcut_is_scoped_and_invalid_or_cancelled_labels_do_not_save() -> anyhow::Result<()> {
    let (_directory, mut app) = managed_application()?;
    let root = app.session.editor.path.parent().unwrap().to_path_buf();
    key(&mut app, KeyCode::Char('l'));
    assert!(app.label_dialog.is_none());
    assert!(app.session.editor.source().starts_with('l'));

    app.focus = Focus::Projects;
    app.projects.files_focused = true;
    key(&mut app, KeyCode::Char('l'));
    assert!(app.label_dialog.is_none());
    app.projects.files_focused = false;
    key(&mut app, KeyCode::Char('l'));
    assert!(app.label_dialog.is_some());
    for _ in 0..=PRESET_LABELS.len() {
        key(&mut app, KeyCode::Down);
    }
    key(&mut app, KeyCode::Enter);
    app.handle_event(Event::Paste("one two three four".into()), SCREEN);
    key(&mut app, KeyCode::Enter);
    assert!(!app.label_dialog.as_ref().unwrap().error.is_empty());
    assert!(
        resume_project::Project::load(&root)?
            .manifest
            .label
            .is_none()
    );

    for (width, height) in [(100, 30), (32, 10), (2, 2)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height))?;
        terminal.draw(|frame| app.draw(frame))?;
    }
    key(&mut app, KeyCode::Esc);
    key(&mut app, KeyCode::Esc);
    assert!(app.label_dialog.is_none());
    assert!(
        resume_project::Project::load(&root)?
            .manifest
            .label
            .is_none()
    );
    Ok(())
}

#[test]
fn resume_manage_control_opens_rename_and_label_actions() -> anyhow::Result<()> {
    let (_directory, mut app) = managed_application()?;
    app.focus = Focus::Projects;
    let source = app.session.editor.source();
    let mut terminal = Terminal::new(TestBackend::new(SCREEN.width, SCREEN.height))?;
    terminal.draw(|frame| app.draw(frame))?;
    let resumes = app.workspace_layout(SCREEN).projects;
    let button = app.projects.manage_button(resumes);
    assert_eq!(button.y, resumes.bottom() - 2);
    let text: String = (button.x..button.right())
        .map(|x| terminal.backend().buffer()[(x, button.y)].symbol())
        .collect();
    assert!(text.contains("Manage"));
    assert_eq!(
        terminal.backend().buffer()[(button.x, button.y)].bg,
        theme::KEYCAP
    );

    app.handle_event(
        Event::Mouse(event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: button.x,
            row: button.y,
            modifiers: KeyModifiers::NONE,
        }),
        SCREEN,
    );
    assert!(matches!(
        app.actions.as_ref().unwrap().selected(),
        Action::RenameResume
    ));
    key(&mut app, KeyCode::Enter);
    assert!(app.project_dialog.is_some());
    key(&mut app, KeyCode::Esc);

    app.focus = Focus::Projects;
    key(&mut app, KeyCode::Char('m'));
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Enter);
    assert!(app.label_dialog.is_some());
    assert_eq!(app.session.editor.source(), source);
    Ok(())
}

#[test]
fn saved_custom_labels_remain_choices_after_unassigning_and_restarting() -> anyhow::Result<()> {
    let (directory, mut app) = managed_application()?;
    let root = app.session.editor.path.parent().unwrap().to_path_buf();
    let label = ResumeLabel::new("Systems engineer", LabelColor::Teal)?;
    // Discover an existing custom label created before the reusable catalog existed.
    resume_project::Project::load(&root)?.set_label(Some(label.clone()))?;
    app.refresh_projects();
    app.open_label_dialog();
    assert!(app.label_dialog.as_ref().unwrap().choices.contains(&label));
    key(&mut app, KeyCode::Home);
    key(&mut app, KeyCode::Enter);
    assert!(
        resume_project::Project::load(&root)?
            .manifest
            .label
            .is_none()
    );

    let second = resume_project::create(directory.path(), "Second")?;
    let mut reopened = App::with_workspace(
        Editor::open(second.document())?,
        Preview::default(),
        directory.path().to_path_buf(),
    )?;
    reopened.open_label_dialog();
    let choices = &reopened.label_dialog.as_ref().unwrap().choices;
    assert_eq!(choices.iter().filter(|choice| **choice == label).count(), 1);
    let index = choices.iter().position(|choice| *choice == label).unwrap();
    for _ in 0..=index {
        key(&mut reopened, KeyCode::Char('j'));
    }
    key(&mut reopened, KeyCode::Enter);
    assert_eq!(
        resume_project::Project::load(&second.root)?.manifest.label,
        Some(label)
    );
    Ok(())
}
