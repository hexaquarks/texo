use std::{
    collections::HashSet,
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use ignore::WalkBuilder;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::resume_label::ResumeLabel;
use texo::sandbox::MAX_PROJECT_MANIFEST_BYTES;
pub use texo::sandbox::PROJECT_MANIFEST_FILE as MANIFEST_FILE;

const MAX_NAME_CHARS: usize = 80;
const MAX_IMPORT_BYTES: u64 = 256 * 1024 * 1024;
const MAX_IMPORT_FILES: usize = 10_000;
const MAX_DIRECTORY_ATTEMPTS: usize = 10_000;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub id: Uuid,
    pub name: String,
    pub entrypoint: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<ResumeLabel>,
}

#[derive(Clone)]
pub struct Project {
    pub root: PathBuf,
    pub manifest: Manifest,
}

#[derive(Default)]
pub struct Catalog {
    pub projects: Vec<Project>,
    pub warnings: Vec<String>,
    pub loose_files: Vec<PathBuf>,
}

impl Project {
    pub fn document(&self) -> PathBuf {
        self.root.join(&self.manifest.entrypoint)
    }

    pub fn load(root: &Path) -> Result<Self> {
        let metadata = fs::symlink_metadata(root.join(MANIFEST_FILE))?;
        ensure!(
            metadata.is_file() && !metadata.file_type().is_symlink(),
            "Manifest must be a regular file"
        );
        ensure!(
            metadata.len() <= MAX_PROJECT_MANIFEST_BYTES,
            "Manifest is too large"
        );

        let mut text = String::new();
        fs::File::open(root.join(MANIFEST_FILE))?
            .take(MAX_PROJECT_MANIFEST_BYTES + 1)
            .read_to_string(&mut text)?;
        ensure!(
            text.len() as u64 <= MAX_PROJECT_MANIFEST_BYTES,
            "Manifest is too large"
        );
        let mut manifest: Manifest = toml_edit::de::from_str(&text)
            .map_err(|_| anyhow::anyhow!("Invalid texo.toml manifest"))?;
        validate_name(&manifest.name)?;
        if let Some(label) = &manifest.label {
            manifest.label = Some(ResumeLabel::new(&label.name, label.color)?);
        }
        validate_entrypoint(root, &manifest.entrypoint)?;

        Ok(Self {
            root: root.to_path_buf(),
            manifest,
        })
    }

    pub fn set_label(&mut self, label: Option<ResumeLabel>) -> Result<()> {
        let label = label
            .map(|label| ResumeLabel::new(&label.name, label.color))
            .transpose()?;
        let mut manifest = self.manifest.clone();
        manifest.label = label;

        write_manifest(&self.root, &manifest)?;
        self.manifest = manifest;
        Ok(())
    }

    pub fn rename(&mut self, name: &str) -> Result<()> {
        let name = validate_name(name)?;

        // A dialog may hold an older project; preserve metadata changed since it opened.
        let mut manifest = Self::load(&self.root)?.manifest;
        manifest.name = name;

        write_manifest(&self.root, &manifest)?;
        self.manifest = manifest;
        Ok(())
    }
}

pub fn discover(root: &Path) -> Result<Catalog> {
    let mut catalog = Catalog::default();

    if !root.exists() {
        return Ok(catalog);
    }

    let mut paths = fs::read_dir(root)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    paths.sort();
    let mut ids = HashSet::new();

    for path in paths {
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            continue;
        }

        if metadata.is_dir() && path.join(MANIFEST_FILE).exists() {
            match Project::load(&path) {
                Ok(project) if ids.insert(project.manifest.id) => catalog.projects.push(project),
                Ok(_) => catalog
                    .warnings
                    .push(format!("Duplicate project ID in {}", path.display())),
                Err(error) => catalog
                    .warnings
                    .push(format!("{}: {error}", path.display())),
            }
        } else if metadata.is_file() && path.extension().is_some_and(|extension| extension == "tex")
        {
            catalog.loose_files.push(path);
        }
    }

    catalog.projects.sort_by(|left, right| {
        left.manifest
            .name
            .to_lowercase()
            .cmp(&right.manifest.name.to_lowercase())
            .then(left.manifest.id.cmp(&right.manifest.id))
    });
    Ok(catalog)
}

pub fn validate_entrypoint(root: &Path, entrypoint: &Path) -> Result<()> {
    ensure!(
        !entrypoint.to_string_lossy().chars().any(char::is_control),
        "Entrypoint cannot contain control characters"
    );
    ensure!(
        entrypoint
            .extension()
            .is_some_and(|extension| extension == "tex"),
        "Entrypoint must be a .tex file"
    );
    ensure!(
        entrypoint
            .components()
            .all(|part| matches!(part, Component::Normal(_))),
        "Entrypoint must stay inside the project"
    );

    let mut current = root.to_path_buf();
    for component in entrypoint.components() {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) => ensure!(
                !metadata.file_type().is_symlink(),
                "Entrypoint cannot contain symlinks"
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }

    ensure!(!current.is_dir(), "Entrypoint must be a file");
    Ok(())
}

fn validate_name(name: &str) -> Result<String> {
    let name = name.trim();
    ensure!(
        !name.is_empty() && name.chars().count() <= MAX_NAME_CHARS,
        "Use a name between 1 and {MAX_NAME_CHARS} characters"
    );
    ensure!(
        !name.chars().any(char::is_control),
        "Names cannot contain control characters"
    );
    Ok(name.to_owned())
}

fn write_manifest(root: &Path, manifest: &Manifest) -> Result<()> {
    use std::io::Write;

    let mut temporary = tempfile::NamedTempFile::new_in(root)?;
    temporary.write_all(toml_edit::ser::to_string_pretty(manifest)?.as_bytes())?;
    temporary.persist(root.join(MANIFEST_FILE))?;
    Ok(())
}

fn reserve_directory(workspace: &Path, name: &str) -> Result<PathBuf> {
    fs::create_dir_all(workspace).context("Could not create the resume workspace")?;
    let slug: String = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let slug = slug.trim_matches('-');
    let slug = if slug.is_empty() { "resume" } else { slug };

    for suffix in 0..MAX_DIRECTORY_ATTEMPTS {
        let filename = if suffix == 0 {
            slug.to_owned()
        } else {
            format!("{slug}-{suffix}")
        };
        let path = workspace.join(filename);
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error).context("Could not create the resume directory"),
        }
    }
    anyhow::bail!("Choose another name; too many matching directories")
}

pub fn create(workspace: &Path, name: &str) -> Result<Project> {
    create_project(workspace, name, Path::new("main.tex"), None, |root| {
        fs::write(root.join("main.tex"), crate::editor::STARTER_DOCUMENT)?;
        Ok(())
    })
}

fn create_project(
    workspace: &Path,
    name: &str,
    entrypoint: &Path,
    label: Option<ResumeLabel>,
    populate: impl FnOnce(&Path) -> Result<()>,
) -> Result<Project> {
    let name = validate_name(name)?;
    let root = reserve_directory(workspace, &name)?;
    let manifest = Manifest {
        id: Uuid::new_v4(),
        name,
        entrypoint: entrypoint.to_path_buf(),
        label,
    };

    let result = (|| {
        populate(&root)?;
        validate_entrypoint(&root, entrypoint)?;

        // Publish the manifest last: an incomplete copy is never recognized as a project.
        write_manifest(&root, &manifest)?;
        Ok(Project {
            root: root.clone(),
            manifest,
        })
    })();

    if result.is_err() {
        let _ = fs::remove_dir_all(&root);
    }
    result
}

pub struct ImportPlan {
    pub source: PathBuf,
    pub candidates: Vec<PathBuf>,
    label: Option<ResumeLabel>,
}

impl ImportPlan {
    pub fn inspect(path: &Path) -> Result<Self> {
        let path = path
            .canonicalize()
            .context("Choose an existing TeX file or project folder")?;

        if path.is_file() {
            ensure!(
                path.extension().is_some_and(|extension| extension == "tex"),
                "Choose a .tex file or project folder"
            );
            return Ok(Self {
                source: path
                    .parent()
                    .context("Source needs a parent folder")?
                    .to_path_buf(),
                candidates: vec![PathBuf::from(path.file_name().unwrap())],
                label: None,
            });
        }

        if path.join(MANIFEST_FILE).exists() {
            let project = Project::load(&path)?;
            return Ok(Self {
                source: path,
                candidates: vec![project.manifest.entrypoint],
                label: project.manifest.label,
            });
        }

        let mut candidates = Vec::new();
        let mut fallback = Vec::new();
        for entry in source_files(&path)? {
            if entry
                .extension()
                .is_some_and(|extension| extension == "tex")
            {
                let mut source = String::new();
                fs::File::open(path.join(&entry))?
                    .take(texo::resume_tools::MAX_DRAFT_BYTES as u64)
                    .read_to_string(&mut source)?;
                if source.contains("\\documentclass") || source.contains("\\begin{document}") {
                    candidates.push(entry.clone());
                }
                fallback.push(entry);
            }
        }

        if candidates.is_empty() {
            candidates = fallback;
        }
        ensure!(
            !candidates.is_empty(),
            "No TeX entrypoint found in this folder"
        );
        candidates.sort();
        Ok(Self {
            source: path,
            candidates,
            label: None,
        })
    }

    pub fn import(&self, workspace: &Path, name: &str, entrypoint: &Path) -> Result<Project> {
        self.copy_project(workspace, name, entrypoint, &[])
    }

    fn copy_project(
        &self,
        workspace: &Path,
        name: &str,
        entrypoint: &Path,
        working_copies: &[(PathBuf, String)],
    ) -> Result<Project> {
        ensure!(
            self.candidates
                .iter()
                .any(|candidate| candidate == entrypoint),
            "Choose one of the discovered entrypoints"
        );
        validate_entrypoint(&self.source, entrypoint)?;
        ensure!(
            self.source.join(entrypoint).is_file(),
            "The selected source is missing"
        );

        fs::create_dir_all(workspace)?;
        let workspace = workspace.canonicalize()?;
        ensure!(
            workspace == self.source || !workspace.starts_with(&self.source),
            "Choose a project folder, not a parent of the workspace"
        );

        let files = source_files(&self.source)?;
        create_project(
            &workspace,
            name,
            entrypoint,
            self.label.clone(),
            |destination| {
                for relative in &files {
                    let target = destination.join(relative);
                    fs::create_dir_all(target.parent().unwrap())?;
                    fs::copy(self.source.join(relative), target)?;
                }

                for (relative, source) in working_copies {
                    ensure!(
                        relative
                            .components()
                            .all(|part| matches!(part, Component::Normal(_)))
                            && relative
                                .extension()
                                .is_some_and(|extension| extension == "tex"),
                        "Working copies must be TeX files inside the resume"
                    );
                    let target = destination.join(relative);
                    fs::create_dir_all(target.parent().unwrap())?;
                    fs::write(target, source)?;
                }
                Ok(())
            },
        )
    }
}

pub(crate) fn source_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let mut total_bytes = 0;
    for entry in WalkBuilder::new(root)
        .standard_filters(false)
        .follow_links(false)
        .filter_entry(include_import_entry)
        .build()
        .skip(1)
    {
        let entry = entry.context("Could not read import source")?;
        let metadata = fs::symlink_metadata(entry.path())?;
        ensure!(
            !metadata.file_type().is_symlink(),
            "Import contains a symlink; copy its target into the project first"
        );
        if metadata.is_dir() {
            continue;
        }
        ensure!(metadata.is_file(), "Import contains a non-regular file");
        total_bytes += metadata.len();
        ensure!(
            total_bytes <= MAX_IMPORT_BYTES && files.len() < MAX_IMPORT_FILES,
            "Import is too large; choose a dedicated resume project folder"
        );
        files.push(entry.path().strip_prefix(root)?.to_path_buf());
    }
    files.sort();
    Ok(files)
}

fn include_import_entry(entry: &ignore::DirEntry) -> bool {
    if entry.depth() == 0 {
        return true;
    }

    let name = entry.file_name().to_string_lossy();
    let excluded = matches!(
        name.as_ref(),
        ".git" | "node_modules" | "target" | MANIFEST_FILE
    ) || name.starts_with(".env")
        || name.starts_with(".texo-");
    let another_project = entry.path().is_dir() && entry.path().join(MANIFEST_FILE).exists();

    !excluded && !another_project
}

pub fn duplicate(
    workspace: &Path,
    project: &Project,
    name: &str,
    working_copies: &[(PathBuf, String)],
) -> Result<Project> {
    let plan = ImportPlan {
        source: project.root.clone(),
        candidates: vec![project.manifest.entrypoint.clone()],
        label: project.manifest.label.clone(),
    };
    plan.copy_project(
        workspace,
        name,
        &project.manifest.entrypoint,
        working_copies,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_survive_reload_rename_duplicate_and_managed_import() -> Result<()> {
        use crate::resume_label::LabelColor;

        let workspace = tempfile::tempdir()?;
        let other_workspace = tempfile::tempdir()?;
        let mut project = create(workspace.path(), "Original")?;
        assert!(project.manifest.label.is_none());
        let source = fs::read_to_string(project.document())?;
        let label = ResumeLabel::new("Systems engineer", LabelColor::Teal)?;
        project.set_label(Some(label.clone()))?;
        project.rename("Renamed")?;
        let project = Project::load(&project.root)?;
        assert_eq!(project.manifest.label, Some(label.clone()));
        assert_eq!(fs::read_to_string(project.document())?, source);

        let copy = duplicate(
            workspace.path(),
            &project,
            "Copy",
            &[(project.manifest.entrypoint.clone(), "unsaved source".into())],
        )?;
        assert_eq!(copy.manifest.label, Some(label.clone()));
        assert_ne!(copy.manifest.id, project.manifest.id);
        assert_eq!(fs::read_to_string(copy.document())?, "unsaved source");

        let plan = ImportPlan::inspect(&project.root)?;
        let imported = plan.import(
            other_workspace.path(),
            "Imported",
            &project.manifest.entrypoint,
        )?;
        assert_eq!(imported.manifest.label, Some(label));
        assert_ne!(imported.manifest.id, project.manifest.id);

        let mut project = project;
        project.set_label(None)?;
        assert!(Project::load(&project.root)?.manifest.label.is_none());
        assert!(!fs::read_to_string(project.root.join(MANIFEST_FILE))?.contains("[label]"));
        Ok(())
    }

    #[test]
    fn invalid_label_changes_preserve_metadata_and_invalid_disk_labels_are_reported() -> Result<()>
    {
        use crate::resume_label::LabelColor;

        let workspace = tempfile::tempdir()?;
        let mut project = create(workspace.path(), "Original")?;
        let path = project.root.join(MANIFEST_FILE);
        let before = fs::read_to_string(&path)?;
        let invalid = ResumeLabel {
            name: "one two three four".into(),
            color: LabelColor::Blue,
        };
        assert!(project.set_label(Some(invalid)).is_err());
        assert_eq!(fs::read_to_string(&path)?, before);
        assert!(project.manifest.label.is_none());

        fs::write(
            &path,
            format!("{before}\n[label]\nname = 'one two three four'\ncolor = 'blue'\n"),
        )?;
        let catalog = discover(workspace.path())?;
        assert!(catalog.projects.is_empty());
        assert_eq!(catalog.warnings.len(), 1);
        Ok(())
    }

    #[test]
    fn projects_keep_identity_across_rename_and_use_unique_safe_directories() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let mut first = create(directory.path(), "C++ / Systems")?;
        let second = create(directory.path(), "C++ / Systems")?;

        assert_ne!(first.root, second.root);
        assert_ne!(first.manifest.id, second.manifest.id);
        assert_eq!(first.root.parent(), Some(directory.path()));

        let id = first.manifest.id;
        let path = first.document();
        first.rename("AI engineer")?;

        assert_eq!(Project::load(&first.root)?.manifest.id, id);
        assert_eq!(first.document(), path);
        assert_eq!(
            discover(directory.path())?.projects[0].manifest.name,
            "AI engineer"
        );
        Ok(())
    }

    #[test]
    fn import_preserves_supporting_paths_and_duplicate_is_independent() -> Result<()> {
        let workspace = tempfile::tempdir()?;
        let original = tempfile::tempdir()?;
        fs::create_dir(original.path().join("assets"))?;
        fs::write(original.path().join("assets/photo.dat"), "image")?;
        fs::write(original.path().join("main.tex"), "\\documentclass{article}")?;
        fs::write(
            original.path().join("other.tex"),
            "\\documentclass{article}",
        )?;

        let plan = ImportPlan::inspect(original.path())?;
        assert_eq!(plan.candidates.len(), 2);

        let imported = plan.import(workspace.path(), "General", Path::new("main.tex"))?;
        assert_eq!(
            fs::read_to_string(imported.root.join("assets/photo.dat"))?,
            "image"
        );

        fs::write(imported.document(), "changed copy")?;
        assert_eq!(
            fs::read_to_string(original.path().join("main.tex"))?,
            "\\documentclass{article}"
        );

        let duplicate = duplicate(
            workspace.path(),
            &imported,
            "General",
            &[(
                imported.manifest.entrypoint.clone(),
                "unsaved duplicate".into(),
            )],
        )?;
        assert_ne!(duplicate.manifest.id, imported.manifest.id);
        assert_eq!(
            fs::read_to_string(duplicate.document())?,
            "unsaved duplicate"
        );
        assert_eq!(fs::read_to_string(imported.document())?, "changed copy");
        Ok(())
    }

    #[test]
    fn discovery_skips_malformed_metadata_and_loose_files_without_modifying_them() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let broken = directory.path().join("broken");
        fs::create_dir(&broken)?;
        fs::write(broken.join(MANIFEST_FILE), "invalid")?;
        fs::write(directory.path().join("legacy.tex"), "legacy source")?;

        let catalog = discover(directory.path())?;
        assert!(catalog.projects.is_empty());
        assert_eq!(catalog.warnings.len(), 1);
        assert_eq!(catalog.loose_files.len(), 1);
        assert_eq!(fs::read_to_string(broken.join(MANIFEST_FILE))?, "invalid");
        assert!(validate_entrypoint(directory.path(), Path::new("../escape.tex")).is_err());
        assert!(validate_entrypoint(directory.path(), Path::new("/escape.tex")).is_err());
        assert!(ImportPlan::inspect(&directory.path().join("missing")).is_err());
        Ok(())
    }

    #[test]
    fn missing_sources_and_failed_copies_do_not_publish_projects() -> Result<()> {
        let workspace = tempfile::tempdir()?;
        let source = tempfile::tempdir()?;
        let input = source.path().join("main.tex");
        fs::write(&input, "source")?;
        let plan = ImportPlan::inspect(&input)?;
        fs::remove_file(&input)?;

        assert!(
            plan.import(workspace.path(), "Missing", Path::new("main.tex"))
                .is_err()
        );
        assert!(discover(workspace.path())?.projects.is_empty());

        let failed = create_project(
            workspace.path(),
            "Incomplete",
            Path::new("main.tex"),
            None,
            |root| {
                fs::write(root.join("asset.txt"), "partial copy")?;
                anyhow::bail!("simulated copy failure")
            },
        );
        assert!(failed.is_err());
        assert_eq!(fs::read_dir(workspace.path())?.count(), 0);
        Ok(())
    }

    #[test]
    fn imports_create_missing_workspaces_and_duplicate_ids_are_reported() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("source");
        fs::create_dir(&source)?;
        fs::write(source.join("main.tex"), "source")?;
        let workspace = directory.path().join("new-workspace");
        let plan = ImportPlan::inspect(&source)?;
        let project = plan.import(&workspace, "General", Path::new("main.tex"))?;

        let invalid_copy = workspace.join("copied-directory");
        fs::create_dir(&invalid_copy)?;
        fs::copy(
            project.root.join(MANIFEST_FILE),
            invalid_copy.join(MANIFEST_FILE),
        )?;

        let catalog = discover(&workspace)?;
        assert_eq!(catalog.projects.len(), 1);
        assert_eq!(catalog.warnings.len(), 1);
        assert!(project.document().exists());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn import_and_manifest_reject_symlinks() -> Result<()> {
        let workspace = tempfile::tempdir()?;
        let source = tempfile::tempdir()?;
        fs::write(source.path().join("main.tex"), "resume")?;
        std::os::unix::fs::symlink("/etc/passwd", source.path().join("outside.tex"))?;
        let plan = ImportPlan::inspect(&source.path().join("main.tex"))?;
        assert!(
            plan.import(workspace.path(), "General", Path::new("main.tex"))
                .is_err()
        );
        assert!(discover(workspace.path())?.projects.is_empty());
        assert!(validate_entrypoint(source.path(), Path::new("outside.tex")).is_err());
        Ok(())
    }
}
