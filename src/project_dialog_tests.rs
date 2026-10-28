use super::test_support::*;
use super::*;
use crate::resume_project::{ImportPlan, Project};
use ratatui::{Terminal, backend::TestBackend};

const SCREEN: Rect = Rect::new(0, 0, 100, 30);

fn press(app: &mut App, code: KeyCode) {
    app.handle_event(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)), SCREEN);
}

fn rendered_dialog(app: &mut App) -> anyhow::Result<String> {
    let mut terminal = Terminal::new(TestBackend::new(SCREEN.width, SCREEN.height))?;
    terminal.draw(|frame| app.project_dialog.as_mut().unwrap().draw(frame))?;

    Ok(terminal
        .backend()
        .buffer()
        .content
        .chunks(SCREEN.width as usize)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n"))
}

#[test]
fn failed_import_returns_to_editable_fields_and_reinspects_the_source() -> anyhow::Result<()> {
    for candidate_count in [1, 2] {
        let (_workspace, mut app) = application()?;
        let source = tempfile::tempdir()?;
        let first_source = source.path().join("first.tex");
        std::fs::write(&first_source, crate::editor::STARTER_DOCUMENT)?;
        if candidate_count == 2 {
            std::fs::write(
                source.path().join("second.tex"),
                crate::editor::STARTER_DOCUMENT,
            )?;
        }

        app.open_project_dialog(Action::ImportResume);
        let dialog = app.project_dialog.as_mut().unwrap();
        dialog.source.insert_str(source.path().to_string_lossy());
        dialog.name.select_all();
        dialog.name.delete_next_char();
        assert_eq!(dialog.name.lines(), &[""]);
        press(&mut app, KeyCode::Enter);
        if candidate_count == 2 {
            press(&mut app, KeyCode::Down);
            press(&mut app, KeyCode::Enter);
        }

        let dialog = app.project_dialog.as_ref().unwrap();
        assert!(!dialog.error.is_empty());
        assert!(
            dialog.plan.is_none(),
            "A failed import must allow correcting its fields"
        );
        assert!(app.projects.catalog.projects.is_empty());
        assert!(rendered_dialog(&mut app)?.contains("Display name"));

        // Change a two-entrypoint folder to a single file. Its old selection must not survive.
        let dialog = app.project_dialog.as_mut().unwrap();
        dialog.source.select_all();
        app.handle_event(
            Event::Paste(first_source.to_string_lossy().into_owned()),
            SCREEN,
        );
        press(&mut app, KeyCode::Tab);
        app.handle_event(Event::Paste("Recovered import".into()), SCREEN);
        press(&mut app, KeyCode::Enter);
        app.session.job = None;

        assert!(app.project_dialog.is_none());
        assert_eq!(app.projects.catalog.projects.len(), 1);
        let imported = &app.projects.catalog.projects[0];
        assert_eq!(imported.manifest.name, "Recovered import");
        assert_eq!(
            imported.manifest.entrypoint,
            std::path::Path::new("first.tex")
        );
        assert_eq!(
            std::fs::read_to_string(&first_source)?,
            crate::editor::STARTER_DOCUMENT
        );
    }
    Ok(())
}

#[test]
fn import_dialog_reports_missing_sources_and_can_be_cancelled_without_changes() -> anyhow::Result<()>
{
    let (workspace, mut app) = application()?;
    let original_path = app.session.editor.path.clone();
    let original_text = app.session.editor.source();
    app.open_project_dialog(Action::ImportResume);
    app.project_dialog
        .as_mut()
        .unwrap()
        .source
        .insert_str(workspace.path().join("missing.tex").to_string_lossy());
    press(&mut app, KeyCode::Enter);

    assert!(app.project_dialog.as_ref().unwrap().plan.is_none());
    assert!(rendered_dialog(&mut app)?.contains("Choose an existing TeX file"));
    press(&mut app, KeyCode::Esc);
    assert!(app.project_dialog.is_none());
    assert_eq!(app.session.editor.path, original_path);
    assert_eq!(app.session.editor.source(), original_text);
    assert!(app.projects.catalog.projects.is_empty());
    Ok(())
}

#[test]
fn import_entrypoint_picker_shows_choices_and_bounds_navigation() -> anyhow::Result<()> {
    let (_workspace, mut app) = application()?;
    let source = tempfile::tempdir()?;
    for name in ["first.tex", "second.tex"] {
        std::fs::write(source.path().join(name), crate::editor::STARTER_DOCUMENT)?;
    }
    app.open_project_dialog(Action::ImportResume);
    app.project_dialog
        .as_mut()
        .unwrap()
        .source
        .insert_str(source.path().to_string_lossy());
    press(&mut app, KeyCode::Enter);

    let screen = rendered_dialog(&mut app)?;
    assert!(screen.contains("Choose the TeX entrypoint"));
    assert!(screen.contains("first.tex"));
    assert!(screen.contains("second.tex"));
    for code in [KeyCode::Up, KeyCode::Char('k')] {
        press(&mut app, code);
        assert_eq!(
            app.project_dialog.as_ref().unwrap().selection.selected(),
            Some(0)
        );
    }
    for code in [KeyCode::Down, KeyCode::Char('j'), KeyCode::Down] {
        press(&mut app, code);
        assert_eq!(
            app.project_dialog.as_ref().unwrap().selection.selected(),
            Some(1)
        );
    }
    press(&mut app, KeyCode::Esc);
    assert!(app.project_dialog.is_none());
    assert!(app.projects.catalog.projects.is_empty());
    Ok(())
}

#[test]
fn rename_and_duplicate_preserve_unsaved_original_and_copy_supporting_assets() -> anyhow::Result<()>
{
    let workspace = tempfile::tempdir()?;
    let root = workspace.path().canonicalize()?;
    let original = resume_project::create(&root, "Original")?;
    std::fs::write(original.root.join("support.txt"), "Supporting asset")?;
    let mut app =
        App::with_workspace(Editor::open(original.document())?, Preview::default(), root)?;
    app.session.editor.text.insert_str("Unsaved ");
    let unsaved = app.session.editor.source();

    app.perform_action(Action::RenameResume);
    assert!(rendered_dialog(&mut app)?.contains("Rename resume"));
    app.handle_event(Event::Paste("Renamed".into()), SCREEN);
    press(&mut app, KeyCode::Enter);
    assert!(app.project_dialog.is_none());
    assert_eq!(Project::load(&original.root)?.manifest.name, "Renamed");
    assert_eq!(app.session.editor.source(), unsaved);
    assert_eq!(
        std::fs::read_to_string(original.document())?,
        crate::editor::STARTER_DOCUMENT
    );

    app.perform_action(Action::DuplicateResume);
    assert!(rendered_dialog(&mut app)?.contains("Renamed copy"));
    press(&mut app, KeyCode::Enter);
    app.session.job = None;
    let duplicate = app.projects.project_for(&app.session.editor.path).unwrap();
    assert_ne!(duplicate.manifest.id, original.manifest.id);
    assert_eq!(duplicate.manifest.name, "Renamed copy");
    assert_eq!(std::fs::read_to_string(duplicate.document())?, unsaved);
    assert_eq!(
        std::fs::read_to_string(duplicate.root.join("support.txt"))?,
        "Supporting asset"
    );

    app.open_document(original.document());
    assert_eq!(app.session.editor.source(), unsaved);
    assert!(app.session.editor.is_modified());
    Ok(())
}

#[test]
fn reload_confirmation_discards_local_changes_only_after_successful_read() -> anyhow::Result<()> {
    let workspace = tempfile::tempdir()?;
    let project = resume_project::create(workspace.path(), "Original")?;
    let mut app = App::with_workspace(
        Editor::open(project.document())?,
        Preview::default(),
        workspace.path().to_owned(),
    )?;
    app.session.editor.text.insert_str("Unsaved ");
    let unsaved = app.session.editor.source();
    app.session.proposal_review = Some(ProposalReview::new(&ProposedEdit {
        source_before: unsaved.clone(),
        source_after: "Proposed text".into(),
    }));
    std::fs::remove_file(project.document())?;

    app.perform_action(Action::ReloadSource);
    assert!(rendered_dialog(&mut app)?.contains("discard local edits"));
    press(&mut app, KeyCode::Enter);
    assert!(app.project_dialog.is_some());
    assert_eq!(app.session.editor.source(), unsaved);
    assert!(app.session.proposal_review.is_some());

    std::fs::write(project.document(), "Changed on disk")?;
    press(&mut app, KeyCode::Enter);
    app.session.job = None;
    assert!(app.project_dialog.is_none());
    assert_eq!(app.session.editor.source(), "Changed on disk");
    assert!(!app.session.editor.is_modified());
    assert!(app.session.proposal_review.is_none());
    assert!(app.session.rendered_pdf.is_none());
    Ok(())
}

#[test]
fn project_dialogs_fit_small_terminals_and_show_action_specific_titles() -> anyhow::Result<()> {
    let (workspace, mut app) = application()?;
    for (action, title) in [
        (Action::NewResume, "New resume"),
        (Action::ImportResume, "Import resume"),
        (Action::RenameResume, "Rename resume"),
        (Action::DuplicateResume, "Duplicate resume"),
        (Action::RecoverSource, "Restore missing source"),
    ] {
        app.project_dialog = Some(ProjectDialog::new(action, None, None));
        assert!(rendered_dialog(&mut app)?.contains(title));
        for (width, height) in [(1, 1), (20, 5)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height))?;
            terminal.draw(|frame| app.project_dialog.as_mut().unwrap().draw(frame))?;
        }
    }

    std::fs::write(
        workspace.path().join("entry.tex"),
        crate::editor::STARTER_DOCUMENT,
    )?;
    app.project_dialog.as_mut().unwrap().plan = Some(ImportPlan::inspect(workspace.path())?);
    let mut terminal = Terminal::new(TestBackend::new(20, 5))?;
    terminal.draw(|frame| app.project_dialog.as_mut().unwrap().draw(frame))?;
    Ok(())
}

#[test]
fn rename_preserves_label_changes_made_after_the_dialog_was_opened() -> anyhow::Result<()> {
    use crate::resume_label::{LabelColor, ResumeLabel};

    let workspace = tempfile::tempdir()?;
    let original = resume_project::create(workspace.path(), "Original")?;
    let mut app = App::with_workspace(
        Editor::open(original.document())?,
        Preview::default(),
        workspace.path().to_owned(),
    )?;
    app.open_project_dialog(Action::RenameResume);
    let label = ResumeLabel::new("Backend", LabelColor::Teal)?;
    Project::load(&original.root)?.set_label(Some(label.clone()))?;

    app.handle_event(Event::Paste("Renamed".into()), SCREEN);
    press(&mut app, KeyCode::Enter);

    let saved = Project::load(&original.root)?;
    assert_eq!(saved.manifest.name, "Renamed");
    assert_eq!(saved.manifest.label, Some(label));
    assert_eq!(saved.manifest.id, original.manifest.id);
    Ok(())
}

#[test]
fn rename_does_not_overwrite_an_externally_corrupted_manifest() -> anyhow::Result<()> {
    let workspace = tempfile::tempdir()?;
    let project = resume_project::create(workspace.path(), "Original")?;
    let mut app = App::with_workspace(
        Editor::open(project.document())?,
        Preview::default(),
        workspace.path().to_owned(),
    )?;
    app.open_project_dialog(Action::RenameResume);
    let manifest = project.root.join(resume_project::MANIFEST_FILE);
    std::fs::write(&manifest, "external unfinished edit")?;
    app.handle_event(Event::Paste("Renamed".into()), SCREEN);
    press(&mut app, KeyCode::Enter);

    assert_eq!(
        std::fs::read_to_string(&manifest)?,
        "external unfinished edit"
    );
    assert!(!app.project_dialog.as_ref().unwrap().error.is_empty());
    assert_eq!(app.session.editor.path, project.document());
    Ok(())
}

#[test]
fn export_requires_reconfirmation_after_destination_changes_and_rejects_stale_builds()
-> anyhow::Result<()> {
    let (workspace, mut app) = application()?;
    let build = workspace.path().join("built.pdf");
    std::fs::write(&build, "compiled PDF")?;
    app.session.rendered_pdf = Some(build.clone());
    app.session.rendered_source = Some(app.session.editor.source());
    let first = workspace.path().join("first.pdf");
    let second = workspace.path().join("second.pdf");
    std::fs::write(&first, "first original")?;
    std::fs::write(&second, "second original")?;
    app.perform_action(Action::Export);
    app.export.as_mut().unwrap().input.select_all();
    app.handle_event(Event::Paste(first.to_string_lossy().into_owned()), SCREEN);
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.export.as_ref().unwrap().message, "File exists.");

    app.export.as_mut().unwrap().input.select_all();
    app.handle_event(Event::Paste(second.to_string_lossy().into_owned()), SCREEN);
    press(&mut app, KeyCode::Enter);
    assert_eq!(std::fs::read_to_string(&second)?, "second original");
    press(&mut app, KeyCode::Enter);
    assert!(app.export.is_none());
    assert_eq!(std::fs::read_to_string(&first)?, "first original");
    assert_eq!(std::fs::read_to_string(&second)?, "compiled PDF");
    assert_eq!(std::fs::read_to_string(&build)?, "compiled PDF");

    app.perform_action(Action::Export);
    app.session.editor.text.insert_str("New draft ");
    press(&mut app, KeyCode::Enter);
    assert!(app.export.is_none());
    assert!(app.session.status.contains("PDF changed while exporting"));
    assert!(!workspace.path().join("resume-export.pdf").exists());
    Ok(())
}
