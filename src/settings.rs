use std::path::{Path, PathBuf};

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    widgets::{Block, Borders, Clear, Padding, Paragraph, Wrap},
};
use ratatui_textarea::TextArea;
use texo::theme;

const DIALOG_WIDTH: u16 = 72;
const DIALOG_HEIGHT: u16 = 17;

pub struct SettingsDialog {
    pub input: TextArea<'static>,
    pub error: String,
    pub notice: String,
    pub config_path: PathBuf,
    pub prefix_label: String,
}

impl SettingsDialog {
    pub fn new(root: &Path, config_path: &Path, prefix_label: String) -> Self {
        let mut input = TextArea::new(vec![crate::workspace::display_path(root)]);
        input.set_style(theme::text().bg(theme::ELEVATED));
        input.set_cursor_line_style(theme::text().bg(theme::ELEVATED));
        input.set_selection_style(theme::text().bg(theme::SELECTION));
        input.select_all();
        Self {
            input,
            error: String::new(),
            notice: String::new(),
            config_path: config_path.to_path_buf(),
            prefix_label,
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
        let [label, input, warning, config, prefix, actions, error, hint] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(2),
            Constraint::Length(2),
            Constraint::Length(2),
            Constraint::Length(1),
            Constraint::Length(1),
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
            Paragraph::new(format!(
                "Config: {}",
                crate::workspace::display_path(&self.config_path)
            ))
            .style(theme::muted())
            .wrap(Wrap { trim: false }),
            config,
        );
        frame.render_widget(
            Paragraph::new(format!("Prefix: {}", self.prefix_label)),
            prefix,
        );
        frame.render_widget(
            Paragraph::new("Ctrl+O Open config · Ctrl+R Reload").style(theme::pane_title(true)),
            actions,
        );
        let (message, color) = if self.error.is_empty() {
            (self.notice.as_str(), theme::ACCENT)
        } else {
            (self.error.as_str(), theme::ERROR)
        };
        frame.render_widget(
            Paragraph::new(message)
                .style(theme::text().fg(color))
                .wrap(Wrap { trim: false }),
            error,
        );
        frame.render_widget(
            Paragraph::new("Enter apply · Esc close").style(theme::muted()),
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
        let mut dialog = SettingsDialog::new(
            Path::new("/old/resumes"),
            Path::new("/config/texo/config.toml"),
            "ctrl-x".into(),
        );
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
        assert!(text.contains("/config/texo/config.toml"));
        assert!(text.contains("Prefix: ctrl-x"));
        assert!(text.contains("Ctrl+O Open config"));

        for (width, height) in [(1, 1), (20, 5), (40, 12)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height))?;
            terminal.draw(|frame| dialog.draw(frame))?;
        }
        Ok(())
    }
}
