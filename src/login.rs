use std::{
    path::PathBuf,
    process::{Child, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{Result, anyhow};
use ratatui::{
    Frame,
    crossterm::event::{
        Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
    },
    layout::{Margin, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Paragraph, Wrap},
};
use secrecy::SecretString;
use texo::{
    auth::{
        CODEX_STATUS_TIMEOUT, KeySource, Keychain, MAX_API_KEY_BYTES, Profile, Provider,
        SecretStore, codex_command, resolve_key, validate_key,
    },
    theme,
};
use zeroize::{Zeroize, Zeroizing};

const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);
const MENU_HEADER_ROWS: u16 = 2;

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
                    text.zeroize();
                }
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
                KeyCode::Up => self.selection = self.selection.saturating_sub(1),
                KeyCode::Down => {
                    self.selection = (self.selection + 1).min(self.models.len().saturating_sub(1))
                }
                KeyCode::Enter => {
                    if let Some(model) = self.models.get(self.selection) {
                        self.model = model.clone();
                        return self.save_api_profile().map(Some);
                    }
                    self.start_model_discovery()?;
                }
                _ => {}
            }
        } else if matches!(self.step, Step::Provider | Step::Method) {
            match key.code {
                KeyCode::Up => self.selection = self.selection.saturating_sub(1),
                KeyCode::Down => {
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
            resolve_key(&profile, self.store.as_ref()).map_err(|_| {
                self.step = Step::Method;
                anyhow!("The selected API key is unavailable. Choose a new API key or another sign-in method")
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

    fn save_api_profile(&mut self) -> Result<Profile> {
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
                        Ok(models) => self.models = models,
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

    pub fn draw(&self, frame: &mut Frame, area: Rect) {
        frame.render_widget(Block::default().style(theme::text()), area);
        let content = dialog_content(area);
        let title = match self.step {
            Step::Provider => "Connect an assistant",
            Step::Method => "Select a sign-in method",
            Step::Key => "Enter your API key",
            Step::Model => "Choose a model · newest first",
            Step::LoadingModels => "Fetching available models…",
            Step::Waiting => "Complete sign-in",
        };
        let mut lines = vec![
            Line::styled(title, Style::default().fg(theme::LAVENDER)),
            Line::default(),
        ];
        if matches!(self.step, Step::Provider | Step::Method) {
            for (index, choice) in self.choices().iter().enumerate() {
                let selected = index == self.selection;
                lines.push(Line::styled(
                    format!(
                        "{} {}",
                        if selected { "›" } else { " " },
                        choice.label(self.provider)
                    ),
                    if selected {
                        theme::pane_title(true)
                    } else {
                        theme::text()
                    },
                ));
            }
            if self.step == Step::Method && self.provider == Provider::Anthropic {
                lines.push(Line::default());
                lines.push(Line::styled(
                    "Claude subscription login is not supported in this version.",
                    theme::muted(),
                ));
            }
        } else if self.step == Step::Model {
            let visible = model_visible_rows(content);
            let offset = self.selection.saturating_sub(visible.saturating_sub(1));
            for (index, model) in self.models.iter().enumerate().skip(offset).take(visible) {
                let selected = index == self.selection;
                lines.push(Line::styled(
                    format!("{} {}", if selected { "›" } else { " " }, model),
                    if selected {
                        theme::pane_title(true)
                    } else {
                        theme::text()
                    },
                ));
            }
            if self.models.is_empty() {
                lines.push(Line::styled(
                    "Press Enter to retry · Esc cancel",
                    theme::muted(),
                ));
            }
        } else {
            let text = match self.step {
                Step::Key => {
                    if self.key.is_empty() {
                        "Paste your key here (hidden)".into()
                    } else {
                        "•".repeat(self.key.len().min(32))
                    }
                }
                Step::LoadingModels => "Requesting the provider’s live catalog…".into(),
                Step::Waiting => {
                    if self.process.as_ref().is_some_and(|p| p.status_only) {
                        "Checking the existing Codex login…".into()
                    } else {
                        concat!(
                            "Finish signing in in your browser. Esc cancels.\n",
                            "If no browser opened, run `codex login` in another terminal, ",
                            "then retry using the existing login.",
                        )
                        .into()
                    }
                }
                _ => String::new(),
            };
            lines.push(Line::styled(text, theme::pane_title(true)));
            if self.step == Step::Key {
                lines.push(Line::default());
                lines.push(Line::styled(
                    "Saved in your OS credential store, never in the project.",
                    theme::muted(),
                ));
            }
        }
        lines.push(Line::default());
        lines.push(Line::styled(
            "↑↓ navigate · Enter select · Esc cancel",
            theme::muted(),
        ));
        if !self.error.is_empty() {
            lines.push(Line::default());
            lines.push(Line::from(Span::styled(
                &self.error,
                Style::default().fg(theme::ERROR),
            )));
        }
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), content);
    }
}

type ModelLoader = fn(Provider, SecretString, &AtomicBool) -> Result<Vec<String>>;

struct ModelJob {
    receiver: mpsc::Receiver<Result<Vec<String>>>,
    cancelled: Arc<AtomicBool>,
}

impl ModelJob {
    fn start(provider: Provider, key: SecretString, loader: ModelLoader) -> Self {
        let (sender, receiver) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = Arc::clone(&cancelled);
        thread::spawn(move || {
            let _ = sender.send(loader(provider, key, &worker_cancelled));
        });
        Self {
            receiver,
            cancelled,
        }
    }
}

impl Drop for ModelJob {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

fn model_visible_rows(content: Rect) -> usize {
    const FOOTER_ROWS: u16 = 4;
    usize::from(
        content
            .height
            .saturating_sub(MENU_HEADER_ROWS + FOOTER_ROWS),
    )
    .max(1)
}

fn dialog_content(area: Rect) -> Rect {
    area.inner(Margin {
        horizontal: theme::PANEL_PADDING,
        vertical: 1,
    })
}

struct LoginProcess {
    child: Child,
    started: Instant,
    status_only: bool,
}
impl LoginProcess {
    fn start(status_only: bool) -> Result<Self> {
        let mut command = codex_command();
        command.arg("login");
        if status_only {
            command.arg("status");
        }
        command
            .env_remove("OPENAI_API_KEY")
            .env_remove("ANTHROPIC_API_KEY")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let child = command.spawn().map_err(|_| {
            anyhow!("Codex is not installed. Install it for ChatGPT sign-in, or choose an API key")
        })?;
        Ok(Self {
            child,
            started: Instant::now(),
            status_only,
        })
    }

    fn poll(&mut self) -> Option<Result<()>> {
        match self.child.try_wait() {
            Ok(Some(status)) => Some(if status.success() {
                Ok(())
            } else {
                Err(anyhow!(
                    "Sign-in did not complete. Retry, or run `codex login` in your terminal"
                ))
            }),
            Ok(None)
                if self.started.elapsed()
                    < if self.status_only {
                        CODEX_STATUS_TIMEOUT
                    } else {
                        LOGIN_TIMEOUT
                    } =>
            {
                None
            }
            Ok(None) => Some(Err(anyhow!("Sign-in timed out. Use /login to retry"))),
            Err(_) => Some(Err(anyhow!(
                "Could not check sign-in status. Use /login to retry"
            ))),
        }
    }
}
impl Drop for LoginProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::ensure;
    use ratatui::{Terminal, backend::TestBackend};
    use secrecy::ExposeSecret;
    use std::process::Command;
    use std::{cell::RefCell, rc::Rc};

    struct TestStore {
        saved: Rc<RefCell<Option<String>>>,
        fail: bool,
    }
    impl SecretStore for TestStore {
        fn get(&self, _: Provider) -> Result<Option<SecretString>> {
            Ok(self
                .saved
                .borrow()
                .as_ref()
                .map(|key| SecretString::from(key.clone())))
        }
        fn set(&self, _: Provider, key: &SecretString) -> Result<()> {
            ensure!(!self.fail, "Credential store is locked");
            *self.saved.borrow_mut() = Some(key.expose_secret().into());
            Ok(())
        }
        fn delete(&self, _: Provider) -> Result<()> {
            Ok(())
        }
    }
    fn dialog(fail: bool) -> (tempfile::TempDir, LoginDialog, Rc<RefCell<Option<String>>>) {
        let directory = tempfile::tempdir().unwrap();
        let saved = Rc::new(RefCell::new(None));
        let mut dialog = LoginDialog::new();
        dialog.model_loader = |_, _, _| Ok(vec!["test-model-new".into(), "test-model-old".into()]);
        dialog.preferences = Some(directory.path().join("provider.json"));
        dialog.store = Box::new(TestStore {
            saved: Rc::clone(&saved),
            fail,
        });
        (directory, dialog, saved)
    }
    fn enter(dialog: &mut LoginDialog) -> LoginResult {
        let deadline = Instant::now() + Duration::from_secs(2);
        while dialog.step == Step::LoadingModels && Instant::now() < deadline {
            dialog.poll();
            thread::yield_now();
        }
        dialog.handle(
            Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            Rect::new(0, 0, 80, 20),
        )
    }

    #[test]
    fn api_key_flow_masks_secrets_and_persists_only_preferences_for_both_providers() {
        for provider in [Provider::OpenAi, Provider::Anthropic] {
            let (_directory, mut dialog, saved) = dialog(false);
            dialog.selection = usize::from(provider == Provider::Anthropic);
            enter(&mut dialog);
            dialog.selection = usize::from(provider == Provider::OpenAi);
            enter(&mut dialog);
            assert!(dialog.step == Step::Key);
            dialog.handle(
                Event::Paste("test-secret-never-render".into()),
                Rect::default(),
            );
            let mut terminal = Terminal::new(TestBackend::new(90, 20)).unwrap();
            terminal
                .draw(|frame| dialog.draw(frame, frame.area()))
                .unwrap();
            let text: String = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|c| c.symbol())
                .collect();
            assert!(!text.contains("test-secret"));
            enter(&mut dialog);
            assert!(matches!(enter(&mut dialog), LoginResult::Connected(_)));
            assert!(dialog.key.is_empty());
            assert_eq!(saved.borrow().as_deref(), Some("test-secret-never-render"));
            let contents = std::fs::read_to_string(dialog.preferences.as_ref().unwrap()).unwrap();
            assert!(!contents.contains("test-secret"));
            assert_eq!(
                Profile::load_from(dialog.preferences.as_ref().unwrap())
                    .unwrap()
                    .provider,
                provider
            );
        }
    }

    #[test]
    fn existing_saved_keys_can_be_selected_without_reentering_them() {
        let (_directory, mut dialog, saved) = dialog(false);
        *saved.borrow_mut() = Some("saved-test-secret".into());
        dialog.provider = Provider::Anthropic;
        dialog.step = Step::Model;
        dialog.models = vec!["test-model".into()];
        dialog.reuse_saved_key = true;
        assert!(matches!(enter(&mut dialog), LoginResult::Connected(_)));
        assert!(dialog.key.is_empty());
        assert_eq!(saved.borrow().as_deref(), Some("saved-test-secret"));
    }

    #[test]
    fn failed_storage_and_invalid_paste_do_not_change_the_profile() {
        let (_directory, mut dialog, saved) = dialog(true);
        dialog.provider = Provider::Anthropic;
        dialog.model = dialog.provider.default_model().into();
        dialog.step = Step::Key;
        dialog.handle(Event::Paste("secret\nsecond-line".into()), Rect::default());
        assert!(dialog.key.is_empty());
        dialog.handle(Event::Paste("test-secret".into()), Rect::default());
        enter(&mut dialog);
        assert!(matches!(enter(&mut dialog), LoginResult::Pending));
        assert!(dialog.error.contains("locked"));
        assert!(saved.borrow().is_none());
        assert!(!dialog.preferences.as_ref().unwrap().exists());
    }

    #[cfg(unix)]
    #[test]
    fn existing_account_success_failure_and_timeout_use_the_same_dialog() {
        for (script, expected_success) in [
            ("exit 0", true),
            ("exit 1", false),
            ("exec sleep 10", false),
        ] {
            let (_directory, mut dialog, _saved) = dialog(false);
            let child = Command::new("sh").args(["-c", script]).spawn().unwrap();
            dialog.process = Some(LoginProcess {
                child,
                started: Instant::now() - LOGIN_TIMEOUT,
                status_only: true,
            });
            dialog.step = Step::Waiting;
            // Give the stub process a bounded opportunity to exit before checking the timeout path.
            if script.starts_with("exit") {
                dialog.process.as_mut().unwrap().child.wait().unwrap();
            }
            let profile = dialog.poll();
            assert_eq!(profile.is_some(), expected_success);
            if expected_success {
                assert_eq!(profile.unwrap().provider, Provider::Codex);
                assert_eq!(
                    Profile::load_from(dialog.preferences.as_ref().unwrap())
                        .unwrap()
                        .provider,
                    Provider::Codex
                );
            } else {
                assert!(!dialog.error.is_empty());
                assert!(dialog.step == Step::Method);
                assert!(!dialog.preferences.as_ref().unwrap().exists());
            }
        }
    }

    #[test]
    fn missing_saved_key_prompts_for_a_key_before_fetching_models() {
        for provider in [Provider::OpenAi, Provider::Anthropic] {
            let (_directory, mut dialog, _) = dialog(false);
            dialog.provider = provider;
            dialog.step = Step::Method;
            dialog.selection = if provider == Provider::OpenAi { 2 } else { 1 };
            enter(&mut dialog);
            assert!(dialog.step == Step::Key);
            assert!(dialog.model_job.is_none());
            assert!(!dialog.reuse_saved_key);
            dialog.handle(Event::Paste("new-test-key".into()), Rect::default());
            enter(&mut dialog);
            assert!(dialog.step == Step::LoadingModels);
            assert!(matches!(enter(&mut dialog), LoginResult::Connected(_)));
        }
    }

    #[test]
    fn saved_key_is_passed_to_discovery_before_model_selection() {
        let (_directory, mut dialog, saved) = dialog(false);
        *saved.borrow_mut() = Some("saved-discovery-key".into());
        dialog.model_loader = |provider, key, _| {
            assert_eq!(provider, Provider::OpenAi);
            assert_eq!(key.expose_secret(), "saved-discovery-key");
            Ok(vec!["gpt-live-catalog".into()])
        };
        dialog.step = Step::Method;
        dialog.selection = 2;
        enter(&mut dialog);
        assert!(dialog.step == Step::LoadingModels);
        let LoginResult::Connected(profile) = enter(&mut dialog) else {
            panic!("expected selected model")
        };
        assert_eq!(profile.model, "gpt-live-catalog");
    }

    #[test]
    fn model_picker_scrolls_and_saves_the_selected_live_model() {
        let (_directory, mut dialog, _) = dialog(false);
        dialog.step = Step::Model;
        dialog.key.push_str("test-secret");
        dialog.models = (0..40).map(|i| format!("gpt-live-{i}")).collect();
        for _ in 0..25 {
            dialog
                .handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
                .unwrap();
        }
        dialog
            .handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE))
            .unwrap();
        let mut terminal = Terminal::new(TestBackend::new(70, 12)).unwrap();
        terminal
            .draw(|frame| dialog.draw(frame, frame.area()))
            .unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains("› gpt-live-24"));
        let LoginResult::Connected(profile) = enter(&mut dialog) else {
            panic!("model not saved")
        };
        assert_eq!(profile.model, "gpt-live-24");
    }

    #[test]
    fn model_discovery_errors_allow_retry_without_saving_credentials() {
        let (_directory, mut dialog, saved) = dialog(false);
        dialog.key.push_str("test-secret");
        dialog.model_loader = |_, _, _| Err(anyhow!("Network unavailable"));
        dialog.start_model_discovery().unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while dialog.step == Step::LoadingModels && Instant::now() < deadline {
            dialog.poll();
            thread::yield_now();
        }
        assert!(dialog.error.contains("Network unavailable"));
        assert!(saved.borrow().is_none());
        assert!(!dialog.preferences.as_ref().unwrap().exists());
        dialog.model_loader = |_, _, _| Ok(vec!["gpt-new-from-api".into()]);
        enter(&mut dialog);
        assert!(matches!(enter(&mut dialog), LoginResult::Connected(_)));
    }

    #[test]
    fn cancelling_and_tiny_terminals_are_safe() {
        let (_directory, mut dialog, saved) = dialog(false);
        dialog.step = Step::Key;
        dialog.key.push_str("discard-secret");
        assert!(matches!(
            dialog.handle(
                Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
                Rect::default()
            ),
            LoginResult::Cancelled
        ));
        for (width, height) in [(1, 1), (20, 5)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| dialog.draw(frame, frame.area()))
                .unwrap();
        }
        assert!(saved.borrow().is_none());
    }
}
