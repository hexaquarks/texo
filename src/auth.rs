use std::{
    env, fs,
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{Result, anyhow, ensure};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};

const KEYCHAIN_SERVICE: &str = "texo.llm";
pub const MAX_MODEL_BYTES: usize = 100;
pub const MAX_API_KEY_BYTES: usize = 4096;
pub const CODEX_STATUS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    #[default]
    Codex,
    OpenAi,
    Anthropic,
    Disconnected,
}

impl Provider {
    pub fn label(self) -> &'static str {
        match self {
            Self::Codex => "ChatGPT / Codex",
            Self::OpenAi => "OpenAI API",
            Self::Anthropic => "Anthropic API",
            Self::Disconnected => "Not connected",
        }
    }

    pub fn default_model(self) -> &'static str {
        match self {
            Self::OpenAi => "gpt-5.4",
            Self::Anthropic => "claude-sonnet-4-6",
            _ => "",
        }
    }

    pub fn key_variable(self) -> Option<&'static str> {
        match self {
            Self::OpenAi => Some("OPENAI_API_KEY"),
            Self::Anthropic => Some("ANTHROPIC_API_KEY"),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum KeySource {
    #[default]
    Saved,
    Environment,
}

/// Preferences contain only the provider, model, and credential source. Credentials never belong in this file.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub provider: Provider,
    pub model: String,
    pub key_source: KeySource,
}

impl Profile {
    pub fn new(provider: Provider) -> Self {
        Self {
            provider,
            model: provider.default_model().into(),
            key_source: KeySource::Saved,
        }
    }

    pub fn validate(&self) -> Result<()> {
        if self.provider.key_variable().is_some() {
            ensure!(
                !self.model.is_empty()
                    && self.model.len() <= MAX_MODEL_BYTES
                    && self
                        .model
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || "-._".contains(c)),
                "Enter a valid model ID (letters, numbers, hyphens, dots, or underscores)"
            );
        }
        Ok(())
    }

    pub fn load() -> Result<Self> {
        Self::load_from(&profile_path()?)
    }

    pub fn save(&self) -> Result<()> {
        self.save_to(&profile_path()?)
    }

    pub fn load_from(path: &Path) -> Result<Self> {
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(_) => return Err(anyhow!("Could not read Texo's provider preferences")),
        };
        let profile: Self = serde_json::from_slice(&bytes).map_err(|_| {
            anyhow!("Invalid provider preferences. Use /login to choose a provider again")
        })?;
        profile.validate()?;
        Ok(profile)
    }

    pub fn save_to(&self, path: &Path) -> Result<()> {
        self.validate()?;
        let directory = path
            .parent()
            .ok_or_else(|| anyhow!("Invalid preferences directory"))?;
        fs::create_dir_all(directory)
            .map_err(|_| anyhow!("Could not create Texo's preferences directory"))?;
        let mut file = tempfile::NamedTempFile::new_in(directory)
            .map_err(|_| anyhow!("Could not save provider preferences"))?;
        file.write_all(&serde_json::to_vec(self)?)?;
        file.persist(path)
            .map_err(|_| anyhow!("Could not save provider preferences"))?;
        Ok(())
    }
}

fn profile_path() -> Result<PathBuf> {
    let project = directories::ProjectDirs::from("", "", "texo")
        .ok_or_else(|| anyhow!("Could not locate Texo's user configuration directory"))?;
    Ok(project.config_dir().join("provider.json"))
}

/// Codex receives only the environment needed to find its executable, login, and OS services.
/// API keys and unrelated project secrets must not reach its subprocess or shell snapshots.
pub fn codex_command() -> std::process::Command {
    let mut command = std::process::Command::new("codex");
    configure_codex_environment(&mut command);
    command
}

fn configure_codex_environment(command: &mut std::process::Command) {
    command.env_clear();
    for name in [
        "PATH",
        "HOME",
        "CODEX_HOME",
        "USER",
        "USERNAME",
        "USERPROFILE",
        "APPDATA",
        "LOCALAPPDATA",
        "SystemRoot",
        "WINDIR",
        "TMPDIR",
        "TEMP",
        "TMP",
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "XDG_CACHE_HOME",
        "SSL_CERT_FILE",
        "SSL_CERT_DIR",
    ] {
        if let Some(value) = env::var_os(name) {
            command.env(name, value);
        }
    }
}

pub fn validate_key(key: &str) -> Result<()> {
    ensure!(
        !key.is_empty()
            && key.len() <= MAX_API_KEY_BYTES
            && key.bytes().all(|byte| byte.is_ascii_graphic()),
        "Enter a nonempty API key without spaces or line breaks"
    );
    Ok(())
}

/// This boundary also lets tests exercise credential failures without accessing a user's keychain.
pub trait SecretStore {
    fn get(&self, provider: Provider) -> Result<Option<SecretString>>;
    fn set(&self, provider: Provider, key: &SecretString) -> Result<()>;
    fn delete(&self, provider: Provider) -> Result<()>;
}

pub struct Keychain;

impl Keychain {
    fn entry(provider: Provider) -> Result<keyring::Entry> {
        let account = provider
            .key_variable()
            .ok_or_else(|| anyhow!("This provider does not use an API key"))?;
        keyring::Entry::new(KEYCHAIN_SERVICE, account)
            .map_err(|_| anyhow!("The OS credential store is unavailable. Use the provider's API key environment variable"))
    }
}

impl SecretStore for Keychain {
    fn get(&self, provider: Provider) -> Result<Option<SecretString>> {
        match Self::entry(provider)?.get_password() {
            Ok(key) => Ok(Some(key.into())),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err(anyhow!(
                "Could not unlock the OS credential store. Unlock it and retry /login"
            )),
        }
    }

    fn set(&self, provider: Provider, key: &SecretString) -> Result<()> {
        validate_key(key.expose_secret())?;
        Self::entry(provider)?.set_password(key.expose_secret())
            .map_err(|_| anyhow!("Could not save the key in the OS credential store. No plaintext copy was written"))
    }

    fn delete(&self, provider: Provider) -> Result<()> {
        match Self::entry(provider)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(anyhow!(
                "Could not remove the saved API key from the OS credential store"
            )),
        }
    }
}

pub fn resolve_key(profile: &Profile, store: &dyn SecretStore) -> Result<SecretString> {
    resolve_key_with(profile, store, |name| env::var(name).ok())
}

fn resolve_key_with(
    profile: &Profile,
    store: &dyn SecretStore,
    environment: impl FnOnce(&str) -> Option<String>,
) -> Result<SecretString> {
    let provider = profile.provider;
    let variable = provider
        .key_variable()
        .ok_or_else(|| anyhow!("This provider does not use an API key"))?;
    // Never silently switch credential sources or providers after a failed login.
    let key = match profile.key_source {
        KeySource::Environment => environment(variable).map(SecretString::from),
        KeySource::Saved => store.get(provider)?,
    }
    .ok_or_else(|| {
        anyhow!(
            "{} is not connected. Type /login to add an API key or choose another sign-in method",
            provider.label()
        )
    })?;
    validate_key(key.expose_secret())?;
    Ok(key)
}

pub fn disconnect(
    profile: &Profile,
    store: &dyn SecretStore,
    preferences: Option<&Path>,
) -> Result<()> {
    let disconnected = Profile::new(Provider::Disconnected);
    match preferences {
        Some(path) => disconnected.save_to(path)?,
        None => disconnected.save()?,
    }
    if profile.provider.key_variable().is_some() && profile.key_source == KeySource::Saved {
        store.delete(profile.provider)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    struct MemoryStore(RefCell<Option<SecretString>>);
    impl SecretStore for MemoryStore {
        fn get(&self, _: Provider) -> Result<Option<SecretString>> {
            Ok(self.0.borrow().clone())
        }
        fn set(&self, _: Provider, key: &SecretString) -> Result<()> {
            *self.0.borrow_mut() = Some(key.clone());
            Ok(())
        }
        fn delete(&self, _: Provider) -> Result<()> {
            *self.0.borrow_mut() = None;
            Ok(())
        }
    }

    #[test]
    fn preferences_round_trip_without_credentials_and_fail_closed_when_corrupt() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("provider.json");
        assert_eq!(Profile::load_from(&path).unwrap().provider, Provider::Codex);
        Profile::new(Provider::Anthropic).save_to(&path).unwrap();
        assert_eq!(
            Profile::load_from(&path).unwrap().model,
            "claude-sonnet-4-6"
        );
        let contents = fs::read_to_string(&path).unwrap();
        assert!(!contents.contains("api_key"));
        fs::write(&path, r#"{"provider":"unknown","model":"x"}"#).unwrap();
        assert!(Profile::load_from(&path).is_err());
    }

    #[test]
    fn credentials_use_only_the_selected_providers_environment() {
        let store = MemoryStore::default();
        store
            .set(Provider::OpenAi, &SecretString::from("saved-secret"))
            .unwrap();
        let mut profile = Profile::new(Provider::Anthropic);
        profile.key_source = KeySource::Environment;
        let key = resolve_key_with(&profile, &store, |name| {
            assert_eq!(name, "ANTHROPIC_API_KEY");
            Some("environment-secret".into())
        })
        .unwrap();
        assert_eq!(key.expose_secret(), "environment-secret");
        assert!(!format!("{key:?}").contains("environment-secret"));
        let key = resolve_key_with(&Profile::new(Provider::OpenAi), &store, |_| {
            panic!("Saved keys must not read environment variables")
        })
        .unwrap();
        assert_eq!(key.expose_secret(), "saved-secret");
        store.delete(Provider::OpenAi).unwrap();
        let error = resolve_key_with(&Profile::new(Provider::OpenAi), &store, |_| {
            panic!("Saved keys must not read environment variables")
        })
        .unwrap_err();
        assert!(error.to_string().contains("/login"));
    }

    #[test]
    fn explicit_key_sources_never_fall_back_to_each_other() {
        let store = MemoryStore::default();
        store
            .set(Provider::OpenAi, &SecretString::from("saved-secret"))
            .unwrap();
        let mut profile = Profile::new(Provider::OpenAi);
        let key = resolve_key_with(&profile, &store, |_| Some("other-secret".into())).unwrap();
        assert_eq!(key.expose_secret(), "saved-secret");
        profile.key_source = KeySource::Environment;
        assert!(resolve_key_with(&profile, &store, |_| None).is_err());
        assert!(resolve_key_with(&profile, &store, |_| Some("invalid value".into())).is_err());
    }

    #[test]
    fn logout_disconnects_even_when_credential_deletion_fails() {
        struct LockedStore;
        impl SecretStore for LockedStore {
            fn get(&self, _: Provider) -> Result<Option<SecretString>> {
                unreachable!()
            }
            fn set(&self, _: Provider, _: &SecretString) -> Result<()> {
                unreachable!()
            }
            fn delete(&self, _: Provider) -> Result<()> {
                Err(anyhow!("Locked credential store"))
            }
        }
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("provider.json");
        let profile = Profile::new(Provider::OpenAi);
        profile.save_to(&path).unwrap();
        assert!(disconnect(&profile, &LockedStore, Some(&path)).is_err());
        assert_eq!(
            Profile::load_from(&path).unwrap().provider,
            Provider::Disconnected
        );

        let store = MemoryStore::default();
        store
            .set(Provider::OpenAi, &SecretString::from("saved-secret"))
            .unwrap();
        disconnect(&profile, &store, Some(&path)).unwrap();
        assert!(store.get(Provider::OpenAi).unwrap().is_none());
    }

    #[cfg(unix)]
    #[test]
    fn subprocess_environment_does_not_inherit_project_secrets() {
        let mut command = std::process::Command::new("sh");
        command.env("TEXO_TEST_SECRET", "synthetic-secret");
        command.env("OPENAI_API_KEY", "synthetic-api-secret");
        configure_codex_environment(&mut command);
        let check_environment = r#"test -z "${TEXO_TEST_SECRET+x}" && test -z "${OPENAI_API_KEY+x}" && test -n "$PATH""#;
        let status = command.args(["-c", check_environment]).status().unwrap();
        assert!(status.success());
    }

    #[test]
    fn invalid_credentials_and_model_ids_are_rejected_without_echoing_them() {
        for key in ["", "secret\nvalue", "secret value"] {
            assert!(validate_key(key).is_err());
        }
        let mut profile = Profile::new(Provider::OpenAi);
        profile.model = "model\nAuthorization: secret".into();
        let error = profile.validate().unwrap_err().to_string();
        assert!(!error.contains("secret"));
    }
}
