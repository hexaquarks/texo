use std::path::Path;

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    widgets::{Block, Borders, Clear, Padding, Paragraph, Wrap},
};
use ratatui_textarea::TextArea;
use texo::theme;

const DIALOG_WIDTH: u16 = 72;
const DIALOG_HEIGHT: u16 = 12;

pub struct SettingsDialog {
    pub input: TextArea<'static>,
    pub error: String,
}

impl SettingsDialog {
    pub fn new(root: &Path) -> Self {
        let mut input = TextArea::new(vec![crate::workspace::display_path(root)]);
        input.set_style(theme::text().bg(theme::ELEVATED));
        input.set_cursor_line_style(theme::text().bg(theme::ELEVATED));
        input.set_selection_style(theme::text().bg(theme::SELECTION));
        input.select_all();
        Self {
            input,
            error: String::new(),
        }
    }

    pub fn value(&self) -> String {
        self.input.lines().join("\n")
    }

    pub fn draw(&self, frame: &mut Frame) {
        let screen = frame.area();
        let width = DIALOG_WIDTH.min(screen.width);
        let height = DIALOG_HEIGHT.min(screen.height);
        let area = Rect::new(
            screen.x + (screen.width - width) / 2,
            screen.y + (screen.height - height) / 2,
            width,
            height,
        );
        let panel = Block::default()
            .title(" Settings ")
            .borders(Borders::ALL)
            .border_style(theme::pane_title(true))
            .style(theme::text())
            .padding(Padding::uniform(1));
        let content = panel.inner(area);
        frame.render_widget(Clear, area);
        frame.render_widget(panel, area);
        let [label, input, warning, error, hint] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(2),
            Constraint::Length(2),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .areas(content);
        frame.render_widget(Paragraph::new("Resume folder"), label);
        frame.render_widget(&self.input, input);
        frame.render_widget(
            Paragraph::new("Changing folders discards unsaved text, proposed edits, and chat.")
                .style(theme::muted())
                .wrap(Wrap { trim: false }),
            warning,
        );
        frame.render_widget(
            Paragraph::new(self.error.as_str())
                .style(theme::text().fg(theme::ERROR))
                .wrap(Wrap { trim: false }),
            error,
        );
        frame.render_widget(
            Paragraph::new("Enter apply · Esc cancel").style(theme::muted()),
            hint,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn dialog_is_centered_and_pasting_replaces_the_selected_path() -> anyhow::Result<()> {
        let mut dialog = SettingsDialog::new(Path::new("/old/resumes"));
        dialog.input.insert_str("/new/resumes");
        assert_eq!(dialog.value(), "/new/resumes");
        let mut terminal = Terminal::new(TestBackend::new(100, 30))?;
        terminal.draw(|frame| dialog.draw(frame))?;
        let buffer = terminal.backend().buffer();
        let left = (100 - DIALOG_WIDTH) / 2;
        let top = (30 - DIALOG_HEIGHT) / 2;
        assert_eq!(buffer[(left, top)].symbol(), "┌");
        assert_eq!(
            buffer[(left + DIALOG_WIDTH - 1, top + DIALOG_HEIGHT - 1)].symbol(),
            "┘"
        );
        let text: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
        assert!(text.contains("/new/resumes"));
        assert!(text.contains("Enter apply"));

        for (width, height) in [(1, 1), (20, 5), (40, 12)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height))?;
            terminal.draw(|frame| dialog.draw(frame))?;
        }
        Ok(())
    }
}
