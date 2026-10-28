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

// Script model responses and deliver results through the assistant's completion channel.
// Run real tool dispatch, sandbox compilation, proposal review, and PDF rendering.
fn request_revision(app: &mut App, expected_context: &str, edit: ToolCall) -> Result<()> {
    let context = app.assistant_source();
    assert_eq!(context, expected_context);
    let live_source = app.session.editor.source();
    let saved_source = fs::read_to_string(&app.session.editor.path)?;
    let sender = app.session.assistant.pending_request(context.clone());
    let mut tools = ResumeTools::new("resume.tex".into(), context)
        .in_directory(app.session.editor.path.parent().unwrap().to_owned());

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
                    assert_eq!(history[0].results[0].output["tex"], expected_context);
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

    sender.send(Ok(CompletedTask {
        reply,
        build: tools.take_build(),
    }))?;
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
    app.perform_action(Action::Export);
    app.export.as_mut().unwrap().input.select_all();
    app.handle_event(
        Event::Paste(exported.to_string_lossy().into_owned()),
        SCREEN,
    );
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
