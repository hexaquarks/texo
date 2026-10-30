use ratatui::{
    Frame,
    buffer::Buffer,
    layout::{Alignment, Constraint, Layout, Margin, Position, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Padding, Paragraph, Widget},
};
use ratatui_textarea::{CursorMove, TextArea};
use texo::{auth::Provider, theme};

use super::{Assistant, Message};

const MIN_INPUT_HEIGHT: u16 = 3;
const INPUT_VERTICAL_PADDING: u16 = 1;
pub(super) const MAX_INPUT_ROWS: u16 = 6;
const MIN_HISTORY_ROWS: u16 = 2;
const HEADER_HEIGHT: u16 = 1;
const HEADER_GAP: u16 = 1;
const REVIEW_CONTROLS_HEIGHT: u16 = 2;

const CHAT_TITLE_WIDTH: u16 = 13;

const MODEL_LABEL_HEIGHT: u16 = 1;

const PROMPT_WIDTH: u16 = 2;

const THINKING_VERTICAL_PADDING: u16 = 1;

const THINKING_HEIGHT: u16 = 1 + THINKING_VERTICAL_PADDING * 2;

const THINKING_FRAME_MS: u128 = 100;

const THINKING_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

fn maximize_hint(maximized: bool, prefix: &str) -> Line<'static> {
    let shortcut = format!("{prefix} u");
    let action = if maximized { "Restore" } else { "Maximize" };

    theme::shortcuts(&[(&shortcut, action)])
}

impl Assistant {
    pub fn input_area(&self, area: Rect) -> Rect {
        self.composer_layout(area)
    }

    pub fn review_area(&self, area: Rect) -> Rect {
        if area.is_empty() || area.height < MIN_INPUT_HEIGHT + 2 {
            return Rect::default();
        }

        let composer = self.input_area(area);
        Rect::new(
            composer.x,
            composer
                .y
                .saturating_sub(REVIEW_CONTROLS_HEIGHT)
                .max(area.y),
            composer.width,
            1,
        )
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

    pub fn maximize_button(area: Rect, maximized: bool, prefix: &str) -> Rect {
        if area.is_empty() || area.height < MIN_INPUT_HEIGHT + 2 {
            return Rect::default();
        }

        let inner = area.inner(Margin::new(theme::PANEL_PADDING, 1));
        let [_, button] =
            Layout::horizontal([Constraint::Length(CHAT_TITLE_WIDTH), Constraint::Min(0)]).areas(
                Rect::new(inner.x, inner.y, inner.width, inner.height.min(1)),
            );

        let width = (maximize_hint(maximized, prefix).width() as u16).min(button.width);
        Rect::new(button.right() - width, button.y, width, button.height)
    }

    pub fn draw(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        focused: bool,
        maximized: bool,
        prefix: &str,
        reviewing: bool,
    ) {
        if let Some(login) = &self.login
            && !login.is_loading_models()
        {
            login.draw(frame, area);
            return;
        }

        if area.is_empty() || area.height < MIN_INPUT_HEIGHT + 2 {
            return;
        }

        frame.render_widget(Block::default().style(theme::text()), area);
        let inner = area.inner(Margin {
            horizontal: theme::PANEL_PADDING,
            vertical: 1,
        });
        if inner.is_empty() {
            return;
        }

        let activity_height = self.activity_height();
        let composer_height = self.composer_height(inner);

        let review_height = if reviewing { REVIEW_CONTROLS_HEIGHT } else { 0 };
        let [header, _, history, activity, _, composer] = Layout::vertical([
            Constraint::Length(HEADER_HEIGHT),
            Constraint::Length(HEADER_GAP),
            Constraint::Min(0),
            Constraint::Length(activity_height),
            Constraint::Length(review_height),
            Constraint::Length(composer_height),
        ])
        .areas(inner);
        let hint = maximize_hint(maximized, prefix);
        let [label, _] =
            Layout::horizontal([Constraint::Length(CHAT_TITLE_WIDTH), Constraint::Min(0)])
                .areas(header);
        frame.render_widget(
            Paragraph::new("texo / chat").style(theme::pane_title(focused)),
            label,
        );
        frame.render_widget(
            Paragraph::new(hint)
                .alignment(Alignment::Right)
                .style(theme::muted()),
            Self::maximize_button(area, maximized, prefix),
        );
        self.draw_history(frame, history);

        if let Some(job) = &self.job {
            let phase =
                (job.elapsed().as_millis() / THINKING_FRAME_MS) % THINKING_FRAMES.len() as u128;
            let mut label = Line::styled(
                format!("{} Texo is thinking…  ", THINKING_FRAMES[phase as usize]),
                theme::text().fg(theme::ACCENT),
            );
            label.extend(theme::shortcuts(&[("Esc", "cancel")]).spans);

            frame.render_widget(
                Paragraph::new(label),
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

        let [input, model] = composer_rows(composer);
        frame.render_widget(
            Block::default().style(Style::default().bg(theme::BUTTON)),
            input,
        );
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
    }

    pub(super) fn draw_history(&mut self, frame: &mut Frame, area: Rect) {
        if area.is_empty() {
            return;
        }

        if self.messages.is_empty() {
            let introduction = vec![
                Line::from("Ask Texo to refine the open resume."),
                Line::from("Review proposed edits before accepting."),
                Line::from("Save accepted text with the save shortcut."),
            ];
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

    fn activity_height(&self) -> u16 {
        if self.is_running() {
            THINKING_HEIGHT
        } else {
            1
        }
    }

    fn composer_layout(&self, area: Rect) -> Rect {
        let inner = area.inner(Margin::new(theme::PANEL_PADDING, 1));
        let [_, composer] = Layout::vertical([
            Constraint::Min(0),
            Constraint::Length(self.composer_height(inner)),
        ])
        .areas(inner);

        composer_rows(composer)[0]
    }

    fn composer_height(&self, inner: Rect) -> u16 {
        let width = input_block().inner(inner).width;
        let reserved = HEADER_HEIGHT
            + HEADER_GAP
            + self.activity_height()
            + MODEL_LABEL_HEIGHT
            + MIN_HISTORY_ROWS
            + INPUT_VERTICAL_PADDING * 2;
        let maximum_rows = inner
            .height
            .saturating_sub(reserved)
            .clamp(1, MAX_INPUT_ROWS);
        let rows = self.wrapped_input_rows(width, maximum_rows);

        rows + INPUT_VERTICAL_PADDING * 2 + MODEL_LABEL_HEIGHT
    }

    fn wrapped_input_rows(&self, width: u16, maximum: u16) -> u16 {
        let width = usize::from(width.max(1));
        // Long pastes can reach the height cap without copying or wrapping the draft twice.
        let minimum_rows: usize = self
            .input
            .lines()
            .iter()
            .take(usize::from(maximum))
            .map(|line| Line::raw(line.as_str()).width().div_ceil(width).max(1))
            .sum();
        if minimum_rows >= usize::from(maximum) {
            return maximum;
        }

        // The textarea owns word/grapheme wrapping and cursor mapping. Measure a text-only
        // copy so layout never changes the live cursor, selection, scroll position, or undo.
        let mut measure = TextArea::new(self.input.lines().to_vec());
        measure.set_wrap_mode(self.input.wrap_mode());
        measure.set_tab_length(self.input.tab_length());
        measure.move_cursor(CursorMove::Bottom);
        measure.move_cursor(CursorMove::End);
        let area = Rect::new(0, 0, width as u16, 1);
        (&measure).render(area, &mut Buffer::empty(area));

        u16::try_from(measure.screen_cursor().row + 1)
            .unwrap_or(u16::MAX)
            .min(maximum)
    }
}

fn composer_rows(area: Rect) -> [Rect; 2] {
    Layout::vertical([Constraint::Min(0), Constraint::Length(MODEL_LABEL_HEIGHT)]).areas(area)
}

fn input_block() -> Block<'static> {
    Block::default()
        .style(Style::default().bg(theme::BUTTON))
        .padding(Padding::new(
            PROMPT_WIDTH + 1,
            1,
            INPUT_VERTICAL_PADDING,
            INPUT_VERTICAL_PADDING,
        ))
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
