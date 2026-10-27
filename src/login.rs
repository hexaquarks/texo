use std::{path::PathBuf, sync::mpsc};

use anyhow::{Result, anyhow, ensure};
use ratatui::{
    crossterm::event::{
        Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
    },
    layout::Rect,
};
use secrecy::SecretString;
use texo::auth::{
    KeySource, Keychain, MAX_API_KEY_BYTES, Profile, Provider, SecretStore, resolve_key,
    validate_key,
};
use zeroize::{Zeroize, Zeroizing};

pub const MODEL_LOGIN_REQUIRED: &str = "Log in first with /login.";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Step {
    Provider,
    Method,
    Key,
    Model,
    LoadingModels,
    Waiting,
}

#[derive(Clone, Copy)]
enum Choice {
    OpenAi,
    Anthropic,
    ChatGpt,
    ApiKey,
    SavedApiKey,
    Environment,
    ExistingCodex,
}

impl Choice {
    fn label(self, provider: Provider) -> &'static str {
        match self {
            Self::OpenAi => "OpenAI / ChatGPT",
            Self::Anthropic => "Anthropic / Claude",
            Self::ChatGpt => "Sign in with ChatGPT (opens browser via Codex)",
            Self::ApiKey => "Sign in with a new API key",
            Self::SavedApiKey => "Use the saved API key",
            Self::Environment if provider == Provider::OpenAi => {
                "Use OPENAI_API_KEY from the environment"
            }
            Self::Environment => "Use ANTHROPIC_API_KEY from the environment",
            Self::ExistingCodex => "Use the existing Codex login",
        }
    }
}

pub enum LoginResult {
    Pending,
    Cancelled,
    Connected(Profile),
}

pub struct LoginDialog {
    step: Step,
    model_only: bool,
    selection: usize,
    provider: Provider,
    key: Zeroizing<String>,
    model: String,
    models: Vec<String>,
    model_job: Option<ModelJob>,
    model_loader: ModelLoader,
    use_environment: bool,
    reuse_saved_key: bool,
    error: String,
    process: Option<LoginProcess>,
    store: Box<dyn SecretStore>,
    preferences: Option<PathBuf>,
}

impl LoginDialog {
    pub fn new() -> Self {
        Self {
            step: Step::Provider,
            model_only: false,
            selection: 0,
            provider: Provider::OpenAi,
            key: Zeroizing::new(String::new()),
            model: String::new(),
            models: Vec::new(),
            model_job: None,
            model_loader: texo::llm::list_models,
            use_environment: false,
            reuse_saved_key: false,
            error: String::new(),
            process: None,
            store: Box::new(Keychain),
            preferences: None,
        }
    }

    pub fn for_model(profile: &Profile) -> Result<Self> {
        ensure!(
            profile.provider != Provider::Disconnected,
            MODEL_LOGIN_REQUIRED
        );
        let mut dialog = Self::new();
        dialog.model_only = true;
        dialog.provider = profile.provider;
        dialog.model = profile.model.clone();
        dialog.use_environment = profile.key_source == KeySource::Environment;
        dialog.reuse_saved_key = !dialog.use_environment;
        dialog.start_model_discovery()?;
        Ok(dialog)
    }

    pub fn is_loading_models(&self) -> bool {
        self.step == Step::LoadingModels
    }

    pub fn is_model_selection(&self) -> bool {
        self.model_only
    }

    fn choices(&self) -> &'static [Choice] {
        match self.step {
            Step::Provider => &[Choice::OpenAi, Choice::Anthropic],
            Step::Method if self.provider == Provider::OpenAi => &[
                Choice::ChatGpt,
                Choice::ApiKey,
                Choice::SavedApiKey,
                Choice::Environment,
                Choice::ExistingCodex,
            ],
            Step::Method => &[Choice::ApiKey, Choice::SavedApiKey, Choice::Environment],
            _ => &[],
        }
    }

    pub fn handle(&mut self, event: Event, area: Rect) -> LoginResult {
        let outcome = match event {
            Event::Key(key) => {
                if key.kind == KeyEventKind::Release {
                    return LoginResult::Pending;
                }
                if key.code == KeyCode::Esc
                    || (key.modifiers.contains(KeyModifiers::CONTROL)
                        && key.code == KeyCode::Char('c'))
                {
                    self.key.zeroize();
                    self.model_job = None;
                    self.process = None;
                    return LoginResult::Cancelled;
                }
                self.handle_key(key)
            }
            Event::Paste(mut text) => {
                if self.step == Step::Key {
                    let value = text.trim();
                    if validate_key(value).is_ok()
                        && self.key.len() + value.len() <= MAX_API_KEY_BYTES
                    {
                        self.key.push_str(value);
                        self.error.clear();
                    } else {
                        self.error = "Paste a single API key without spaces or line breaks".into();
                    }
                }
                text.zeroize();
                Ok(None)
            }
            Event::Mouse(mouse) if mouse.kind == MouseEventKind::Down(MouseButton::Left) => {
                let content = dialog_content(area);
                let row = mouse.row.saturating_sub(content.y + MENU_HEADER_ROWS);
                if self.step == Step::Model
                    && content.contains((mouse.column, mouse.row).into())
                    && mouse.row >= content.y.saturating_add(MENU_HEADER_ROWS)
                    && usize::from(row) < model_visible_rows(content)
                {
                    let offset = self
                        .selection
                        .saturating_sub(model_visible_rows(content).saturating_sub(1));
                    if offset + usize::from(row) < self.models.len() {
                        self.selection = offset + usize::from(row);
                        return match self
                            .handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
                        {
                            Ok(Some(profile)) => LoginResult::Connected(profile),
                            Err(error) => {
                                self.error = error.to_string();
                                LoginResult::Pending
                            }
                            _ => LoginResult::Pending,
                        };
                    }
                }
                if content.contains((mouse.column, mouse.row).into())
                    && mouse.row >= content.y.saturating_add(MENU_HEADER_ROWS)
                    && usize::from(row) < self.choices().len()
                {
                    self.selection = usize::from(row);
                    self.activate()
                } else {
                    Ok(None)
                }
            }
            _ => Ok(None),
        };
        match outcome {
            Ok(Some(profile)) => LoginResult::Connected(profile),
            Ok(None) => LoginResult::Pending,
            Err(error) => {
                self.error = error.to_string();
                LoginResult::Pending
            }
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<Option<Profile>> {
        if self.step == Step::Model {
            match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    self.selection = self.selection.saturating_sub(1)
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.selection = (self.selection + 1).min(self.models.len().saturating_sub(1))
                }
                KeyCode::Enter => {
                    if let Some(model) = self.models.get(self.selection) {
                        self.model = model.clone();
                        return self.save_model_profile().map(Some);
                    }
                    self.start_model_discovery()?;
                }
                _ => {}
            }
        } else if matches!(self.step, Step::Provider | Step::Method) {
            match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    self.selection = self.selection.saturating_sub(1)
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.selection = (self.selection + 1).min(self.choices().len() - 1)
                }
                KeyCode::Enter => return self.activate(),
                _ => {}
            }
        } else {
            match key.code {
                KeyCode::Enter if self.step == Step::Key => {
                    validate_key(&self.key)?;
                    self.start_model_discovery()?;
                }
                KeyCode::Backspace if self.step == Step::Key => {
                    self.key.pop();
                }
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    if self.step == Step::Key {
                        self.key.zeroize();
                    }
                }
                KeyCode::Char(character)
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                        && self.step == Step::Key
                        && self.key.len() < MAX_API_KEY_BYTES
                        && character.is_ascii_graphic() =>
                {
                    self.key.push(character);
                }
                _ => {}
            }
        }
        Ok(None)
    }

    fn activate(&mut self) -> Result<Option<Profile>> {
        let Some(choice) = self.choices().get(self.selection).copied() else {
            return Ok(None);
        };
        self.error.clear();
        match choice {
            Choice::OpenAi | Choice::Anthropic => {
                self.provider = if matches!(choice, Choice::OpenAi) {
                    Provider::OpenAi
                } else {
                    Provider::Anthropic
                };
                self.model.clear();
                self.step = Step::Method;
                self.selection = 0;
            }
            Choice::ChatGpt | Choice::ExistingCodex => {
                self.process = Some(LoginProcess::start(matches!(
                    choice,
                    Choice::ExistingCodex
                ))?);
                self.step = Step::Waiting;
            }
            Choice::ApiKey | Choice::SavedApiKey | Choice::Environment => {
                self.use_environment = matches!(choice, Choice::Environment);
                self.reuse_saved_key = matches!(choice, Choice::SavedApiKey);
                self.step = if matches!(choice, Choice::ApiKey) {
                    Step::Key
                } else {
                    Step::Method
                };
                if self.reuse_saved_key && self.store.get(self.provider)?.is_none() {
                    self.reuse_saved_key = false;
                    self.step = Step::Key;
                    self.error =
                        "No API key is saved yet. Paste your key to load available models".into();
                } else if self.use_environment || self.reuse_saved_key {
                    self.start_model_discovery()?;
                }
            }
        }
        Ok(None)
    }

    fn start_model_discovery(&mut self) -> Result<()> {
        if self.provider == Provider::Codex {
            // Codex uses the account login it manages, never an API key from our store.
            self.models = texo::llm::supported_models(Provider::Codex)
                .iter()
                .map(|model| (*model).to_owned())
                .collect();
            self.selection = self
                .models
                .iter()
                .position(|model| model == &self.model)
                .unwrap_or(0);
            self.step = Step::Model;
            return Ok(());
        }
        let profile = Profile {
            provider: self.provider,
            key_source: if self.use_environment {
                KeySource::Environment
            } else {
                KeySource::Saved
            },
            model: String::new(),
        };
        let key = if self.use_environment || self.reuse_saved_key {
            resolve_key(&profile, self.store.as_ref()).inspect_err(|_| {
                self.step = Step::Method;
            })?
        } else {
            validate_key(&self.key)?;
            SecretString::from(self.key.to_string())
        };
        // Only enter discovery after credentials exist; an account login is not an API key.
        self.models.clear();
        self.selection = 0;
        self.error.clear();
        self.model_job = Some(ModelJob::start(self.provider, key, self.model_loader));
        self.step = Step::LoadingModels;
        Ok(())
    }

    fn save_model_profile(&mut self) -> Result<Profile> {
        let profile = Profile {
            provider: self.provider,
            model: self.model.trim().into(),
            key_source: if self.use_environment {
                KeySource::Environment
            } else {
                KeySource::Saved
            },
        };
        profile.validate()?;
        if self.provider == Provider::Codex {
            self.save_preferences(&profile)?;
            return Ok(profile);
        }
        if self.use_environment {
            let variable = self.provider.key_variable().unwrap();
            let value = std::env::var(variable)
                .map_err(|_| anyhow!("{variable} is not set in this process"))?;
            let value = Zeroizing::new(value);
            validate_key(&value)?;
        } else if self.reuse_saved_key {
            let key = self.store.get(self.provider)?.ok_or_else(|| {
                anyhow!("No key is saved for this provider. Cancel and choose a new API key")
            })?;
            validate_key(secrecy::ExposeSecret::expose_secret(&key))?;
        } else {
            validate_key(&self.key)?;
            let secret = SecretString::from(self.key.to_string());
            self.store.set(self.provider, &secret)?;
        }
        self.save_preferences(&profile)?;
        self.key.zeroize();
        Ok(profile)
    }

    fn save_preferences(&self, profile: &Profile) -> Result<()> {
        match &self.preferences {
            Some(path) => profile.save_to(path),
            None => profile.save(),
        }
    }

    pub fn poll(&mut self) -> Option<Profile> {
        if let Some(job) = &self.model_job {
            match job.receiver.try_recv() {
                Ok(result) => {
                    self.model_job = None;
                    self.step = Step::Model;
                    match result {
                        Ok(models) => {
                            self.selection = models
                                .iter()
                                .position(|model| model == &self.model)
                                .unwrap_or(0);
                            self.models = models;
                        }
                        Err(error) => self.error = error.to_string(),
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.model_job = None;
                    self.step = Step::Model;
                    self.error = "Model discovery stopped. Press Enter to retry".into();
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
            return None;
        }
        let process = self.process.as_mut()?;
        let outcome = process.poll()?;
        self.process = None;
        match outcome.and_then(|()| {
            let profile = Profile::new(Provider::Codex);
            self.save_preferences(&profile)?;
            Ok(profile)
        }) {
            Ok(profile) => Some(profile),
            Err(error) => {
                self.error = error.to_string();
                self.step = Step::Method;
                None
            }
        }
    }
}
#[path = "login_job.rs"]
mod login_job;
#[path = "login_view.rs"]
mod login_view;

use login_job::{LoginProcess, ModelJob, ModelLoader};
use login_view::{MENU_HEADER_ROWS, dialog_content, model_visible_rows};

#[cfg(test)]
#[path = "login_tests.rs"]
mod tests;
