use std::path::Path;

use crate::login::{LoginDialog, LoginResult, MODEL_LOGIN_REQUIRED};
use anyhow::{Result, ensure};
use ratatui::{
    crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers},
    layout::Rect,
    style::Style,
};
use ratatui_textarea::{TextArea, WrapMode};
use texo::{
    auth::{Keychain, Profile, Provider, disconnect},
    theme,
};

use crate::assistant_job::{AssistantJob, ConversationMessage};

const SCROLL_LINES: u16 = 3;
const CHAT_COMMANDS: [(&str, &str); 4] = [
    ("/settings", "Configure the resume folder"),
    ("/login", "Connect or switch an assistant provider"),
    ("/model", "Choose a model for the current provider"),
    (
        "/logout",
        "Disconnect Texo and remove the selected saved API key",
    ),
];

#[derive(Clone)]
pub struct ProposedEdit {
    pub source_before: String,
    pub source_after: String,
}

struct Message {
    role: &'static str,
    content: String,
    error: bool,
}

impl Message {
    fn new(role: &'static str, content: String, error: bool) -> Self {
        Self {
            role,
            content,
            error,
        }
    }
}

pub struct Assistant {
    pub input: TextArea<'static>,
    messages: Vec<Message>,
    job: Option<AssistantJob>,
    source_snapshot: String,
    proposed_edit: Option<ProposedEdit>,
    compiled_draft: Option<texo::sandbox::CompiledDraft>,
    scroll_from_bottom: u16,
    login: Option<LoginDialog>,
    command_selection: usize,
    profile: Profile,
}

impl Default for Assistant {
    fn default() -> Self {
        Self::new()
    }
}

impl Assistant {
    pub fn new() -> Self {
        Self {
            input: new_input(),
            messages: Vec::new(),
            job: None,
            source_snapshot: String::new(),
            proposed_edit: None,
            compiled_draft: None,
            scroll_from_bottom: 0,
            login: None,
            command_selection: 0,
            profile: Profile::load().unwrap_or_else(|_| Profile::new(Provider::Disconnected)),
        }
    }

    pub fn clear_input(&mut self) {
        self.input = new_input();
    }

    pub fn login_open(&self) -> bool {
        self.login.is_some()
    }

    pub fn handle_login_event(&mut self, event: Event, area: Rect) {
        let Some(dialog) = self.login.as_mut() else {
            return;
        };
        match dialog.handle(event, area) {
            LoginResult::Cancelled => self.login = None,
            LoginResult::Connected(profile) => self.connected(profile),
            LoginResult::Pending => {}
        }
    }

    fn connected(&mut self, profile: Profile) {
        let model_changed = self
            .login
            .as_ref()
            .is_some_and(LoginDialog::is_model_selection);
        self.login = None;
        self.profile = profile.clone();
        if model_changed {
            self.notice(format!("Model selected: {}", profile.model), false);
            return;
        }
        self.messages.clear();
        let mode = if profile.provider == Provider::Codex {
            "Connected through the existing Codex login.".into()
        } else {
            format!(
                "{} selected · {}. Credentials will be checked on your next message.",
                profile.provider.label(),
                profile.model
            )
        };
        self.notice(mode, false);
    }

    fn notice(&mut self, text: String, error: bool) {
        self.messages.push(Message::new("assistant", text, error));
        self.scroll_from_bottom = 0;
    }

    fn run_command(&mut self, command: &str) -> Result<()> {
        self.input = new_input();
        self.command_selection = 0;
        match command {
            "/login" => self.login = Some(LoginDialog::new()),
            "/model" => {
                let dialog = Profile::load()
                    .and_then(|profile| LoginDialog::for_model(&profile))
                    .map_err(|_| anyhow::anyhow!(MODEL_LOGIN_REQUIRED))?;
                self.login = Some(dialog);
            }
            "/logout" => {
                let profile = Profile::load()?;
                let result = disconnect(&profile, &Keychain, None);
                self.profile = Profile::new(Provider::Disconnected);
                result?;
                self.messages.clear();
                self.notice(
                    "Texo disconnected. External Codex sign-in and environment variables are unchanged.".into(),
                    false,
                );
            }
            _ => self.notice(
                "Unknown command. Use /login, /model, or /logout.".into(),
                true,
            ),
        }
        Ok(())
    }

    fn command_matches(&self) -> Vec<(&'static str, &'static str)> {
        if self.input.lines().len() != 1 {
            return Vec::new();
        }
        let text = self.input.lines()[0].trim();
        if !text.starts_with('/') || text.contains(char::is_whitespace) {
            return Vec::new();
        }
        CHAT_COMMANDS
            .iter()
            .copied()
            .filter(|(name, _)| name.starts_with(text))
            .collect()
    }

    pub fn complete_command(&mut self, key: KeyEvent) -> bool {
        let choices = self.command_matches();
        if choices.is_empty()
            || key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return false;
        }
        self.command_selection = self.command_selection.min(choices.len() - 1);
        match key.code {
            KeyCode::Up => self.command_selection = self.command_selection.saturating_sub(1),
            KeyCode::Down => {
                self.command_selection = (self.command_selection + 1).min(choices.len() - 1)
            }
            KeyCode::Tab | KeyCode::Enter => {
                self.input = new_input();
                self.input.insert_str(choices[self.command_selection].0);
                // Enter continues to normal submission; Tab only fills the command.
                return key.code == KeyCode::Tab;
            }
            _ => return false,
        }
        true
    }

    pub fn is_running(&self) -> bool {
        self.job.is_some()
    }

    pub fn submit(&mut self, source: String, path: &Path) -> Result<()> {
        ensure!(
            !self.is_running(),
            "Wait for Texo or cancel the current request"
        );
        let prompt = self.input.lines().join("\n");
        ensure!(!prompt.trim().is_empty(), "Type a message first");
        if prompt.trim_start().starts_with('/') {
            if let Err(error) = self.run_command(prompt.trim()) {
                self.notice(error.to_string(), true);
            }
            return Ok(());
        }
        let mut conversation: Vec<_> = self
            .messages
            .iter()
            .filter(|message| !message.error)
            .map(|message| ConversationMessage {
                role: message.role,
                content: message.content.clone(),
            })
            .collect();
        conversation.push(ConversationMessage {
            role: "user",
            content: prompt.clone(),
        });
        let job = AssistantJob::start(&source, path, &conversation)?;

        self.messages.push(Message::new("user", prompt, false));
        self.input = new_input();
        self.source_snapshot = source;
        self.proposed_edit = None;
        self.compiled_draft = None;
        self.scroll_from_bottom = 0;
        self.job = Some(job);
        Ok(())
    }

    #[cfg(test)]
    pub fn pending_request(
        &mut self,
        source: String,
    ) -> std::sync::mpsc::Sender<Result<crate::assistant_job::CompletedTask>> {
        let (job, sender) = AssistantJob::pending();
        self.source_snapshot = source;
        self.job = Some(job);

        sender
    }

    pub fn poll(&mut self) -> Option<Result<()>> {
        if let Some(profile) = self.login.as_mut().and_then(LoginDialog::poll) {
            self.connected(profile);
        }
        let result = self.job.as_ref()?.poll()?;
        self.job = None;
        self.scroll_from_bottom = 0;
        match result {
            Ok(completed) => {
                let response = completed.reply;
                self.compiled_draft = completed.build;
                self.proposed_edit = response.proposed_tex.map(|source_after| ProposedEdit {
                    source_before: self.source_snapshot.clone(),
                    source_after,
                });
                self.messages
                    .push(Message::new("assistant", response.reply, false));
                Some(Ok(()))
            }
            Err(error) => {
                self.messages
                    .push(Message::new("assistant", format!("{error:#}"), true));
                Some(Err(error))
            }
        }
    }

    /// Append the review outcome without rewriting the assistant's earlier summary.
    pub fn report_edit_result(&mut self, outcome: &str) {
        self.notice(outcome.to_owned(), false);
    }

    pub fn cancel(&mut self) {
        if self.job.take().is_some() {
            self.messages.push(Message::new(
                "assistant",
                "Request cancelled. You can send another message.".into(),
                true,
            ));
            self.scroll_from_bottom = 0;
        }
    }

    /// Cancel before clearing so a late reply can never appear in another document's chat.
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    pub fn proposal(&self) -> Option<&ProposedEdit> {
        self.proposed_edit.as_ref()
    }

    pub fn take_compiled_draft(&mut self) -> Option<texo::sandbox::CompiledDraft> {
        self.compiled_draft.take()
    }

    pub fn take_proposal(&mut self) -> Option<ProposedEdit> {
        self.compiled_draft = None;
        self.proposed_edit.take()
    }

    pub fn scroll(&mut self, up: bool) {
        if up {
            self.scroll_from_bottom = self.scroll_from_bottom.saturating_add(SCROLL_LINES);
        } else {
            self.scroll_from_bottom = self.scroll_from_bottom.saturating_sub(SCROLL_LINES);
        }
    }
}

fn new_input() -> TextArea<'static> {
    let mut input = TextArea::default();
    input.set_wrap_mode(WrapMode::WordOrGlyph);
    input.set_style(theme::text().bg(theme::BUTTON));
    input.set_cursor_line_style(Style::default());
    input.set_placeholder_text("Ask Texo...");
    input.set_placeholder_style(theme::muted().bg(theme::BUTTON));
    input
}

#[path = "chat_view.rs"]
mod chat_view;

#[cfg(test)]
#[path = "assistant_tests.rs"]
mod tests;
