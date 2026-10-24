use ratatui::{
    Frame,
    layout::{Margin, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Paragraph, Wrap},
};
use texo::{auth::Provider, theme};

use super::{LoginDialog, Step};

pub(super) const MENU_HEADER_ROWS: u16 = 2;
const MASKED_KEY_CHARACTERS: usize = 32;

impl LoginDialog {
    pub fn draw(&self, frame: &mut Frame, area: Rect) {
        frame.render_widget(Block::default().style(theme::text()), area);
        let content = dialog_content(area);
        let title = match self.step {
            Step::Provider => "Connect an assistant",
            Step::Method => "Select a sign-in method",
            Step::Key => "Enter your API key",
            Step::Model | Step::LoadingModels => "Choose a model",
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
                        "•".repeat(self.key.len().min(MASKED_KEY_CHARACTERS))
                    }
                }
                Step::LoadingModels => String::new(),
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

pub(super) fn model_visible_rows(content: Rect) -> usize {
    const FOOTER_ROWS: u16 = 4;
    usize::from(
        content
            .height
            .saturating_sub(MENU_HEADER_ROWS + FOOTER_ROWS),
    )
    .max(1)
}

pub(super) fn dialog_content(area: Rect) -> Rect {
    area.inner(Margin {
        horizontal: theme::PANEL_PADDING,
        vertical: 1,
    })
}
