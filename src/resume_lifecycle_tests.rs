use super::test_support::*;
use super::*;
use crate::assistant_job::CompletedTask;
use anyhow::{Result, ensure};
use ratatui::{Terminal, backend::TestBackend};
use serde_json::{Value, json};
use std::{fs, path::Path, process::Command, sync::atomic::AtomicBool};
use texo::{
    agent::{self, AgentTurn},
    llm::AssistantReply,
    resume_tools::ResumeTools,
    tools::ToolCall,
};

const SCREEN: Rect = Rect::new(0, 0, 160, 45);
const FULL_RESUME: &str = include_str!("../tests/fixtures/resume.tex");

fn press(app: &mut App, code: KeyCode) {
    app.handle_event(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)), SCREEN);
}

fn tool_turn(calls: Vec<ToolCall>) -> AgentTurn {
    AgentTurn::Tools {
        calls,
        continuation: json!([]),
    }
}

fn tool_call(id: &str, name: &str, arguments: Value) -> ToolCall {
    ToolCall {
        id: id.into(),
        name: name.into(),
        arguments,
    }
}

// Script model responses while running real tools and sandbox compilation.
fn scripted_revision(context: &str, document: &Path, edit: ToolCall) -> Result<CompletedTask> {
    let mut tools = ResumeTools::new("resume.tex".into(), context.into())
        .in_directory(document.parent().unwrap().to_owned());

    let reply = agent::run(
        "Revise the synthetic resume using the supplied facts",
        &mut tools,
        &AtomicBool::new(false),
        |_, _, history, _| {
            for exchange in history {
                for result in &exchange.results {
                    ensure!(!result.is_error, "Tool failed: {:?}", result.output);
                }
            }

            Ok(match history.len() {
                0 => tool_turn(vec![tool_call("read", "read_resume", json!({}))]),
                1 => {
                    assert_eq!(history[0].results[0].output["tex"], context);
                    tool_turn(vec![
                        edit.clone(),
                        tool_call("compile", "compile_resume", json!({})),
                    ])
                }
                _ => AgentTurn::Complete(AssistantReply {
                    reply: "Updated the requested draft for review.".into(),
                    proposed_tex: None,
                }),
            })
        },
    )?;

    Ok(CompletedTask {
        reply,
        build: tools.take_build(),
    })
}

fn request_revision(app: &mut App, expected_context: &str, edit: ToolCall) -> Result<()> {
    let context = app.assistant_source();
    assert_eq!(context, expected_context);
    let live_source = app.session.editor.source();
    let saved_source = fs::read_to_string(&app.session.editor.path)?;
    let sender = app.session.assistant.pending_request(context.clone());
    let completed = scripted_revision(&context, &app.session.editor.path, edit)?;

    sender.send(Ok(completed))?;
    app.receive_assistant();
    wait_for_reviews(app)?;

    assert!(app.session.proposal_review.is_some());
    assert!(app.session.proposed_build.is_some());
    assert_eq!(app.session.editor.source(), live_source);
    assert_eq!(fs::read_to_string(&app.session.editor.path)?, saved_source);
    Ok(())
}

fn replace_text(old_text: &str, new_text: &str) -> ToolCall {
    tool_call(
        "edit",
        "replace_text",
        json!({"old_text": old_text, "new_text": new_text}),
    )
}

fn pdf_text(path: &Path) -> Result<String> {
    let output = Command::new("pdftotext").arg(path).arg("-").output()?;
    ensure!(
        output.status.success(),
        "Could not extract exported PDF text"
    );
    Ok(String::from_utf8(output.stdout)?)
}

#[test]
#[ignore = "requires macOS, Tectonic cache, and Poppler"]
fn empty_workspace_to_revised_resume_export_and_reopen() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let root = directory.path().canonicalize()?;
    let placeholder = root.join("resume.tex");
    let mut app = App::with_workspace(
        Editor::placeholder(placeholder.clone())?,
        Preview::default(),
        root.clone(),
    )?;
    app.session.empty_workspace = true;
    assert!(app.projects.catalog.projects.is_empty());

    app.perform_action(Action::NewResume);
    app.project_dialog.as_mut().unwrap().name.select_all();
    app.handle_event(Event::Paste("Alex's resume".into()), SCREEN);
    press(&mut app, KeyCode::Enter);
    wait_for_reviews(&mut app)?;
    assert!(!app.session.empty_workspace);
    assert!(!placeholder.exists());
    assert_eq!(app.projects.catalog.projects.len(), 1);
    let document = app.session.editor.path.clone();
    let published_pdf = document.with_extension("pdf");
    let starter_pdf = fs::read(&published_pdf)?;

    request_revision(
        &mut app,
        crate::editor::STARTER_DOCUMENT,
        tool_call("edit", "propose_edit", json!({"tex": FULL_RESUME})),
    )?;
    app.handle_prefix_action(KeyCode::Char('m'), SCREEN);
    request_revision(
        &mut app,
        FULL_RESUME,
        replace_text("Built reporting tools.", "Built reporting tools in Rust."),
    )?;
    assert_eq!(fs::read(&published_pdf)?, starter_pdf);
    assert_eq!(
        fs::read_to_string(&document)?,
        crate::editor::STARTER_DOCUMENT
    );

    let refined = FULL_RESUME.replace("Built reporting tools.", "Built reporting tools in Rust.");
    app.handle_prefix_action(KeyCode::Char('a'), SCREEN);
    assert_eq!(app.session.editor.source(), refined);
    assert!(app.session.editor.is_modified());
    let accepted_pdf = fs::read(&published_pdf)?;
    assert_ne!(accepted_pdf, starter_pdf);

    request_revision(
        &mut app,
        &refined,
        replace_text("Software engineer.", "Unwanted revision."),
    )?;
    app.handle_prefix_action(KeyCode::Char('d'), SCREEN);
    assert!(app.session.proposal_review.is_none());
    assert_eq!(app.session.editor.source(), refined);
    assert_eq!(fs::read(&published_pdf)?, accepted_pdf);

    request_revision(
        &mut app,
        &refined,
        replace_text("Rust, Python, SQL.", "Rust, Python, SQL, Git."),
    )?;
    app.handle_prefix_action(KeyCode::Char('a'), SCREEN);
    let final_source = refined.replace("Rust, Python, SQL.", "Rust, Python, SQL, Git.");
    assert_eq!(app.session.editor.source(), final_source);
    app.handle_prefix_action(KeyCode::Char('w'), SCREEN);
    assert!(!app.session.editor.is_modified());
    assert_eq!(fs::read_to_string(&document)?, final_source);

    let exported = root.join("application.pdf");
    app.open_export();
    app.export.as_mut().unwrap().folder.select_all();
    app.handle_event(
        Event::Paste(exported.parent().unwrap().display().to_string()),
        SCREEN,
    );
    press(&mut app, KeyCode::Tab);
    app.export.as_mut().unwrap().input.select_all();
    app.handle_event(Event::Paste("application.pdf".into()), SCREEN);
    press(&mut app, KeyCode::Enter);
    assert!(app.export.is_none());
    assert_eq!(fs::read(&exported)?, fs::read(&published_pdf)?);
    let text = pdf_text(&exported)?;
    for expected in ["Alex Example", "Experience", "Education", "in Rust", "Git"] {
        assert!(
            text.contains(expected),
            "Export is missing {expected}: {text}"
        );
    }
    assert!(!text.contains("Unwanted revision"));

    drop(app);
    let mut reopened = App::start(Editor::open(document)?, Preview::default(), root)?;
    let mut terminal = Terminal::new(TestBackend::new(SCREEN.width, SCREEN.height))?;
    wait_for_visible_pdf(&mut reopened, &mut terminal)?;
    assert_eq!(reopened.session.editor.source(), final_source);
    assert_eq!(
        reopened.session.preview.page_status().as_deref(),
        Some("1 / 1")
    );
    assert_eq!(
        reopened.projects.catalog.projects[0].manifest.name,
        "Alex's resume"
    );
    assert_eq!(pdf_text(&exported)?, text);
    Ok(())
}

fn built_resume() -> Result<(tempfile::TempDir, App)> {
    let directory = tempfile::tempdir()?;
    let root = directory.path().canonicalize()?;
    let project = resume_project::create(&root, "Lifecycle resume")?;
    fs::write(project.document(), FULL_RESUME)?;
    let mut app = App::start(Editor::open(project.document())?, Preview::default(), root)?;
    wait_for_reviews(&mut app)?;
    ensure!(
        app.session.rendered_pdf.is_some(),
        "Initial PDF did not build"
    );
    Ok((directory, app))
}

#[test]
#[ignore = "requires macOS, Tectonic cache, and Poppler"]
fn export_rechecks_pending_review_before_overwriting_an_existing_pdf() -> Result<()> {
    let (directory, mut app) = built_resume()?;
    let exported = directory.path().join("application.pdf");
    fs::write(&exported, "Existing application")?;
    let sender = app.session.assistant.pending_request(FULL_RESUME.into());
    app.open_export();
    app.export.as_mut().unwrap().folder.select_all();
    app.handle_event(
        Event::Paste(exported.parent().unwrap().display().to_string()),
        SCREEN,
    );
    press(&mut app, KeyCode::Tab);
    app.export.as_mut().unwrap().input.select_all();
    app.handle_event(Event::Paste("application.pdf".into()), SCREEN);
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.export.as_ref().unwrap().message, "File exists.");

    // A reply can arrive between the overwrite warning and its confirmation.
    let completed = scripted_revision(
        FULL_RESUME,
        &app.session.editor.path,
        replace_text("Software engineer.", "Backend engineer."),
    )?;
    sender.send(Ok(completed))?;
    app.receive_assistant();
    wait_for_reviews(&mut app)?;
    press(&mut app, KeyCode::Enter);
    assert!(
        fs::read(&exported)? == b"Existing application",
        "The existing export must survive until the new proposal is reviewed"
    );
    assert!(app.session.status.contains("Accept or discard"));

    app.discard_proposal();
    app.open_export();
    assert!(app.export.is_some());
    Ok(())
}

#[test]
#[ignore = "requires macOS, Tectonic cache, and Poppler"]
fn failed_asset_rebuild_blocks_export_until_a_successful_retry() -> Result<()> {
    let (_directory, mut app) = built_resume()?;
    let asset = app
        .session
        .editor
        .path
        .parent()
        .unwrap()
        .join("experience.tex");
    fs::write(&asset, "Original experience")?;
    let source = FULL_RESUME.replace("Built reporting tools.", r"\input{experience}");
    app.session.editor.replace_source(&source);
    app.start_render();
    wait_for_reviews(&mut app)?;
    let published = app.session.rendered_pdf.clone().unwrap();
    let previous_pdf = fs::read(&published)?;

    fs::write(&asset, r"\undefinedResumeCommand")?;
    app.start_render();
    wait_for_reviews(&mut app)?;
    assert!(app.session.render_failed);
    assert_eq!(fs::read(&published)?, previous_pdf);
    app.open_export();
    assert!(app.export.is_none());

    fs::write(&asset, "Repaired experience")?;
    app.start_render();
    wait_for_reviews(&mut app)?;
    assert!(!app.session.render_failed);
    app.open_export();
    assert!(app.export.is_some());
    assert!(pdf_text(&published)?.contains("Repaired experience"));
    Ok(())
}

#[test]
#[ignore = "requires macOS, Tectonic cache, and Poppler"]
fn failed_pdf_publish_preserves_the_review_and_can_be_retried() -> Result<()> {
    let (_directory, mut app) = built_resume()?;
    let document = app.session.editor.path.clone();
    let published = document.with_extension("pdf");
    request_revision(
        &mut app,
        FULL_RESUME,
        replace_text("Software engineer.", "Backend engineer."),
    )?;
    let draft_pdf = app
        .session
        .proposed_build
        .as_ref()
        .unwrap()
        .pdf_path
        .clone();

    // Simulate an output-path conflict that appears after the draft compiled.
    fs::remove_file(&published)?;
    fs::create_dir(&published)?;
    app.apply_proposal();
    assert_eq!(app.session.editor.source(), FULL_RESUME);
    assert_eq!(fs::read_to_string(&document)?, FULL_RESUME);
    assert!(app.session.proposal_review.is_some());
    assert!(draft_pdf.is_file());
    assert!(app.session.diagnostics.is_some());

    fs::remove_dir(&published)?;
    app.apply_proposal();
    assert!(app.session.proposal_review.is_none());
    assert!(app.session.diagnostics.is_none());
    assert!(!draft_pdf.exists());
    assert!(pdf_text(&published)?.contains("Backend engineer"));
    app.handle_control_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
    assert!(fs::read_to_string(&document)?.contains("Backend engineer"));
    app.open_export();
    assert!(app.export.is_some());
    Ok(())
}

#[test]
#[ignore = "requires macOS, Tectonic cache, and Poppler"]
fn late_refinement_follows_its_project_and_preserves_the_other_resume() -> Result<()> {
    let (directory, mut app) = built_resume()?;
    let first = app.session.editor.path.clone();
    let second = resume_project::create(&directory.path().canonicalize()?, "Other resume")?;
    request_revision(
        &mut app,
        FULL_RESUME,
        replace_text("Software engineer.", "Backend engineer."),
    )?;
    let pending = FULL_RESUME.replace("Software engineer.", "Backend engineer.");
    let sender = app.session.assistant.pending_request(pending.clone());
    let completed = scripted_revision(
        &pending,
        &first,
        replace_text("Rust, Python, SQL.", "Rust, Python, SQL, Git."),
    )?;
    app.open_document(second.document());
    wait_for_reviews(&mut app)?;
    let second_pdf = fs::read(second.document().with_extension("pdf"))?;
    sender.send(Ok(completed))?;
    app.receive_assistant();
    assert!(app.session.proposal_review.is_none());
    assert_eq!(app.session.editor.source(), crate::editor::STARTER_DOCUMENT);

    app.open_document(first.clone());
    app.receive_assistant();
    wait_for_reviews(&mut app)?;
    assert_eq!(app.session.editor.source(), FULL_RESUME);
    app.apply_proposal();
    let accepted = pending.replace("Rust, Python, SQL.", "Rust, Python, SQL, Git.");
    assert_eq!(app.session.editor.source(), accepted);
    assert_eq!(fs::read_to_string(&first)?, FULL_RESUME);
    let text = pdf_text(&first.with_extension("pdf"))?;
    assert!(text.contains("Backend engineer") && text.contains("Git"));

    app.open_document(second.document());
    assert_eq!(
        fs::read(second.document().with_extension("pdf"))?,
        second_pdf
    );
    assert_eq!(
        fs::read_to_string(second.document())?,
        crate::editor::STARTER_DOCUMENT
    );
    app.open_document(first);
    assert_eq!(app.session.editor.source(), accepted);
    assert!(app.session.editor.is_modified());
    Ok(())
}

#[test]
#[ignore = "requires macOS, Tectonic cache, and Poppler"]
fn failed_refinement_and_cancelled_reply_cannot_destroy_an_existing_draft() -> Result<()> {
    let (_directory, mut app) = built_resume()?;
    let document = app.session.editor.path.clone();
    let published_pdf = fs::read(document.with_extension("pdf"))?;
    request_revision(
        &mut app,
        FULL_RESUME,
        replace_text("Software engineer.", "Backend engineer."),
    )?;
    let pending = app.assistant_source();
    let draft_pdf = app
        .session
        .proposed_build
        .as_ref()
        .unwrap()
        .pdf_path
        .clone();
    let sender = app.session.assistant.pending_request(pending.clone());
    let failed = scripted_revision(
        &pending,
        &document,
        replace_text("Backend engineer.", r"\undefinedResumeCommand"),
    );
    assert!(failed.is_err());
    sender.send(failed)?;
    app.receive_assistant();
    assert_eq!(app.assistant_source(), pending);
    assert!(draft_pdf.is_file());
    assert_eq!(fs::read(document.with_extension("pdf"))?, published_pdf);

    let sender = app.session.assistant.pending_request(pending.clone());
    let late_reply = scripted_revision(
        &pending,
        &document,
        replace_text("Rust, Python, SQL.", "Rust, Python, SQL, Git."),
    )?;
    app.discard_proposal();
    assert!(sender.send(Ok(late_reply)).is_err());
    app.receive_assistant();
    assert!(!draft_pdf.exists());
    assert!(app.session.proposal_review.is_none());
    assert_eq!(app.session.editor.source(), FULL_RESUME);
    assert_eq!(fs::read_to_string(&document)?, FULL_RESUME);
    assert_eq!(fs::read(document.with_extension("pdf"))?, published_pdf);

    request_revision(
        &mut app,
        FULL_RESUME,
        replace_text("Software engineer.", "Fresh request."),
    )?;
    app.apply_proposal();
    assert!(pdf_text(&document.with_extension("pdf"))?.contains("Fresh request"));
    Ok(())
}

#[test]
#[ignore = "requires macOS, Tectonic cache, and Poppler"]
fn external_source_change_blocks_save_and_reload_cancels_pending_work() -> Result<()> {
    let (_directory, mut app) = built_resume()?;
    let document = app.session.editor.path.clone();
    request_revision(
        &mut app,
        FULL_RESUME,
        replace_text("Software engineer.", "Backend engineer."),
    )?;
    app.apply_proposal();
    let accepted = app.session.editor.source();
    let external = FULL_RESUME.replace("Software engineer.", "External edit.");
    fs::write(&document, &external)?;
    app.handle_control_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
    assert_eq!(fs::read_to_string(&document)?, external);
    assert_eq!(app.session.editor.source(), accepted);
    assert!(app.session.editor.is_modified());
    assert!(app.diagnostic_details_open());
    press(&mut app, KeyCode::Esc);

    let sender = app.session.assistant.pending_request(accepted);
    app.perform_action(Action::ReloadSource);
    press(&mut app, KeyCode::Right);
    press(&mut app, KeyCode::Enter);
    wait_for_reviews(&mut app)?;
    assert!(sender.send(Err(anyhow::anyhow!("Late result"))).is_err());
    assert!(!app.session.assistant.is_running());
    assert!(!app.session.editor.is_modified());
    assert_eq!(app.session.editor.source(), external);
    assert!(pdf_text(&document.with_extension("pdf"))?.contains("External edit"));
    app.open_export();
    assert!(app.export.is_some());
    Ok(())
}
