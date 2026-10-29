//! Persist pane dimensions separately from user-editable settings.

use std::{
    io::Read,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::App;

const MAX_LAYOUT_STATE_BYTES: u64 = 8 * 1024;

#[derive(Clone, Copy, Default, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub(super) struct PaneSizes {
    pub(super) sidebar: Option<u16>,
    pub(super) preview: Option<u16>,
    pub(super) editor: Option<u16>,
}

impl PaneSizes {
    pub(super) fn path() -> Result<PathBuf> {
        let project = directories::ProjectDirs::from("", "", "texo")
            .context("Could not locate the layout state folder")?;
        let directory = project
            .state_dir()
            .unwrap_or_else(|| project.data_local_dir());

        Ok(layout_path(
            directory,
            std::env::var_os("XDG_STATE_HOME").as_deref(),
        ))
    }

    pub(super) fn load(path: &Path) -> Self {
        // Layout is disposable state. A missing or damaged file must not prevent startup.
        let Ok(file) = std::fs::File::open(path) else {
            return Self::default();
        };
        let mut bytes = Vec::new();
        let read_result = file
            .take(MAX_LAYOUT_STATE_BYTES + 1)
            .read_to_end(&mut bytes);
        if read_result.is_err() || bytes.len() as u64 > MAX_LAYOUT_STATE_BYTES {
            return Self::default();
        }

        serde_json::from_slice(&bytes).unwrap_or_default()
    }

    pub(super) fn save(&self, path: &Path) -> Result<()> {
        let parent = path
            .parent()
            .context("Layout state needs a parent folder")?;
        std::fs::create_dir_all(parent).context("Could not create the layout state folder")?;

        // Replace the file atomically so interrupted writes leave the previous layout intact.
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        serde_json::to_writer_pretty(file.as_file_mut(), self)?;
        file.persist(path)
            .context("Could not save the pane layout")?;
        Ok(())
    }
}

fn layout_path(default_directory: &Path, state_home: Option<&std::ffi::OsStr>) -> PathBuf {
    // Honor an explicit state directory on macOS too, keeping isolated runs off user state.
    match state_home.map(Path::new).filter(|path| path.is_absolute()) {
        Some(directory) => directory.join("texo/layout.json"),
        None => default_directory.join("layout.json"),
    }
}

impl App {
    pub(super) fn restore_layout(&mut self, path: PathBuf) {
        self.pane_sizes = PaneSizes::load(&path);
        self.layout_state_path = Some(path);
    }

    pub(super) fn save_layout(&mut self) {
        let Some(path) = &self.layout_state_path else {
            return;
        };

        if let Err(error) = self.pane_sizes.save(path) {
            self.report_error(error);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_state_directory_overrides_the_platform_default() {
        let default = Path::new("/platform/texo");
        assert_eq!(
            layout_path(default, Some(std::ffi::OsStr::new("/isolated/state"))),
            Path::new("/isolated/state/texo/layout.json")
        );
        for state_home in [None, Some(std::ffi::OsStr::new("relative"))] {
            assert_eq!(
                layout_path(default, state_home),
                default.join("layout.json")
            );
        }
    }

    #[test]
    fn saved_sizes_round_trip_and_replace_previous_state() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("state/layout.json");
        let mut sizes = PaneSizes {
            sidebar: Some(32),
            preview: Some(60),
            editor: Some(24),
        };
        sizes.save(&path)?;
        assert!(PaneSizes::load(&path) == sizes);

        sizes.preview = Some(70);
        sizes.save(&path)?;
        assert!(PaneSizes::load(&path) == sizes);
        assert_eq!(std::fs::read_dir(path.parent().unwrap())?.count(), 1);
        Ok(())
    }

    #[test]
    fn missing_invalid_and_oversized_state_uses_defaults() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("layout.json");
        assert!(PaneSizes::load(&path) == PaneSizes::default());

        for contents in ["{", r#"{"editor": -1}"#, r#"{"preview": 65536}"#] {
            std::fs::write(&path, contents)?;
            assert!(PaneSizes::load(&path) == PaneSizes::default());
        }

        std::fs::write(&path, vec![b' '; MAX_LAYOUT_STATE_BYTES as usize + 1])?;
        assert!(PaneSizes::load(&path) == PaneSizes::default());
        Ok(())
    }

    #[test]
    fn unwritable_state_reports_a_save_error() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let parent = directory.path().join("file");
        std::fs::write(&parent, "Cannot use a file as a directory")?;
        assert!(
            PaneSizes::default()
                .save(&parent.join("layout.json"))
                .is_err()
        );
        Ok(())
    }
}
