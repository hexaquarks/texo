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
    #[serde(default)]
    pub keybindings: Keybindings,
    #[serde(default)]
    pub editor: EditorPreferences,
}

#[derive(Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct EditorPreferences {
    pub soft_wrap: bool,
    pub mode: EditingMode,
}

impl Default for EditorPreferences {
    fn default() -> Self {
        Self {
            soft_wrap: true,
            mode: EditingMode::Standard,
        }
    }
}

#[derive(Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum EditingMode {
    #[default]
    Standard,
    Vim,
}

impl EditingMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Standard => "Standard",
            Self::Vim => "Vim",
        }
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Keybindings {
    pub prefix: String,
}

impl Default for Keybindings {
    fn default() -> Self {
        Self {
            prefix: "ctrl+x".into(),
        }
    }
}

impl Settings {
    pub fn path() -> Result<PathBuf> {
        let base =
            directories::BaseDirs::new().context("Could not locate your configuration folder")?;
        Ok(config_path(
            base.home_dir(),
            base.config_dir(),
            std::env::var_os("XDG_CONFIG_HOME").as_deref(),
        ))
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

        let text = std::str::from_utf8(&bytes).context("Config must be UTF-8 text")?;
        let settings: Self = toml_edit::de::from_str(text).map_err(|_| {
            anyhow::anyhow!("Invalid config.toml. Check the setting names and TOML syntax")
        })?;
        crate::keybindings::Prefix::new(&settings.keybindings.prefix)?;
        Ok(settings)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        crate::keybindings::Prefix::new(&self.keybindings.prefix)?;
        // Validate existing contents before editing them, and retain user comments.
        Self::load(path)?;
        let mut document = match std::fs::read_to_string(path) {
            Ok(text) => text
                .parse::<toml_edit::DocumentMut>()
                .map_err(|_| anyhow::anyhow!("Invalid config.toml. Check the TOML syntax"))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                toml_edit::DocumentMut::new()
            }
            Err(error) => return Err(error.into()),
        };
        if let Some(folder) = &self.resume_folder {
            document["resume_folder"] = toml_edit::value(folder.to_string_lossy().as_ref());
        }
        document["keybindings"]["prefix"] = toml_edit::value(&self.keybindings.prefix);

        document["editor"]["soft_wrap"] = toml_edit::value(self.editor.soft_wrap);
        document["editor"]["mode"] = toml_edit::value(self.editor.mode.label().to_lowercase());

        let text = document.to_string();
        ensure!(
            text.len() as u64 <= MAX_SETTINGS_BYTES,
            "Settings file is too large"
        );

        let parent = path.parent().context("Settings need a parent folder")?;
        std::fs::create_dir_all(parent).context("Could not create the settings folder")?;
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        file.write_all(text.as_bytes())?;
        file.persist(path).context("Could not save settings")?;
        Ok(())
    }

    pub fn ensure_file(path: &Path) -> Result<()> {
        if path.exists() {
            return Ok(());
        }

        let parent = path.parent().context("Config needs a parent folder")?;
        std::fs::create_dir_all(parent)?;
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        let defaults = concat!(
            "resume_folder = \"~/resumes\"\n\n",
            "[keybindings]\nprefix = \"ctrl+x\"\n\n",
            "[editor]\nsoft_wrap = true\nmode = \"standard\"\n",
        );
        file.write_all(defaults.as_bytes())?;
        file.persist_noclobber(path)
            .context("Could not create config.toml")?;
        Ok(())
    }
}

/// Ask macOS for a text editor rather than relying on a .toml file association.
pub fn open_editor(path: &Path) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        let status = text_editor_command(path).status()?;
        ensure!(status.success(), "Could not launch the text editor");
        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    {
        open::that_detached(path).map_err(Into::into)
    }
}

#[cfg(target_os = "macos")]
fn text_editor_command(path: &Path) -> std::process::Command {
    let mut command = std::process::Command::new("/usr/bin/open");
    command
        .arg("-t")
        .arg(path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    command
}

fn config_path(home: &Path, windows_config: &Path, xdg: Option<&std::ffi::OsStr>) -> PathBuf {
    if cfg!(windows) {
        return windows_config.join("texo/config.toml");
    }

    let base = xdg
        .map(Path::new)
        .filter(|path| path.is_absolute())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| home.join(".config"));
    base.join("texo/config.toml")
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

    #[cfg(target_os = "macos")]
    #[test]
    fn config_opener_requests_a_text_editor_and_keeps_paths_as_one_argument() {
        let path = Path::new("/tmp/folder with spaces/config.toml");
        let command = text_editor_command(path);
        let arguments: Vec<_> = command.get_args().collect();

        assert_eq!(command.get_program(), "/usr/bin/open");
        assert_eq!(arguments, [std::ffi::OsStr::new("-t"), path.as_os_str()]);
    }

    #[test]
    fn config_paths_use_xdg_or_home_on_unix() {
        if cfg!(windows) {
            assert_eq!(
                config_path(Path::new("/home/user"), Path::new("/appdata"), None),
                PathBuf::from("/appdata/texo/config.toml")
            );
            return;
        }

        let home = Path::new("/home/user");
        assert_eq!(
            config_path(home, home, None),
            home.join(".config/texo/config.toml")
        );
        assert_eq!(
            config_path(home, home, Some(std::ffi::OsStr::new("/custom"))),
            PathBuf::from("/custom/texo/config.toml")
        );
        assert_eq!(
            config_path(home, home, Some(std::ffi::OsStr::new("relative"))),
            home.join(".config/texo/config.toml")
        );
    }

    #[test]
    fn config_creation_preserves_existing_files_and_saves_keep_comments_and_prefix() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("config.toml");
        Settings::ensure_file(&path)?;
        assert_eq!(Settings::load(&path)?.keybindings.prefix, "ctrl+x");

        let original = r#"# My preferences
resume_folder = "~/resumes"

# Keyboard shortcuts
[keybindings]
prefix = "ctrl+a"
"#;
        std::fs::write(&path, original)?;
        Settings::ensure_file(&path)?;
        assert_eq!(std::fs::read_to_string(&path)?, original);

        let mut settings = Settings::load(&path)?;
        settings.resume_folder = Some(directory.path().to_path_buf());
        settings.save(&path)?;
        let saved = std::fs::read_to_string(&path)?;
        assert!(saved.contains("# My preferences") && saved.contains("# Keyboard shortcuts"));
        assert_eq!(Settings::load(&path)?.keybindings.prefix, "ctrl+a");
        Ok(())
    }

    #[test]
    fn invalid_keys_and_unknown_fields_are_rejected_without_echoing_contents() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("config.toml");
        for content in [
            "[keybindings]\nprefix = \"typo-value\"",
            "api_key = \"secret-value\"",
            "[keybindings]\nunknown = true",
        ] {
            std::fs::write(&path, content)?;
            let error = Settings::load(&path)
                .err()
                .expect("invalid config")
                .to_string();
            assert!(!error.contains("secret-value") && !error.contains("typo-value"));
        }
        Ok(())
    }

    #[test]
    fn settings_round_trip_without_authentication_fields() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let file = directory.path().join("config/config.toml");
        assert!(Settings::load(&file)?.resume_folder.is_none());
        let root = directory.path().canonicalize()?;
        Settings {
            resume_folder: Some(root.clone()),
            ..Settings::default()
        }
        .save(&file)?;
        assert_eq!(Settings::load(&file)?.resume_folder, Some(root));
        let contents = std::fs::read_to_string(file)?;
        assert!(!contents.contains("api_key"));
        Ok(())
    }

    #[test]
    fn invalid_oversized_and_unwritable_settings_fail() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let file = directory.path().join("config.toml");
        for contents in [
            "invalid".to_owned(),
            "x".repeat(MAX_SETTINGS_BYTES as usize + 1),
        ] {
            std::fs::write(&file, contents)?;
            assert!(Settings::load(&file).is_err());
        }
        assert!(Settings::default().save(&file.join("config.toml")).is_err());
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

#[cfg(test)]
mod editor_preference_tests {
    use super::*;

    #[test]
    fn editing_mode_defaults_to_standard_and_vim_round_trips() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("config.toml");
        let mut settings = Settings::load(&path)?;
        assert!(settings.editor.mode == EditingMode::Standard);
        settings.editor.mode = EditingMode::Vim;
        settings.save(&path)?;
        assert!(Settings::load(&path)?.editor.mode == EditingMode::Vim);
        std::fs::write(&path, "[editor]\nmode = \"invalid\"\n")?;
        assert!(Settings::load(&path).is_err());
        Ok(())
    }
}
