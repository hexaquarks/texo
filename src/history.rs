//! Per-file source history. A pending entry bridges the two atomic file replacements:
//! after interruption it becomes a saved revision only if the source reached disk.
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

pub const DIRECTORY: &str = ".texo-history";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum RevisionKind {
    Initial,
    Save,
    Restore { from: Uuid },
    Recovery,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Revision {
    pub id: Uuid,
    pub timestamp: i64,
    pub kind: RevisionKind,
    pub source: String,
}

impl Revision {
    fn new(source: &str, kind: RevisionKind) -> Self {
        Self {
            id: Uuid::new_v4(),
            timestamp: OffsetDateTime::now_utc().unix_timestamp(),
            kind,
            source: source.into(),
        }
    }

    pub fn label(&self) -> &str {
        match self.kind {
            RevisionKind::Initial => "Initial",
            RevisionKind::Save => "Saved",
            RevisionKind::Restore { .. } => "Restored",
            RevisionKind::Recovery => "Recovery",
        }
    }

    pub fn date_label(&self) -> String {
        match OffsetDateTime::from_unix_timestamp(self.timestamp) {
            Ok(date) => format!(
                "{} {:02}:{:02}:{:02}",
                date.date(),
                date.hour(),
                date.minute(),
                date.second()
            ),
            Err(_) => "Unknown date".into(),
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    revisions: Vec<Revision>,
    pending: Option<Revision>,
}

struct Store {
    path: PathBuf,
    journal: Journal,
    // Lock a separate, stable file: replacing the JSON must not replace the lock inode.
    lock: fs::File,
}

impl Drop for Store {
    fn drop(&mut self) {
        // Explicit unlock also releases descriptors briefly inherited by compiler subprocesses.
        let _ = self.lock.unlock();
    }
}

impl Store {
    fn open(document: &Path) -> Result<Self> {
        let directory = document
            .parent()
            .context("Document needs a parent directory")?
            .join(DIRECTORY);
        fs::create_dir_all(&directory).context("Could not create source history")?;
        ensure!(
            !fs::symlink_metadata(&directory)?.file_type().is_symlink(),
            "History directory must not be a symlink"
        );

        let filename = document.file_name().context("Document needs a filename")?;
        let mut journal_name = filename.to_os_string();
        journal_name.push(".json");
        let mut lock_name = filename.to_os_string();
        lock_name.push(".lock");
        let path = directory.join(journal_name);
        let lock_path = directory.join(lock_name);
        for entry in [&path, &lock_path] {
            if let Ok(metadata) = fs::symlink_metadata(entry) {
                ensure!(
                    metadata.is_file() && !metadata.file_type().is_symlink(),
                    "History contains a non-regular file"
                );
            }
        }

        let lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(lock_path)?;
        lock.try_lock()
            .context("Source history is busy in another Texo window; try again")?;

        let mut store = Self {
            path,
            journal: Journal::default(),
            lock,
        };
        store.journal = match fs::read(&store.path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .context("Source history is damaged; nothing was overwritten")?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Journal::default(),
            Err(error) => return Err(error).context("Could not read source history"),
        };

        if let Some(mut pending) = store.journal.pending.take() {
            if read_source(document)?.as_deref() != Some(&pending.source) {
                // A failed write or an external edit is not evidence of a saved revision.
                // Keep the staged text recoverable without claiming that it reached disk.
                pending.kind = RevisionKind::Recovery;
            }
            store.commit_revision(pending);
            store.persist()?;
        }
        Ok(store)
    }

    fn commit_revision(&mut self, revision: Revision) {
        let latest = self
            .journal
            .revisions
            .iter()
            .rev()
            .find(|entry| entry.kind != RevisionKind::Recovery);
        if revision.kind == RevisionKind::Save
            && latest.is_some_and(|entry| entry.source == revision.source)
        {
            return;
        }
        self.journal.revisions.push(revision);
    }

    fn persist(&self) -> Result<()> {
        let mut temporary = tempfile::NamedTempFile::new_in(self.path.parent().unwrap())?;
        serde_json::to_writer(&mut temporary, &self.journal)?;
        temporary.as_file().sync_all()?;
        temporary
            .persist(&self.path)
            .context("Could not persist source history")?;
        Ok(())
    }

    fn initial(&mut self, source: Option<&str>) -> Result<()> {
        let has_saved_revision = self
            .journal
            .revisions
            .iter()
            .any(|entry| entry.kind != RevisionKind::Recovery);
        if !has_saved_revision && let Some(source) = source {
            self.journal
                .revisions
                .push(Revision::new(source, RevisionKind::Initial));
            self.persist()?;
        }
        Ok(())
    }
}

pub fn revisions(document: &Path) -> Result<Vec<Revision>> {
    let mut store = Store::open(document)?;
    let source = read_source(document)?;
    store.initial(source.as_deref())?;
    Ok(std::mem::take(&mut store.journal.revisions))
}

fn read_source(document: &Path) -> Result<Option<String>> {
    match fs::read_to_string(document) {
        Ok(source) => Ok(Some(source)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).context("Could not read the source on disk"),
    }
}

/// Persist history before touching the source. A final metadata failure leaves a
/// recoverable pending entry and is reported as a warning after the successful save.
pub fn save(
    document: &Path,
    expected: Option<&str>,
    source: &str,
    kind: RevisionKind,
    recovery: Option<&str>,
) -> Result<Option<String>> {
    let mut store = Store::open(document)?;
    let previous = read_source(document)?;
    ensure!(
        previous.as_deref() == expected,
        "Source changed on disk; reload before saving or restoring"
    );
    store.initial(previous.as_deref())?;

    let latest = store
        .journal
        .revisions
        .iter()
        .rev()
        .find(|entry| entry.kind != RevisionKind::Recovery);
    if let Some(previous) = previous.as_deref()
        && latest.is_some_and(|entry| entry.source != previous)
    {
        let checkpoint = Revision::new(previous, RevisionKind::Recovery);
        store.journal.revisions.push(checkpoint);
        store.persist()?;
    }

    if let Some(recovery) = recovery {
        store
            .journal
            .revisions
            .push(Revision::new(recovery, RevisionKind::Recovery));
        store
            .persist()
            .context("Could not preserve unsaved work; restore cancelled")?;
    }

    let latest = store
        .journal
        .revisions
        .iter()
        .rev()
        .find(|revision| revision.kind != RevisionKind::Recovery);
    let changed = latest.is_none_or(|revision| revision.source != source);
    if !changed && kind == RevisionKind::Save && previous.as_deref() == Some(source) {
        return Ok(None);
    }

    store.journal.pending = Some(Revision::new(source, kind));
    store.persist()?;

    let write = publish_source(document, source, previous.as_deref());
    if let Err(error) = write {
        store.journal.pending = None;
        let _ = store.persist();
        return Err(error);
    }

    let revision = store.journal.pending.take().unwrap();
    store.commit_revision(revision);
    match store.persist() {
        Ok(()) => Ok(None),
        Err(error) => Ok(Some(format!(
            "Source saved; history finalization pending: {error}"
        ))),
    }
}

fn publish_source(document: &Path, source: &str, expected: Option<&str>) -> Result<()> {
    let mut temporary = tempfile::NamedTempFile::new_in(document.parent().unwrap())?;
    temporary.write_all(source.as_bytes())?;
    temporary.as_file().sync_all()?;
    ensure!(
        read_source(document)?.as_deref() == expected,
        "Source changed while saving; nothing was overwritten"
    );

    if expected.is_some() {
        temporary
            .persist(document)
            .context("Could not save the TeX document")?;
    } else {
        temporary
            .persist_noclobber(document)
            .context("Source appeared while saving; nothing was overwritten")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_and_changed_saves_survive_reopening_without_duplicates() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let document = directory.path().join("main.tex");
        fs::write(&document, "Initial")?;
        save(
            &document,
            Some("Initial"),
            "Edited",
            RevisionKind::Save,
            None,
        )?;
        save(
            &document,
            Some("Edited"),
            "Edited",
            RevisionKind::Save,
            None,
        )?;

        let entries = revisions(&document)?;
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].kind, RevisionKind::Initial);
        assert_eq!(entries[0].source, "Initial");
        assert_eq!(entries[1].source, "Edited");
        assert_ne!(entries[0].id, entries[1].id);
        assert_eq!(revisions(&document)?.len(), 2);
        assert_eq!(revisions(&document)?[1].id, entries[1].id);
        Ok(())
    }

    #[test]
    fn restore_preserves_unsaved_work_and_all_intervening_versions() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let document = directory.path().join("main.tex");
        fs::write(&document, "Initial")?;
        save(
            &document,
            Some("Initial"),
            "Second",
            RevisionKind::Save,
            None,
        )?;
        save(&document, Some("Second"), "Third", RevisionKind::Save, None)?;
        let initial = revisions(&document)?[0].clone();

        save(
            &document,
            Some("Third"),
            &initial.source,
            RevisionKind::Restore { from: initial.id },
            Some("Unsaved fourth"),
        )?;
        let entries = revisions(&document)?;
        assert_eq!(entries.len(), 5);
        assert_eq!(entries[2].source, "Third");
        assert_eq!(entries[3].kind, RevisionKind::Recovery);
        assert_eq!(entries[3].source, "Unsaved fourth");
        assert_eq!(entries[4].kind, RevisionKind::Restore { from: initial.id });
        assert_eq!(fs::read_to_string(&document)?, "Initial");
        save(
            &document,
            Some("Initial"),
            "Initial",
            RevisionKind::Save,
            None,
        )?;
        assert_eq!(revisions(&document)?.len(), 5);
        Ok(())
    }

    #[test]
    fn interrupted_save_is_finalized_only_when_source_was_published() -> Result<()> {
        for published in [false, true] {
            let directory = tempfile::tempdir()?;
            let document = directory.path().join("main.tex");
            fs::write(&document, "Before")?;
            let pending = Revision::new("After", RevisionKind::Save);
            {
                let mut store = Store::open(&document)?;
                store.initial(Some("Before"))?;
                store.journal.pending = Some(pending.clone());
                store.persist()?;
            }
            if published {
                fs::write(&document, "After")?;
            }
            let entries = revisions(&document)?;
            assert_eq!(entries.len(), 2);
            if published {
                assert_eq!(entries[1].id, pending.id);
                assert_eq!(entries[1].kind, RevisionKind::Save);
            } else {
                assert_eq!(entries[1].kind, RevisionKind::Recovery);
            }
            assert_eq!(revisions(&document)?.len(), entries.len());
        }
        Ok(())
    }

    #[test]
    fn conflicts_corrupt_history_and_busy_history_never_overwrite_source() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let document = directory.path().join("main.tex");
        fs::write(&document, "External edit")?;
        assert!(save(&document, Some("Stale"), "New", RevisionKind::Save, None).is_err());
        assert_eq!(fs::read_to_string(&document)?, "External edit");

        {
            let _store = Store::open(&document)?;
            assert!(
                save(
                    &document,
                    Some("External edit"),
                    "New",
                    RevisionKind::Save,
                    None
                )
                .is_err()
            );
        }
        fs::write(
            directory.path().join(DIRECTORY).join("main.tex.json"),
            "broken",
        )?;
        assert!(
            save(
                &document,
                Some("External edit"),
                "New",
                RevisionKind::Save,
                Some("Unsaved")
            )
            .is_err()
        );
        assert_eq!(fs::read_to_string(&document)?, "External edit");
        Ok(())
    }

    #[test]
    fn duplicate_and_rename_keep_independent_histories() -> Result<()> {
        let workspace = tempfile::tempdir()?;
        let mut project = crate::resume_project::create(workspace.path(), "First")?;
        let before = fs::read_to_string(project.document())?;
        save(
            &project.document(),
            Some(&before),
            "First edit",
            RevisionKind::Save,
            None,
        )?;
        let first_ids: Vec<_> = revisions(&project.document())?
            .iter()
            .map(|revision| revision.id)
            .collect();
        project.rename("Renamed")?;
        assert_eq!(revisions(&project.document())?[0].id, first_ids[0]);

        let copy = crate::resume_project::duplicate(
            workspace.path(),
            &project,
            "Copy",
            &[(project.manifest.entrypoint.clone(), "Independent".into())],
        )?;
        assert!(!copy.root.join(DIRECTORY).exists());
        let entries = revisions(&copy.document())?;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].source, "Independent");
        assert!(!first_ids.contains(&entries[0].id));
        Ok(())
    }
}
