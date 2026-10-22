use std::path::Path;

use anyhow::{Result, ensure};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Margin, Position, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Padding, Paragraph},
};
use ratatui_textarea::{CursorMove, TextArea};
use texo::theme;

use crate::assistant_job::{AssistantJob, ConversationMessage};

const INPUT_HEIGHT: u16 = 5;
const SEND_WIDTH: u16 = 8;
const COMPOSER_HEIGHT: u16 = INPUT_HEIGHT + 1;
const PROMPT_WIDTH: u16 = 2;
const SCROLL_LINES: u16 = 3;

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
    scroll_from_bottom: u16,
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
            scroll_from_bottom: 0,
        }
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
        self.scroll_from_bottom = 0;
        self.job = Some(job);
        Ok(())
    }

    pub fn poll(&mut self) -> Option<Result<()>> {
        let result = self.job.as_ref()?.poll()?;
        self.job = None;
        self.scroll_from_bottom = 0;
        match result {
            Ok(response) => {
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

    /// Attach the editor's actual result so both the user and future requests see what happened.
    pub fn report_edit_result(&mut self, outcome: &str) {
        if let Some(message) = self.messages.last_mut()
            && message.role == "assistant"
            && !message.error
        {
            message.content = format!("{outcome}\n\n{}", message.content);
        }
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

    pub fn take_proposal(&mut self) -> Option<ProposedEdit> {
        self.proposed_edit.take()
    }

    pub fn scroll(&mut self, up: bool) {
        if up {
            self.scroll_from_bottom = self.scroll_from_bottom.saturating_add(SCROLL_LINES);
        } else {
            self.scroll_from_bottom = self.scroll_from_bottom.saturating_sub(SCROLL_LINES);
        }
    }

    pub fn input_area(area: Rect) -> Rect {
        composer_layout(area)[0]
    }

    pub fn click(&mut self, area: Rect, column: u16, row: u16) -> bool {
        let content = input_block(false).inner(Self::input_area(area));
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
        composer_layout(area)[2]
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect, focused: bool) {
        if area.width < SEND_WIDTH || area.height < INPUT_HEIGHT + 2 {
            return;
        }
        frame.render_widget(
            Block::default()
                .style(theme::text())
                .borders(Borders::TOP)
                .border_style(Style::default().fg(theme::DIVIDER)),
            area,
        );
        let inner = area.inner(Margin {
            horizontal: theme::PANEL_PADDING,
            vertical: 1,
        });
        let [header, _, history, _, composer] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
            Constraint::Length(COMPOSER_HEIGHT),
        ])
        .areas(inner);
        let hint = if self.is_running() {
            "Thinking…  Esc cancel"
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
            Paragraph::new("texo / chat").style(Style::default().fg(theme::ACCENT)),
            label,
        );
        frame.render_widget(
            Paragraph::new(hint)
                .alignment(Alignment::Right)
                .style(theme::muted()),
            hint_area,
        );
        self.draw_history(frame, history);

        let [input, shortcuts, send] = composer_layout_from_row(composer);
        self.input.set_block(input_block(focused));
        self.input.set_cursor_style(if focused {
            Style::default().add_modifier(Modifier::REVERSED)
        } else {
            Style::default()
        });
        frame.render_widget(&self.input, input);
        let prompt = Rect::new(input.x + 1, input.y + 2, PROMPT_WIDTH, 1);
        frame.render_widget(
            Paragraph::new("> ").style(Style::default().fg(theme::ACCENT)),
            prompt,
        );
        frame.render_widget(
            Paragraph::new("Ctrl+J focus  /  PgUp/PgDn history").style(theme::muted()),
            shortcuts,
        );
        frame.render_widget(
            Paragraph::new(if self.is_running() {
                "[cancel]"
            } else {
                "[send]"
            })
            .right_aligned()
            .style(theme::muted()),
            send,
        );
    }

    fn draw_history(&mut self, frame: &mut Frame, area: Rect) {
        if area.is_empty() {
            return;
        }
        if self.messages.is_empty() {
            let introduction = "Ask Texo to refine the open resume.\nRequested edits update the editor; Ctrl+S saves them.";
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
    input.set_style(theme::text());
    input.set_cursor_line_style(Style::default());
    input.set_placeholder_text("Ask Texo...");
    input.set_placeholder_style(theme::muted());
    input
}

fn composer_layout(area: Rect) -> [Rect; 3] {
    let inner = area.inner(Margin {
        horizontal: theme::PANEL_PADDING,
        vertical: 1,
    });
    let [_, composer] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(COMPOSER_HEIGHT)]).areas(inner);
    composer_layout_from_row(composer)
}

fn composer_layout_from_row(area: Rect) -> [Rect; 3] {
    let [input, footer] =
        Layout::vertical([Constraint::Length(INPUT_HEIGHT), Constraint::Length(1)]).areas(area);
    let [shortcuts, send] =
        Layout::horizontal([Constraint::Min(1), Constraint::Length(SEND_WIDTH)]).areas(footer);
    [input, shortcuts, send]
}

fn input_block(focused: bool) -> Block<'static> {
    Block::default()
        .borders(Borders::TOP)
        .padding(Padding::new(PROMPT_WIDTH + 1, 1, 1, 1))
        .border_style(Style::default().fg(if focused {
            theme::MUTED
        } else {
            theme::DIVIDER
        }))
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
        let input = Assistant::input_area(area);
        assert!(assistant.click(area, input.x + 5, input.y + 2));
        assert_eq!(assistant.input.cursor(), (0, 2));
        assert!(!assistant.click(area, input.x + 5, input.y + 1));
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
