use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::Style,
    text::Line,
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState, Padding, Paragraph},
};
use ratatui_textarea::TextArea;
use texo::theme;

const DIALOG_WIDTH: u16 = 64;
const EXPORT_HEIGHT: u16 = 8;

#[derive(Clone, Copy)]
pub enum Action {
    Settings,
    Build,
    Export,
    Save,
    ToggleProjects,
    PreviousPage,
    NextPage,
    ScrollUp,
    ScrollDown,
    FocusAssistant,
    ReviewEdit,
    ClearChat,
    Quit,
}

const ACTIONS: [(Action, &str); 13] = [
    (Action::Settings, "Settings                          ?"),
    (Action::Build, "Build PDF                         Ctrl+B"),
    (Action::Export, "Export PDF                        Ctrl+E"),
    (Action::Save, "Save TeX                          Ctrl+S"),
    (
        Action::ToggleProjects,
        "Show / hide projects              Ctrl+T",
    ),
    (
        Action::PreviousPage,
        "Previous PDF page                 Alt+Left",
    ),
    (
        Action::NextPage,
        "Next PDF page                     Alt+Right",
    ),
    (Action::ScrollUp, "Scroll PDF up                     Alt+Up"),
    (
        Action::ScrollDown,
        "Scroll PDF down                   Alt+Down",
    ),
    (
        Action::FocusAssistant,
        "Focus assistant / editor          Ctrl+J",
    ),
    (Action::ReviewEdit, "Review suggested TeX edit"),
    (Action::ClearChat, "Clear conversation"),
    (Action::Quit, "Quit                              Ctrl+Q"),
];

#[derive(Default)]
pub struct ActionMenu {
    state: ListState,
}

impl ActionMenu {
    pub fn open() -> Self {
        Self {
            state: ListState::default().with_selected(Some(0)),
        }
    }

    pub fn move_selection(&mut self, down: bool) {
        let current = self.state.selected().unwrap_or(0);
        let next = if down {
            (current + 1).min(ACTIONS.len() - 1)
        } else {
            current.saturating_sub(1)
        };
        self.state.select(Some(next));
    }

    pub fn selected(&self) -> Action {
        ACTIONS[self.state.selected().unwrap_or(0)].0
    }

    pub fn click(&mut self, screen: Rect, column: u16, row: u16) -> Option<Action> {
        let area = menu_area(screen);
        if column <= area.x || column >= area.right().saturating_sub(1) {
            return None;
        }
        let index = usize::from(row.checked_sub(area.y + 2)?);
        if index >= ACTIONS.len() {
            return None;
        }
        self.state.select(Some(index));
        Some(self.selected())
    }

    pub fn draw(&mut self, frame: &mut Frame) {
        let area = menu_area(frame.area());
        frame.render_widget(Clear, area);
        let rows = ACTIONS
            .iter()
            .map(|(_, label)| ListItem::new(format!(" {label}")));
        let list = List::new(rows)
            .block(
                dialog_block(" Actions · Enter to select · Esc to close ")
                    .padding(Padding::uniform(1)),
            )
            .style(theme::text())
            .highlight_style(Style::default().fg(theme::ACCENT).bg(theme::SELECTION));
        frame.render_stateful_widget(list, area, &mut self.state);
    }
}

pub struct ExportDialog {
    pub input: TextArea<'static>,
    pub message: String,
    overwrite_path: Option<PathBuf>,
}

impl ExportDialog {
    pub fn new(document: &Path) -> Self {
        let stem = document.file_stem().unwrap_or_default().to_string_lossy();
        let destination = document.with_file_name(format!("{stem}-export.pdf"));
        let mut input = TextArea::new(vec![destination.display().to_string()]);
        input.set_style(theme::text());
        input.set_cursor_line_style(Style::default());
        input.set_cursor_style(Style::default().bg(theme::ACCENT).fg(theme::BACKGROUND));
        Self {
            input,
            message: "Enter to export · Esc to cancel".into(),
            overwrite_path: None,
        }
    }

    pub fn edited(&mut self) {
        self.overwrite_path = None;
        self.message = "Enter to export · Esc to cancel".into();
    }

    pub fn export(&mut self, source: &Path, document: &Path) -> Result<Option<PathBuf>> {
        let path = self.input.lines().join("");
        ensure!(!path.trim().is_empty(), "Enter a destination path");
        let destination = if let Some(relative) = path.strip_prefix("~/") {
            PathBuf::from(std::env::var_os("HOME").context("Could not locate home")?).join(relative)
        } else {
            document
                .parent()
                .context("Document has no directory")?
                .join(path)
        };
        ensure!(
            destination
                .extension()
                .is_some_and(|extension| extension == "pdf"),
            "Use a .pdf destination"
        );
        let parent = destination
            .parent()
            .context("Destination has no directory")?
            .canonicalize()
            .context("The destination directory does not exist")?;
        let destination = parent.join(destination.file_name().context("Enter a filename")?);
        ensure!(
            destination != source.canonicalize()?,
            "Choose a destination different from the build output"
        );
        if destination.exists() && self.overwrite_path.as_ref() != Some(&destination) {
            self.overwrite_path = Some(destination);
            self.message = "File exists. Enter again to replace it, or edit the path.".into();
            return Ok(None);
        }

        let temporary = tempfile::NamedTempFile::new_in(parent)?;
        fs::copy(source, temporary.path()).context("Could not copy the built PDF")?;
        temporary
            .persist(&destination)
            .context("Could not write the exported PDF")?;
        Ok(Some(destination))
    }

    pub fn draw(&mut self, frame: &mut Frame) {
        let area = centered(frame.area(), DIALOG_WIDTH, EXPORT_HEIGHT);
        frame.render_widget(Clear, area);
        let panel = dialog_block(" Export PDF ").padding(Padding::uniform(1));
        let content = panel.inner(area);
        frame.render_widget(panel.style(theme::text()), area);
        let [label, input, message] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(2),
            Constraint::Min(1),
        ])
        .areas(content);
        frame.render_widget(Paragraph::new("Destination"), label);
        frame.render_widget(&self.input, input);
        frame.render_widget(
            Paragraph::new(Line::styled(&self.message, theme::muted())),
            message,
        );
    }
}

fn dialog_block(title: &str) -> Block<'_> {
    Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme::DIVIDER))
}

fn menu_area(screen: Rect) -> Rect {
    centered(screen, DIALOG_WIDTH, ACTIONS.len() as u16 + 4)
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_requires_confirming_replacement_and_preserves_build_output() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("main.pdf");
        let document = directory.path().join("main.tex");
        let destination = directory.path().join("main-export.pdf");
        fs::write(&source, "new pdf")?;
        fs::write(&destination, "old pdf")?;
        let mut dialog = ExportDialog::new(&document);
        assert!(dialog.export(&source, &document)?.is_none());
        assert_eq!(fs::read_to_string(&destination)?, "old pdf");
        assert_eq!(
            dialog.export(&source, &document)?,
            Some(destination.canonicalize()?)
        );
        assert_eq!(fs::read_to_string(destination)?, "new pdf");
        assert_eq!(fs::read_to_string(source)?, "new pdf");
        Ok(())
    }
}
