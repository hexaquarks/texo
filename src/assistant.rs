use std::path::Path;

use anyhow::{Result, ensure};
use chrono::Local;
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Margin, Position, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
};
use ratatui_textarea::{CursorMove, TextArea};
use texo::theme;

use crate::assistant_job::{AssistantJob, ConversationMessage};

const INPUT_HEIGHT: u16 = 3;
const SEND_WIDTH: u16 = 10;
const SCROLL_LINES: u16 = 3;

#[derive(Clone)]
pub struct ProposedEdit {
    pub source_before: String,
    pub source_after: String,
}

struct Message {
    role: &'static str,
    content: String,
    time: String,
    error: bool,
}

impl Message {
    fn new(role: &'static str, content: String, error: bool) -> Self {
        Self {
            role,
            content,
            time: Local::now().format("%I:%M %p").to_string(),
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
        let content = Self::input_area(area).inner(Margin {
            horizontal: 1,
            vertical: 1,
        });
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
            horizontal: 1,
            vertical: 1,
        });
        let [header, history, _, composer] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
            Constraint::Length(INPUT_HEIGHT),
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
            Layout::horizontal([Constraint::Length(11), Constraint::Min(0)]).areas(header);
        frame.render_widget(
            Paragraph::new("ASSISTANT").style(Style::default().fg(theme::ACCENT)),
            label,
        );
        frame.render_widget(
            Paragraph::new(hint)
                .alignment(Alignment::Right)
                .style(theme::muted()),
            hint_area,
        );
        self.draw_history(frame, history);

        let [input, _, send] = composer_layout_from_row(composer);
        self.input.set_block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(if focused {
                    theme::ACCENT
                } else {
                    theme::DIVIDER
                })),
        );
        self.input.set_cursor_style(if focused {
            Style::default().add_modifier(Modifier::REVERSED)
        } else {
            Style::default()
        });
        frame.render_widget(&self.input, input);
        frame.render_widget(
            Paragraph::new(if self.is_running() { "Cancel" } else { "Send" })
                .alignment(Alignment::Center)
                .style(Style::default().fg(theme::ACCENT).bg(theme::ELEVATED))
                .block(
                    Block::bordered()
                        .border_type(BorderType::Rounded)
                        .border_style(Style::default().fg(theme::ACCENT)),
                ),
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
        // Keep spacing between cards, without scrolling a visible header away for an empty final row.
        lines.pop();
        let maximum_scroll = u16::try_from(lines.len())
            .unwrap_or(u16::MAX)
            .saturating_sub(area.height);
        self.scroll_from_bottom = self.scroll_from_bottom.min(maximum_scroll);
        let offset = maximum_scroll.saturating_sub(self.scroll_from_bottom);
        frame.render_widget(Paragraph::new(lines).scroll((offset, 0)), area);
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
        horizontal: 1,
        vertical: 1,
    });
    let [_, composer] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(INPUT_HEIGHT)]).areas(inner);
    composer_layout_from_row(composer)
}

fn composer_layout_from_row(area: Rect) -> [Rect; 3] {
    Layout::horizontal([
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(SEND_WIDTH),
    ])
    .areas(area)
}

fn message_lines(message: &Message, width: u16) -> Vec<Line<'static>> {
    let card_style = Style::default().bg(theme::ELEVATED);
    let name = if message.role == "user" {
        "You"
    } else {
        "Texo"
    };
    let avatar = if message.role == "user" {
        "◉ U"
    } else {
        "◉ T"
    };
    let title = format!(" {avatar}  {name}");
    let spacing = usize::from(width).saturating_sub(title.chars().count() + message.time.len() + 1);
    let header = Line::from(vec![
        Span::styled(title, Style::default().fg(theme::ACCENT)),
        Span::raw(" ".repeat(spacing)),
        Span::styled(format!("{} ", message.time), theme::muted()),
    ])
    .style(card_style);
    let mut lines = vec![header];
    let content_width = usize::from(width.saturating_sub(7)).max(1);
    let content_style = card_style.fg(if message.error {
        theme::ERROR
    } else {
        theme::TEXT
    });
    for paragraph in message.content.lines() {
        for line in textwrap::wrap(paragraph, content_width) {
            lines.push(Line::from(format!("      {line} ")).style(content_style));
        }
    }
    lines.push(Line::from(" ").style(card_style));
    for line in &mut lines {
        let padding = usize::from(width).saturating_sub(line.width());
        line.spans.push(Span::raw(" ".repeat(padding)));
    }
    lines.push(Line::default());
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_conversation_keeps_headers_visible_and_fills_card_backgrounds() {
        use ratatui::{Terminal, backend::TestBackend};

        let mut assistant = Assistant::new();
        assistant
            .messages
            .push(Message::new("user", "Shorten my bullets.".into(), false));
        assistant.messages.push(Message::new(
            "assistant",
            "I can tighten the wording\nwhile preserving your facts.".into(),
            false,
        ));
        let mut terminal = Terminal::new(TestBackend::new(60, 8)).unwrap();
        terminal
            .draw(|frame| assistant.draw_history(frame, frame.area()))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let rendered: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
        assert!(rendered.contains("You"));
        assert!(rendered.contains("Texo"));
        for row in [0, 1, 2, 4, 5, 6, 7] {
            for column in 0..60 {
                assert_eq!(buffer[(column, row)].bg, theme::ELEVATED);
            }
        }
        assert_ne!(buffer[(59, 3)].bg, theme::ELEVATED);
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
        assert!(assistant.click(area, input.x + 4, input.y + 1));
        assert_eq!(assistant.input.cursor(), (0, 3));
        assert!(!assistant.click(area, area.x + 2, area.y + 2));
        assert_eq!(assistant.input.cursor(), (0, 3));
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
