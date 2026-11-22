use std::{
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
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
}

pub struct WorkspaceDocuments {
    root: PathBuf,
    current: Option<OpenDocument>,
    changed: Arc<AtomicBool>,
}

impl WorkspaceDocuments {
    pub fn new(
        root: &Path,
        mut current: Option<OpenDocument>,
        changed: Arc<AtomicBool>,
    ) -> Result<Self> {
        let canonical_root = root
            .canonicalize()
            .context("Could not open the resume workspace")?;

        // Normalize the workspace prefix without resolving a document symlink.
        if let Some(document) = &mut current {
            let relative = document
                .path
                .strip_prefix(root)
                .context("Resume is outside the workspace")?;
            ensure!(
                relative
                    .components()
                    .all(|part| matches!(part, Component::Normal(_))),
                "Resume is outside the workspace"
            );
            document.path = canonical_root.join(relative);
        }

        Ok(Self {
            root: canonical_root,
            current,
            changed,
        })
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

    fn copy_project(&self, project: &Project, name: &str) -> Result<Project> {
        let working_copy = self
            .current
            .as_ref()
            .filter(|_| self.is_current(project))
            .map(|current| (project.manifest.entrypoint.clone(), current.source.clone()));

        resume_project::duplicate(&self.root, project, name, working_copy.as_slice())
    }

    fn copy_current(&self, name: &str) -> Result<Project> {
        let current = self.current.as_ref().context("No resume is open")?;
        let Some(id) = current.project_id else {
            return resume_project::create_from_source(&self.root, name, &current.source);
        };

        let project = self.find_project(id)?;
        ensure!(
            self.is_current(&project),
            "The open resume moved. Reopen it before copying."
        );

        self.copy_project(&project, name)
    }
}

impl DocumentWorkspace for WorkspaceDocuments {
    fn has_current(&self) -> bool {
        self.current.is_some()
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

        let source = read_source(&project.document(), &self.root)?;
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
            Some(CURRENT_DOCUMENT_ID) => self.copy_current(name)?,
            Some(id) => {
                let id = Uuid::parse_str(id).context("Use a resume ID from list_resumes")?;
                let source = self.find_project(id)?;
                self.copy_project(&source, name)?
            }
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
