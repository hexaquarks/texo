use std::path::Path;

use crate::login::{LoginDialog, LoginResult, MODEL_LOGIN_REQUIRED};
use anyhow::{Result, ensure};
use ratatui::{
    Frame,
    crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers},
    layout::{Alignment, Constraint, Layout, Margin, Position, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Padding, Paragraph},
};
use ratatui_textarea::{CursorMove, TextArea};
use texo::{
    auth::{Keychain, Profile, Provider, disconnect},
    theme,
};

use crate::assistant_job::{AssistantJob, ConversationMessage};

const INPUT_HEIGHT: u16 = 3;
const SEND_WIDTH: u16 = 8;
const MODEL_LABEL_HEIGHT: u16 = 1;
const COMPOSER_HEIGHT: u16 = INPUT_HEIGHT + MODEL_LABEL_HEIGHT;
const PROMPT_WIDTH: u16 = 2;
const SCROLL_LINES: u16 = 3;
const THINKING_VERTICAL_PADDING: u16 = 1;
const THINKING_HEIGHT: u16 = 1 + THINKING_VERTICAL_PADDING * 2;
const THINKING_FRAME_MS: u128 = 100;
const THINKING_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
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

    pub fn input_area(&self, area: Rect) -> Rect {
        composer_layout(area)[0]
    }

    fn model_label(&self) -> &str {
        match self.profile.provider {
            Provider::Codex if self.profile.model.is_empty() => "Codex · auto",
            Provider::Disconnected => "Not connected",
            _ => &self.profile.model,
        }
    }

    pub fn click(&mut self, area: Rect, column: u16, row: u16) -> bool {
        let content = input_block().inner(self.input_area(area));
        if !content.contains(Position::new(column, row)) {
            return false;
        }

        let (scroll_row, scroll_column) = self.input.scroll_offset();
        let display_row = usize::from(scroll_row) + usize::from(row - content.y);
        let display_column = usize::from(scroll_column) + usize::from(column - content.x);
        let position = self.input.screen_to_data(display_row, display_column);
        self.input.cancel_selection();
        self.input.move_cursor(CursorMove::Jump(
            u16::try_from(position.0).unwrap_or(u16::MAX),
            u16::try_from(position.1).unwrap_or(u16::MAX),
        ));
        true
    }

    pub fn send_button(area: Rect) -> Rect {
        composer_layout(area)[1]
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect, focused: bool) {
        if let Some(login) = &self.login
            && !login.is_loading_models()
        {
            login.draw(frame, area);
            return;
        }
        if area.width < SEND_WIDTH || area.height < INPUT_HEIGHT + 2 {
            return;
        }
        frame.render_widget(Block::default().style(theme::text()), area);
        let inner = area.inner(Margin {
            horizontal: theme::PANEL_PADDING,
            vertical: 1,
        });
        let activity_height = if self.is_running() {
            THINKING_HEIGHT
        } else {
            1
        };

        let [header, _, history, activity, composer] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(activity_height),
            Constraint::Length(COMPOSER_HEIGHT),
        ])
        .areas(inner);
        let hint = if self.is_running() {
            "Esc cancel"
        } else if self.proposal().is_some() {
            "Not applied · Actions to review"
        } else if focused {
            "Enter send · Alt+Enter newline"
        } else {
            ""
        };
        let [label, hint_area] =
            Layout::horizontal([Constraint::Length(13), Constraint::Min(0)]).areas(header);
        frame.render_widget(
            Paragraph::new("texo / chat").style(theme::pane_title(focused)),
            label,
        );
        frame.render_widget(
            Paragraph::new(hint)
                .alignment(Alignment::Right)
                .style(theme::muted()),
            hint_area,
        );
        self.draw_history(frame, history);

        if let Some(job) = &self.job {
            let phase =
                (job.elapsed().as_millis() / THINKING_FRAME_MS) % THINKING_FRAMES.len() as u128;
            let label = format!("{} Texo is thinking…", THINKING_FRAMES[phase as usize]);

            frame.render_widget(
                Paragraph::new(label).style(theme::text().fg(theme::ACCENT)),
                activity.inner(Margin::new(0, THINKING_VERTICAL_PADDING)),
            );
        }

        if focused {
            let choices = self.command_matches();
            if !choices.is_empty() {
                let count = u16::try_from(choices.len())
                    .unwrap_or(u16::MAX)
                    .min(history.height);
                let suggestion_area = Rect::new(
                    history.x,
                    history.bottom().saturating_sub(count),
                    history.width,
                    count,
                );
                frame.render_widget(
                    Block::default().style(theme::text().bg(theme::ELEVATED)),
                    suggestion_area,
                );
                let selected = self.command_selection.min(choices.len() - 1);
                let lines: Vec<_> = choices
                    .iter()
                    .enumerate()
                    .map(|(index, (name, description))| {
                        let style = if index == selected {
                            theme::text().fg(theme::ACCENT).bg(theme::SELECTION)
                        } else {
                            theme::muted()
                        };
                        Line::styled(
                            format!(
                                " {} {name}  {description}",
                                if index == selected { "›" } else { " " }
                            ),
                            style,
                        )
                    })
                    .collect();
                frame.render_widget(Paragraph::new(lines), suggestion_area);
            }
        }

        let [input_box, model] = composer_rows(composer);
        frame.render_widget(
            Block::default().style(Style::default().bg(theme::BUTTON)),
            input_box,
        );
        let [input, send] = composer_layout_from_row(input_box);
        frame.render_widget(
            Paragraph::new(format!(" {}", self.model_label())).style(theme::muted()),
            model,
        );
        self.input.set_block(input_block());
        self.input.set_cursor_style(if focused {
            Style::default().add_modifier(Modifier::REVERSED)
        } else {
            Style::default()
        });
        frame.render_widget(&self.input, input);
        let prompt = Rect::new(input.x + 1, input.y + 1, PROMPT_WIDTH, 1);
        frame.render_widget(
            Paragraph::new("› ").style(Style::default().fg(theme::TEAL).bg(theme::BUTTON)),
            prompt,
        );
        frame.render_widget(
            Paragraph::new(if self.is_running() {
                "[cancel]"
            } else {
                "[send]"
            })
            .right_aligned()
            .style(theme::muted().bg(theme::BUTTON)),
            send,
        );
    }

    fn draw_history(&mut self, frame: &mut Frame, area: Rect) {
        if area.is_empty() {
            return;
        }
        if self.messages.is_empty() {
            let introduction = "Ask Texo to refine the open resume.\nReview proposed edits before accepting; Ctrl+S saves accepted text.";
            frame.render_widget(Paragraph::new(introduction).style(theme::muted()), area);
            return;
        }

        let mut lines: Vec<Line<'static>> = self
            .messages
            .iter()
            .flat_map(|message| message_lines(message, area.width))
            .collect();
        // Keep a blank line between turns without wasting the last visible transcript row.
        lines.pop();
        let maximum_scroll = u16::try_from(lines.len())
            .unwrap_or(u16::MAX)
            .saturating_sub(area.height);
        self.scroll_from_bottom = self.scroll_from_bottom.min(maximum_scroll);
        let offset = maximum_scroll.saturating_sub(self.scroll_from_bottom);
        frame.render_widget(
            Paragraph::new(lines)
                .style(theme::text())
                .scroll((offset, 0)),
            area,
        );
    }
}

fn new_input() -> TextArea<'static> {
    let mut input = TextArea::default();
    input.set_style(theme::text().bg(theme::BUTTON));
    input.set_cursor_line_style(Style::default());
    input.set_placeholder_text("Ask Texo...");
    input.set_placeholder_style(theme::muted().bg(theme::BUTTON));
    input
}

fn composer_layout(area: Rect) -> [Rect; 2] {
    let inner = area.inner(Margin {
        horizontal: theme::PANEL_PADDING,
        vertical: 1,
    });
    let [_, composer] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(COMPOSER_HEIGHT)]).areas(inner);
    composer_layout_from_row(composer_rows(composer)[0])
}

fn composer_rows(area: Rect) -> [Rect; 2] {
    Layout::vertical([Constraint::Min(0), Constraint::Length(MODEL_LABEL_HEIGHT)]).areas(area)
}

fn composer_layout_from_row(area: Rect) -> [Rect; 2] {
    let [input, button_column] =
        Layout::horizontal([Constraint::Min(1), Constraint::Length(SEND_WIDTH)]).areas(area);
    let send = Rect::new(button_column.x, button_column.y + 1, button_column.width, 1);
    [input, send]
}

fn input_block() -> Block<'static> {
    Block::default()
        .style(Style::default().bg(theme::BUTTON))
        .padding(Padding::new(PROMPT_WIDTH + 1, 1, 1, 1))
}

fn message_lines(message: &Message, width: u16) -> Vec<Line<'static>> {
    let is_user = message.role == "user";
    let prefix = if is_user {
        "> "
    } else if message.error {
        "! "
    } else {
        "* "
    };
    let style = if message.error {
        theme::text().fg(theme::ERROR)
    } else if is_user {
        theme::text().fg(theme::ACCENT).bg(theme::ELEVATED)
    } else {
        theme::text()
    };
    let content_width = usize::from(width.saturating_sub(PROMPT_WIDTH + 2)).max(1);
    let padded_blank = || Line::from(" ".repeat(usize::from(width))).style(style);
    let mut lines = Vec::new();
    if is_user {
        lines.push(padded_blank());
    }
    let mut first_line = true;
    for paragraph in message.content.lines() {
        for text in textwrap::wrap(paragraph, content_width) {
            let marker = if first_line { prefix } else { "  " };
            first_line = false;
            let mut line = Line::from(format!(" {marker}{text}")).style(style);
            if is_user {
                let padding = usize::from(width).saturating_sub(line.width());
                line.spans.push(Span::raw(" ".repeat(padding)));
            }
            lines.push(line);
        }
    }
    if is_user {
        lines.push(padded_blank());
    }
    lines.push(Line::default());
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_confirmation_is_appended_and_scrolls_into_view() {
        let mut assistant = Assistant::new();
        assistant.notice("Earlier summary".into(), false);
        assistant.notice("Build failed".into(), true);
        assistant.scroll_from_bottom = SCROLL_LINES;

        assistant.report_edit_result("Proposal discarded; no changes accepted.");

        assert_eq!(assistant.messages.len(), 3);
        assert_eq!(assistant.messages[0].content, "Earlier summary");
        assert_eq!(assistant.messages[1].content, "Build failed");
        assert_eq!(
            assistant.messages[2].content,
            "Proposal discarded; no changes accepted."
        );
        assert!(!assistant.messages[2].error);
        assert_eq!(assistant.scroll_from_bottom, 0);
    }

    #[test]
    fn slash_commands_complete_dynamically_and_never_enter_model_history() {
        let mut assistant = Assistant::new();
        assistant.input.insert_str("/lo");
        assert_eq!(assistant.command_matches().len(), 2);
        assert!(assistant.complete_command(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)));
        assert_eq!(assistant.input.lines()[0], "/login");
        assistant
            .submit("resume".into(), Path::new("resume.tex"))
            .unwrap();
        assert!(assistant.login_open());
        assert!(!assistant.is_running());
        assert!(assistant.messages.is_empty());
        assistant.handle_login_event(
            Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            Rect::new(0, 0, 80, 20),
        );
        assert!(!assistant.login_open());
        assistant.input.insert_str("/unknown accidental-secret");
        assistant
            .submit("resume".into(), Path::new("resume.tex"))
            .unwrap();
        assert!(!assistant.is_running());
        assert!(
            !assistant
                .messages
                .last()
                .unwrap()
                .content
                .contains("accidental-secret")
        );
        assert!(assistant.messages.last().unwrap().error);
    }

    #[test]
    fn switching_providers_clears_old_conversation_without_changing_the_document() {
        let mut assistant = Assistant::new();
        assistant.messages.push(Message::new(
            "user",
            "Earlier private conversation".into(),
            false,
        ));
        assistant.connected(Profile::new(Provider::Anthropic));
        assert_eq!(assistant.messages.len(), 1);
        assert!(assistant.messages[0].content.contains("Anthropic"));
        assert!(!assistant.messages[0].content.contains("Earlier private"));
        assistant.input.insert_str("/logi");
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(90, 20)).unwrap();
        terminal
            .draw(|frame| assistant.draw(frame, frame.area(), true))
            .unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains("/login"));
        assert!(!text.contains("/logout"));
    }

    #[test]
    fn model_command_completes_and_current_model_renders_below_input() {
        let mut assistant = Assistant::new();
        assistant.profile = Profile::new(Provider::OpenAi);
        assistant.profile.model = "gpt-selected".into();
        assistant.input.insert_str("/mo");
        assert!(assistant.complete_command(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)));
        assert_eq!(assistant.input.lines()[0], "/model");
        assistant.input = new_input();
        let area = Rect::new(0, 0, 100, 20);
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 20)).unwrap();
        terminal
            .draw(|frame| assistant.draw(frame, area, true))
            .unwrap();
        let input = assistant.input_area(area);
        let buffer = terminal.backend().buffer();
        let footer: String = (input.x..area.right())
            .map(|x| buffer[(x, input.bottom())].symbol())
            .collect();
        assert!(footer.contains("gpt-selected"));
        assert_eq!(buffer[(input.x, input.bottom())].bg, theme::BACKGROUND);
        assert_eq!(input.x, area.x + theme::PANEL_PADDING);
        assert!(!assistant.click(area, input.x + 1, input.bottom()));
        for width in [8, 15, 30] {
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, 10)).unwrap();
            terminal
                .draw(|frame| assistant.draw(frame, frame.area(), true))
                .unwrap();
        }
    }

    #[test]
    fn transcript_uses_prompt_markers_and_compact_plain_responses() {
        use ratatui::{Terminal, backend::TestBackend};
        let mut assistant = Assistant::new();
        assistant
            .messages
            .push(Message::new("user", "Shorten my bullets.".into(), false));
        assistant.messages.push(Message::new(
            "assistant",
            "Tighter wording.\nFacts preserved.".into(),
            false,
        ));
        let mut terminal = Terminal::new(TestBackend::new(60, 6)).unwrap();
        terminal
            .draw(|frame| assistant.draw_history(frame, frame.area()))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let rendered: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
        assert!(rendered.contains("> Shorten my bullets."));
        assert!(rendered.contains("* Tighter wording."));
        assert!(rendered.contains("Facts preserved."));
        assert_eq!(buffer[(59, 0)].bg, theme::ELEVATED);
        assert_eq!(buffer[(59, 3)].bg, theme::BACKGROUND);
        assert!(!rendered.contains("◉"));
    }

    #[test]
    fn reset_removes_document_specific_conversation_and_proposal() {
        let mut assistant = Assistant::new();
        assistant
            .messages
            .push(Message::new("user", "private details".into(), false));
        assistant.input.insert_str("draft prompt");
        assistant.proposed_edit = Some(ProposedEdit {
            source_before: "old".into(),
            source_after: "new".into(),
        });
        assistant.reset();
        assert!(assistant.messages.is_empty());
        assert!(assistant.proposal().is_none());
        assert_eq!(assistant.input.lines(), &[""]);
    }

    #[test]
    fn click_moves_cursor_inside_composer_and_ignores_history() {
        let mut assistant = Assistant::new();
        assistant.input.insert_str("hello world");
        let area = Rect::new(20, 10, 70, 15);
        let input = assistant.input_area(area);
        assert!(assistant.click(area, input.x + 5, input.y + 1));
        assert_eq!(assistant.input.cursor(), (0, 2));
        assert!(!assistant.click(area, input.x + 5, input.y));
        assert!(!assistant.click(area, area.x + 2, area.y + 2));
        assert_eq!(assistant.input.cursor(), (0, 2));
    }

    #[test]
    fn submit_empty_message_does_not_launch_cli() {
        let mut assistant = Assistant::new();
        assert!(
            assistant
                .submit("source".into(), Path::new("resume.tex"))
                .is_err()
        );
        assert!(!assistant.is_running());
    }
}
