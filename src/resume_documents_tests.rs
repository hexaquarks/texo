use std::{fs, sync::atomic::AtomicBool};

use anyhow::Result;
use serde_json::json;
use texo::{llm::AssistantReply, preview::Preview, resume_tools::ResumeTools, tools::ToolSet};

use super::App;
use crate::{
    assistant::ProposedEdit,
    editor::Editor,
    proposal::ProposalReview,
    resume_project::{self, Project},
    workspace::Workspace,
};

fn document_tools(app: &App) -> Result<ResumeTools> {
    let name = app
        .session
        .editor
        .path
        .file_name()
        .unwrap()
        .to_string_lossy();
    let workspace = app.assistant_workspace()?;
    Ok(ResumeTools::new(name.into_owned(), app.assistant_source())
        .in_workspace(Box::new(workspace)))
}

#[test]
fn list_read_and_copy_use_the_catalog_and_preserve_the_live_editor() -> Result<()> {
    let workspace = tempfile::tempdir()?;
    let root = workspace.path().canonicalize()?;
    let original = resume_project::create_from_source(&root, "Original", "Saved source")?;
    let other = resume_project::create_from_source(&root, "Other", "Other source")?;
    fs::write(original.root.join("asset.txt"), "Supporting asset")?;
    fs::write(root.join("loose.tex"), "Unmanaged source")?;

    let editor = Editor::open(original.document())?;
    let mut app = App::with_workspace(editor, Preview::default(), root)?;
    app.session.editor.replace_source("Unsaved editor source");
    app.session.proposal_review = Some(ProposalReview::new(&ProposedEdit {
        source_before: app.session.editor.source(),
        source_after: "Pending proposal".into(),
    }));
    let mut tools = document_tools(&app)?;
    let cancelled = AtomicBool::new(false);
    let original_id = original.manifest.id.to_string();
    let other_id = other.manifest.id.to_string();

    let list = tools.execute("list_resumes", &json!({}), &cancelled)?;
    assert_eq!(list["documents"].as_array().unwrap().len(), 2);
    assert_eq!(list["documents"][0]["id"], original_id);
    assert_eq!(list["documents"][0]["current"], true);
    assert_eq!(list["documents"][1]["id"], other_id);

    let read_current = json!({"document_id": original_id});
    let read_other = json!({"document_id": other_id});
    assert_eq!(
        tools.execute("read_resume", &read_current, &cancelled)?["tex"],
        "Pending proposal"
    );
    assert_eq!(
        tools.execute("read_resume", &read_other, &cancelled)?["tex"],
        "Other source"
    );

    // Same display names retain independent IDs, directories, and source files.
    let copy = json!({"name": "Other", "source_document_id": "current"});
    let result = tools.execute("create_resume", &copy, &cancelled)?;
    let copy_id = result["created"]["id"].as_str().unwrap();
    assert_ne!(copy_id, other_id);
    assert_eq!(result["saved"], true);
    assert_eq!(
        tools.execute("read_resume", &json!({"document_id": copy_id}), &cancelled)?["tex"],
        "Unsaved editor source"
    );

    app.receive_assistant();
    let copy = app
        .projects
        .catalog
        .projects
        .iter()
        .find(|project| project.manifest.id.to_string() == copy_id)
        .unwrap();
    assert_ne!(copy.root, other.root);
    assert_eq!(
        fs::read_to_string(copy.root.join("asset.txt"))?,
        "Supporting asset"
    );
    assert_eq!(app.session.editor.path, original.document());
    assert_eq!(app.session.editor.source(), "Unsaved editor source");
    assert_eq!(fs::read_to_string(original.document())?, "Saved source");
    assert_eq!(fs::read_to_string(other.document())?, "Other source");

    tools.execute("propose_edit", &json!({"tex": "Updated draft"}), &cancelled)?;
    assert!(
        tools
            .finish(AssistantReply {
                reply: "Edited".into(),
                proposed_tex: None,
            })
            .is_err()
    );
    assert_eq!(
        tools.execute("read_resume", &read_current, &cancelled)?["tex"],
        "Updated draft"
    );
    assert_eq!(app.session.editor.source(), "Unsaved editor source");
    assert_eq!(fs::read_to_string(original.document())?, "Saved source");
    assert_eq!(
        fs::read_to_string(copy.document())?,
        "Unsaved editor source"
    );

    Ok(())
}

#[test]
fn empty_workspace_supports_creation_and_disables_draft_edits() -> Result<()> {
    let workspace = tempfile::tempdir()?;
    let root = workspace.path().canonicalize()?;
    let editor = Editor::placeholder(Workspace::placeholder_path(&root))?;
    let mut app = App::with_workspace(editor, Preview::default(), root)?;
    app.session.empty_workspace = true;
    let mut tools = document_tools(&app)?;
    let cancelled = AtomicBool::new(false);

    assert_eq!(
        tools.execute("list_resumes", &json!({}), &cancelled)?["documents"],
        json!([])
    );
    assert!(
        tools
            .execute("read_resume", &json!({}), &cancelled)
            .is_err()
    );
    assert!(
        !tools
            .definitions()
            .iter()
            .any(|tool| tool.name == "propose_edit")
    );

    let create = json!({"name": "First resume", "tex": "First source"});
    let result = tools.execute("create_resume", &create, &cancelled)?;
    let read = json!({"document_id": result["created"]["id"]});
    assert_eq!(
        tools.execute("read_resume", &read, &cancelled)?["tex"],
        "First source"
    );

    app.receive_assistant();
    let project = &app.projects.catalog.projects[0];
    assert_eq!(fs::read_to_string(project.document())?, "First source");
    assert_ne!(app.session.editor.path, project.document());
    assert!(app.session.empty_workspace);

    Ok(())
}

#[test]
fn current_copy_can_recover_a_missing_entrypoint_from_the_editor() -> Result<()> {
    let workspace = tempfile::tempdir()?;
    let root = workspace.path().canonicalize()?;
    let original = resume_project::create_from_source(&root, "Original", "Saved source")?;
    let editor = Editor::open(original.document())?;
    let mut app = App::with_workspace(editor, Preview::default(), root)?;
    app.session.editor.replace_source("Unsaved editor source");
    fs::remove_file(original.document())?;

    let mut tools = document_tools(&app)?;
    let cancelled = AtomicBool::new(false);
    let copy = json!({"name": "Recovered copy", "source_document_id": "current"});
    let result = tools.execute("create_resume", &copy, &cancelled)?;
    let read = json!({"document_id": result["created"]["id"]});

    assert_eq!(
        tools.execute("read_resume", &read, &cancelled)?["tex"],
        "Unsaved editor source"
    );
    assert!(!original.document().exists());
    Ok(())
}

#[cfg(unix)]
#[test]
fn assistant_documents_reject_paths_and_symlinked_sources() -> Result<()> {
    use std::os::unix::fs::symlink;

    let workspace = tempfile::tempdir()?;
    let outside = tempfile::tempdir()?;
    let root = workspace.path().canonicalize()?;
    let original = resume_project::create_from_source(&root, "Original", "Original source")?;
    let linked = resume_project::create_from_source(&root, "Linked", "Old source")?;
    let external =
        resume_project::create_from_source(outside.path(), "External", "Private source")?;
    let editor = Editor::open(original.document())?;
    let app = App::with_workspace(editor, Preview::default(), root.clone())?;
    let mut tools = document_tools(&app)?;
    let cancelled = AtomicBool::new(false);

    fs::remove_file(linked.document())?;
    symlink(external.document(), linked.document())?;
    symlink(&external.root, root.join("external"))?;
    symlink(external.document(), original.root.join("asset.tex"))?;

    let selectors = [
        json!("../external/main.tex"),
        json!(external.document()),
        json!(external.manifest.id),
        json!(linked.manifest.id),
    ];
    for selector in selectors {
        let read = json!({"document_id": selector});
        let copy = json!({"name": "Invalid copy", "source_document_id": selector});
        assert!(tools.execute("read_resume", &read, &cancelled).is_err());
        assert!(tools.execute("create_resume", &copy, &cancelled).is_err());
    }

    let copy = json!({"name": "Invalid asset copy", "source_document_id": "current"});
    assert!(tools.execute("create_resume", &copy, &cancelled).is_err());
    let documents = tools.execute("list_resumes", &json!({}), &cancelled)?;
    assert_eq!(documents["documents"].as_array().unwrap().len(), 1);
    assert_eq!(fs::read_to_string(external.document())?, "Private source");

    // A replacement project in the original folder must not inherit the request's unsaved copy.
    let replacement =
        resume_project::create_from_source(&root, "Replacement", "Replacement source")?;
    fs::remove_dir_all(&original.root)?;
    fs::rename(&replacement.root, &original.root)?;
    assert!(tools.execute("create_resume", &copy, &cancelled).is_err());
    assert_eq!(
        Project::load(&original.root)?.manifest.id,
        replacement.manifest.id
    );

    Ok(())
}
