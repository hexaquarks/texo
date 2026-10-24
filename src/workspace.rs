use std::{
    env, fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use ignore::WalkBuilder;

const MAX_DISCOVERY_DEPTH: usize = 8;
const MAX_DISCOVERY_ENTRIES: usize = 10_000;
const DEFAULT_WORKSPACE: &str = "resumes";
const DEFAULT_DOCUMENT: &str = "main.tex";

pub struct Workspace {
    pub root: PathBuf,
    pub document: PathBuf,
}

impl Workspace {
    pub fn resolve(input: Option<&Path>) -> Result<Self> {
        Self::resolve_with_settings(input, &crate::config::Settings::path()?)
    }

    fn resolve_with_settings(input: Option<&Path>, settings_path: &Path) -> Result<Self> {
        if let Some(input) = input {
            return Self::resolve_folder(Some(input));
        }

        let settings = crate::config::Settings::load(settings_path)?;
        if let Some(root) = settings.resume_folder {
            let root = crate::config::resume_folder(&root.to_string_lossy())
                .context("Resume folder unavailable. Run texo with another folder and use ? to update settings")?;
            return Self::resolve_folder(Some(&root));
        }
        Self::resolve_folder(None)
    }

    fn resolve_folder(input: Option<&Path>) -> Result<Self> {
        let path = match input {
            Some(path) => env::current_dir()?.join(path),
            None => {
                let home = env::var_os("HOME").context("Could not locate your home directory")?;
                let root = PathBuf::from(home).join(DEFAULT_WORKSPACE);
                fs::create_dir_all(&root).context("Could not create the resumes workspace")?;
                root
            }
        };

        if path.is_dir() {
            let root = path
                .canonicalize()
                .context("Could not open the workspace")?;
            let document = project_document(&root)?.unwrap_or_else(|| root.join(DEFAULT_DOCUMENT));
            return Ok(Self { root, document });
        }

        if path.extension().is_none_or(|extension| extension != "tex") {
            bail!("Open a workspace directory or a .tex document");
        }

        let parent = path
            .parent()
            .context("The document has no parent directory")?;
        let root = parent
            .canonicalize()
            .context("The document directory does not exist")?;
        let filename = path.file_name().context("The document has no filename")?;
        let document = root.join(filename);
        Ok(Self { root, document })
    }
}

pub fn display_path(path: &Path) -> String {
    abbreviate_home(path, env::var_os("HOME").as_deref().map(Path::new))
}

fn abbreviate_home(path: &Path, home: Option<&Path>) -> String {
    if let Some(home) = home
        && let Ok(relative) = path.strip_prefix(home)
    {
        if relative.as_os_str().is_empty() {
            return "~".to_owned();
        }
        return format!("~/{}", relative.display());
    }
    path.display().to_string()
}

/// Prefer a project's entry document over alphabetically earlier section files.
pub fn project_document(root: &Path) -> Result<Option<PathBuf>> {
    for filename in [DEFAULT_DOCUMENT, "resume.tex"] {
        let path = root.join(filename);
        if path.is_file() {
            return Ok(Some(path));
        }
    }
    first_document(root)
}

// Bound startup work for large workspaces and never traverse directory symlinks.
// Sorting the walk makes the initial document stable between launches.
fn first_document(root: &Path) -> Result<Option<PathBuf>> {
    for (index, entry) in WalkBuilder::new(root)
        .hidden(false)
        .follow_links(false)
        .max_depth(Some(MAX_DISCOVERY_DEPTH))
        .sort_by_file_path(|left, right| left.cmp(right))
        .build()
        .enumerate()
    {
        if index >= MAX_DISCOVERY_ENTRIES {
            bail!("Workspace is too large to choose a document; open a specific .tex file");
        }
        let entry = entry.context("Could not inspect the workspace")?;
        if entry.file_type().is_some_and(|kind| kind.is_file())
            && entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "tex")
        {
            return Ok(Some(entry.into_path()));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_folder_is_used_and_explicit_path_overrides_it() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let folder = directory.path().join("resumes");
        fs::create_dir(&folder)?;
        fs::write(folder.join("main.tex"), "saved resume")?;
        let settings_path = directory.path().join("settings.json");
        crate::config::Settings {
            resume_folder: Some(folder.clone()),
        }
        .save(&settings_path)?;

        let workspace = Workspace::resolve_with_settings(None, &settings_path)?;
        assert_eq!(workspace.root, folder.canonicalize()?);
        let explicit = directory.path().join("other.tex");
        fs::write(&settings_path, "invalid settings")?;
        assert_eq!(
            Workspace::resolve_with_settings(Some(&explicit), &settings_path)?.document,
            directory.path().canonicalize()?.join("other.tex")
        );
        assert!(Workspace::resolve_with_settings(None, &settings_path).is_err());
        Ok(())
    }

    #[test]
    fn opens_nested_documents_deterministically_and_respects_ignore_rules() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        fs::create_dir(root.join(".git"))?;
        fs::write(root.join(".gitignore"), "A-generated/\n")?;
        fs::create_dir(root.join("A-generated"))?;
        fs::write(root.join("A-generated/ignored.tex"), "ignored")?;
        fs::create_dir(root.join("AI engineering"))?;
        fs::write(root.join("AI engineering/main.tex"), "first")?;
        fs::write(root.join("second.tex"), "second")?;

        let workspace = Workspace::resolve(Some(root))?;
        assert_eq!(
            workspace.document,
            root.canonicalize()?.join("AI engineering/main.tex")
        );
        Ok(())
    }

    #[test]
    fn project_discovery_prefers_entry_documents_and_handles_empty_folders() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        assert!(project_document(root)?.is_none());
        fs::write(root.join("a-section.tex"), "section")?;
        fs::write(root.join("resume.tex"), "resume")?;
        assert_eq!(project_document(root)?, Some(root.join("resume.tex")));
        fs::write(root.join("main.tex"), "main")?;
        assert_eq!(project_document(root)?, Some(root.join("main.tex")));
        fs::remove_file(root.join("main.tex"))?;
        fs::remove_file(root.join("resume.tex"))?;
        assert_eq!(project_document(root)?, Some(root.join("a-section.tex")));
        Ok(())
    }

    #[test]
    fn empty_workspace_defers_document_creation_until_save() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let workspace = Workspace::resolve(Some(directory.path()))?;
        assert_eq!(
            workspace.document,
            directory.path().canonicalize()?.join(DEFAULT_DOCUMENT)
        );
        assert!(!workspace.document.exists());
        Ok(())
    }

    #[test]
    fn explicit_document_uses_its_parent_as_workspace() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("new.tex");
        let workspace = Workspace::resolve(Some(&path))?;
        assert_eq!(workspace.root, directory.path().canonicalize()?);
        assert!(!path.exists());
        assert!(Workspace::resolve(Some(&directory.path().join("missing"))).is_err());
        Ok(())
    }

    #[test]
    fn home_abbreviation_matches_whole_path_components() {
        let home = Some(Path::new("/Users/alex"));
        assert_eq!(
            abbreviate_home(Path::new("/Users/alex/resumes"), home),
            "~/resumes"
        );
        assert_eq!(
            abbreviate_home(Path::new("/Users/alexander/resumes"), home),
            "/Users/alexander/resumes"
        );
    }
}
