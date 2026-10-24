use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

const MAX_SETTINGS_BYTES: u64 = 16 * 1024;

#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub resume_folder: Option<PathBuf>,
}

impl Settings {
    pub fn path() -> Result<PathBuf> {
        let directories = directories::ProjectDirs::from("", "", "texo")
            .context("Could not locate Texo's configuration folder")?;
        Ok(directories.config_dir().join("settings.json"))
    }

    pub fn load(path: &Path) -> Result<Self> {
        let file = match std::fs::File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(error).context("Could not read settings"),
        };
        let mut bytes = Vec::new();
        file.take(MAX_SETTINGS_BYTES + 1).read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() as u64 <= MAX_SETTINGS_BYTES,
            "Settings file is too large"
        );

        serde_json::from_slice(&bytes).context("Invalid settings file")
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let parent = path.parent().context("Settings need a parent folder")?;
        std::fs::create_dir_all(parent).context("Could not create the settings folder")?;
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        file.write_all(&serde_json::to_vec(self)?)?;
        file.persist(path).context("Could not save settings")?;
        Ok(())
    }
}

pub fn resume_folder(input: &str) -> Result<PathBuf> {
    let input = input.trim();
    ensure!(!input.is_empty(), "Enter a resume folder");
    ensure!(!input.contains(['\r', '\n']), "Enter a single folder path");
    let path = if input == "~" || input.starts_with("~/") {
        let home = directories::BaseDirs::new().context("Could not locate your home folder")?;
        home.home_dir().join(input.strip_prefix("~/").unwrap_or(""))
    } else {
        std::env::current_dir()?.join(input)
    };
    let path = path.canonicalize().context("Choose an existing folder")?;
    ensure!(path.is_dir(), "Choose a folder, not a file");
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip_without_authentication_fields() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let file = directory.path().join("config/settings.json");
        assert!(Settings::load(&file)?.resume_folder.is_none());
        let root = directory.path().canonicalize()?;
        Settings {
            resume_folder: Some(root.clone()),
        }
        .save(&file)?;
        assert_eq!(Settings::load(&file)?.resume_folder, Some(root));
        let contents = std::fs::read_to_string(file)?;
        assert!(!contents.contains("key"));
        Ok(())
    }

    #[test]
    fn invalid_oversized_and_unwritable_settings_fail() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let file = directory.path().join("settings.json");
        for contents in [
            "invalid".to_owned(),
            "x".repeat(MAX_SETTINGS_BYTES as usize + 1),
        ] {
            std::fs::write(&file, contents)?;
            assert!(Settings::load(&file).is_err());
        }
        assert!(
            Settings::default()
                .save(&file.join("settings.json"))
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn folder_paths_accept_spaces_and_reject_missing_paths_and_files() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let folder = directory.path().join("My resumes");
        std::fs::create_dir(&folder)?;
        assert_eq!(
            resume_folder(folder.to_str().unwrap())?,
            folder.canonicalize()?
        );
        let file = directory.path().join("main.tex");
        std::fs::write(&file, "source")?;
        assert!(resume_folder(file.to_str().unwrap()).is_err());
        assert!(resume_folder(directory.path().join("missing").to_str().unwrap()).is_err());
        assert!(resume_folder("  ").is_err());
        assert_eq!(
            resume_folder("~")?,
            directories::BaseDirs::new()
                .unwrap()
                .home_dir()
                .canonicalize()?
        );
        Ok(())
    }
}
