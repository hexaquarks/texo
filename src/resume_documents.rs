use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use anyhow::{Context, Result, ensure};
use texo::resume_tools::{
    CURRENT_DOCUMENT_ID, DocumentSummary, DocumentWorkspace, MAX_DRAFT_BYTES,
};
use uuid::Uuid;

use crate::{
    editor::STARTER_DOCUMENT,
    resume_project::{self, Catalog, Project},
};

pub struct OpenDocument {
    pub path: PathBuf,
    pub project_id: Option<Uuid>,
    pub source: String,
    pub has_pending_proposal: bool,
}

pub struct WorkspaceDocuments {
    root: PathBuf,
    current: Option<OpenDocument>,
    changed: Arc<AtomicBool>,
}

impl WorkspaceDocuments {
    pub fn new(
        root: &Path,
        canonical_root: &Path,
        mut current: Option<OpenDocument>,
        changed: Arc<AtomicBool>,
    ) -> Result<Self> {
        let resolved_root = root
            .canonicalize()
            .context("Could not open the resume workspace")?;
        ensure!(
            resolved_root == canonical_root,
            "The resume workspace moved"
        );

        // Normalize the workspace prefix without resolving a document symlink.
        if let Some(document) = &mut current {
            let relative = document
                .path
                .strip_prefix(root)
                .context("Resume is outside the workspace")?;
            document.path = canonical_root.join(relative);
        }

        let workspace = Self {
            root: resolved_root,
            current,
            changed,
        };
        if workspace.has_current() {
            workspace.validate_current()?;
        }

        Ok(workspace)
    }

    fn validate_root(&self) -> Result<()> {
        let root = self
            .root
            .canonicalize()
            .context("Could not open the resume workspace")?;
        ensure!(root == self.root, "The resume workspace moved");
        Ok(())
    }

    fn catalog(&self) -> Result<Catalog> {
        self.validate_root()?;
        resume_project::discover(&self.root).context("Could not list resumes")
    }

    fn is_current(&self, project: &Project) -> bool {
        self.current.as_ref().is_some_and(|document| {
            document.project_id == Some(project.manifest.id) && document.path == project.document()
        })
    }

    fn summary(&self, project: &Project) -> DocumentSummary {
        DocumentSummary {
            id: project.manifest.id.to_string(),
            name: project.manifest.name.clone(),
            current: self.is_current(project),
        }
    }

    fn find_project(&self, id: Uuid) -> Result<Project> {
        self.catalog()?
            .projects
            .into_iter()
            .find(|project| project.manifest.id == id)
            .context("That resume is no longer in the workspace")
    }

    fn current_project(&self) -> Result<Option<Project>> {
        self.validate_root()?;
        let current = self.current.as_ref().context("No resume is open")?;
        let relative = current.path.strip_prefix(&self.root)?;
        resume_project::validate_entrypoint(&self.root, relative)?;

        if current.project_id.is_none() {
            return Ok(None);
        }

        // Managed projects are direct children of the workspace.
        let folder = relative
            .iter()
            .next()
            .context("Resume has no project folder")?;
        let project = Project::load(&self.root.join(folder))
            .context("Could not reopen the current resume")?;
        ensure!(
            self.is_current(&project),
            "The open resume changed on disk. Reopen it to continue."
        );

        Ok(Some(project))
    }

    fn copy_project(&self, project: &Project, name: &str) -> Result<Project> {
        let working_copy = self
            .current
            .as_ref()
            .filter(|_| self.is_current(project))
            .map(|current| (project.manifest.entrypoint.clone(), current.source.clone()));

        resume_project::duplicate(&self.root, project, name, working_copy.as_slice())
    }

    fn copy_current(&self, name: &str) -> Result<Project> {
        if let Some(project) = self.current_project()? {
            return self.copy_project(&project, name);
        }

        let current = self.current.as_ref().context("No resume is open")?;
        resume_project::create_from_source(&self.root, name, &current.source)
    }

    fn copy_resume(&self, source_id: &str, name: &str) -> Result<Project> {
        if source_id == CURRENT_DOCUMENT_ID {
            return self.copy_current(name);
        }

        let id = Uuid::parse_str(source_id).context("Use a resume ID from list_resumes")?;
        let is_current = self
            .current
            .as_ref()
            .is_some_and(|current| current.project_id == Some(id));
        if is_current {
            return self.copy_current(name);
        }

        let source = self.find_project(id)?;
        self.copy_project(&source, name)
    }
}

impl DocumentWorkspace for WorkspaceDocuments {
    fn has_current(&self) -> bool {
        self.current.is_some()
    }

    fn has_pending_proposal(&self) -> bool {
        self.current
            .as_ref()
            .is_some_and(|current| current.has_pending_proposal)
    }

    fn validate_current(&self) -> Result<()> {
        self.current_project()?;
        Ok(())
    }

    fn list(&self) -> Result<Vec<DocumentSummary>> {
        let mut documents: Vec<_> = self
            .catalog()?
            .projects
            .iter()
            .map(|project| self.summary(project))
            .collect();
        if let Some(current) = &self.current
            && !documents.iter().any(|document| document.current)
        {
            documents.push(DocumentSummary {
                id: CURRENT_DOCUMENT_ID.into(),
                name: current
                    .path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                current: true,
            });
        }

        Ok(documents)
    }

    fn read(&self, id: &str) -> Result<(DocumentSummary, Option<String>)> {
        let id = Uuid::parse_str(id).context("Use a resume ID from list_resumes")?;
        let project = self.find_project(id)?;
        let document = self.summary(&project);
        if document.current {
            return Ok((document, None));
        }

        let source = read_source(&project.document(), &project.root)?;
        Ok((document, Some(source)))
    }

    fn create(
        &self,
        name: &str,
        source_id: Option<&str>,
        tex: Option<&str>,
    ) -> Result<DocumentSummary> {
        ensure!(
            source_id.is_none() || tex.is_none(),
            "Use either a copy source or new TeX"
        );
        self.validate_root()?;

        let project = match source_id {
            Some(id) => self.copy_resume(id, name)?,
            None => resume_project::create_from_source(
                &self.root,
                name,
                tex.unwrap_or(STARTER_DOCUMENT),
            )?,
        };

        self.changed.store(true, Ordering::Release);
        Ok(self.summary(&project))
    }
}

fn read_source(path: &Path, root: &Path) -> Result<String> {
    let resolved = path
        .canonicalize()
        .context("Could not open the resume source")?;
    ensure!(
        resolved == path && resolved.starts_with(root),
        "Resume source moved"
    );
    ensure!(
        fs::symlink_metadata(&resolved)?.is_file(),
        "Resume source must be a regular file"
    );

    let mut source = String::new();
    fs::File::open(&resolved)
        .context("Could not open the resume source")?
        .take(MAX_DRAFT_BYTES as u64 + 1)
        .read_to_string(&mut source)
        .context("Could not read the resume source")?;

    ensure!(
        source.len() <= MAX_DRAFT_BYTES,
        "Resume source is too large to read"
    );
    Ok(source)
}
