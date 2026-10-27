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
const MENU_FRAME_HEIGHT: u16 = 6;

#[derive(Clone, Copy)]
pub enum Action {
    Settings,
    NewResume,
    ImportResume,
    RenameResume,
    LabelResume,
    DuplicateResume,
    FocusResumes,
    FocusFiles,
    ReloadSource,
    RecoverSource,
    WorkspaceWarnings,
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

const ACTIONS: [(Action, &str, Option<&str>); 23] = [
    (Action::NewResume, "New resume", None),
    (
        Action::ImportResume,
        "Import resume (copy source/project)",
        None,
    ),
    (Action::RenameResume, "Rename resume", None),
    (Action::LabelResume, "Set resume label", None),
    (Action::DuplicateResume, "Duplicate resume", None),
    (Action::FocusResumes, "Focus resumes", None),
    (Action::FocusFiles, "Focus files", None),
    (
        Action::ReloadSource,
        "Reload source from disk (confirm)",
        None,
    ),
    (
        Action::RecoverSource,
        "Restore missing source (confirm)",
        None,
    ),
    (Action::WorkspaceWarnings, "Show workspace warnings", None),
    (Action::Settings, "Settings", None),
    (Action::Build, "Build PDF", None),
    (Action::Export, "Export PDF", None),
    (Action::Save, "Save TeX", None),
    (Action::ToggleProjects, "Show / hide projects", None),
    (Action::PreviousPage, "Previous PDF page", None),
    (Action::NextPage, "Next PDF page", None),
    (Action::ScrollUp, "Scroll PDF up", None),
    (Action::ScrollDown, "Scroll PDF down", None),
    (Action::FocusAssistant, "Focus assistant / editor", None),
    (Action::ReviewEdit, "Review suggested TeX edit", None),
    (Action::ClearChat, "Clear conversation", None),
    (Action::Quit, "Quit", Some("Ctrl+Q")),
];

const RESUME_ACTIONS: [(Action, &str, Option<&str>); 3] = [
    (Action::RenameResume, "Rename resume", None),
    (Action::LabelResume, "Set or change label", None),
    (Action::DuplicateResume, "Duplicate resume", None),
];

pub struct ActionMenu {
    state: ListState,
    entries: &'static [(Action, &'static str, Option<&'static str>)],
    title: String,
}

impl ActionMenu {
    pub fn open() -> Self {
        Self {
            state: ListState::default().with_selected(Some(0)),
            entries: &ACTIONS,
            title: "Actions".into(),
        }
    }

    pub fn for_resume(name: &str) -> Self {
        Self {
            state: ListState::default().with_selected(Some(0)),
            entries: &RESUME_ACTIONS,
            title: format!("Manage {name}"),
        }
    }

    pub fn move_selection(&mut self, down: bool) {
        let current = self.state.selected().unwrap_or(0);
        let next = if down {
            (current + 1).min(self.entries.len() - 1)
        } else {
            current.saturating_sub(1)
        };
        self.state.select(Some(next));
    }

    pub fn selected(&self) -> Action {
        self.entries[self.state.selected().unwrap_or(0)].0
    }

    fn layout(&self, screen: Rect) -> [Rect; 3] {
        let area = centered(
            screen,
            DIALOG_WIDTH,
            self.entries.len() as u16 + MENU_FRAME_HEIGHT,
        );
        let inner = dialog_block("").padding(Padding::uniform(1)).inner(area);
        let [choices, _, help] = Layout::vertical([
            Constraint::Min(0),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas(inner);
        [area, choices, help]
    }

    pub fn click(&mut self, screen: Rect, column: u16, row: u16) -> Option<Action> {
        let [_, choices, _] = self.layout(screen);
        if !choices.contains((column, row).into()) {
            return None;
        }
        let index = self.state.offset() + usize::from(row - choices.y);
        if index >= self.entries.len() {
            return None;
        }
        self.state.select(Some(index));
        Some(self.selected())
    }

    pub fn draw(&mut self, frame: &mut Frame, prefix: &str) {
        let [area, choices, help] = self.layout(frame.area());
        frame.render_widget(Clear, area);
        let title = Line::styled(format!(" {} ", self.title), theme::dialog_title());
        frame.render_widget(dialog_block(title), area);
        let selected = self.state.selected();
        let rows =
            self.entries
                .iter()
                .enumerate()
                .map(|(index, (action, description, shortcut))| {
                    let prefix_action = match action {
                        Action::Settings => Some('s'),
                        Action::Build => Some('b'),
                        Action::Export => Some('e'),
                        Action::Save => Some('w'),
                        Action::ToggleProjects => Some('t'),
                        _ => None,
                    };
                    let configured_shortcut = prefix_action.map(|key| format!("{prefix} {key}"));
                    let shortcut = configured_shortcut.as_deref().or(*shortcut);
                    let line = match shortcut {
                        Some(key) => theme::shortcuts(&[(key, description)]),
                        None => Line::from(format!(" {description}")),
                    };
                    let style = if selected == Some(index) {
                        theme::dialog().fg(theme::ACCENT).bg(theme::SELECTION)
                    } else {
                        theme::dialog()
                    };

                    // Apply selection before the spans, so it does not overwrite keycap shading.
                    ListItem::new(line).style(style)
                });

        let list = List::new(rows).style(theme::dialog());
        frame.render_stateful_widget(list, choices, &mut self.state);
        frame.render_widget(
            Paragraph::new(theme::shortcuts(&[
                ("j/k", "move"),
                ("Enter", "select"),
                ("Esc", "close"),
            ]))
            .style(theme::dialog()),
            help,
        );
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
        input.set_style(theme::dialog());
        input.set_cursor_line_style(Style::default());
        input.set_cursor_style(Style::default().bg(theme::ACCENT).fg(theme::BACKGROUND));
        Self {
            input,
            message: String::new(),
            overwrite_path: None,
        }
    }

    pub fn edited(&mut self) {
        self.overwrite_path = None;
        self.message.clear();
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
            self.message = "File exists.".into();
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
        frame.render_widget(panel, area);
        let [label, input, message] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(2),
            Constraint::Min(1),
        ])
        .areas(content);
        frame.render_widget(Paragraph::new("Destination"), label);
        frame.render_widget(&self.input, input);

        let action = if self.overwrite_path.is_some() {
            "replace"
        } else {
            "export"
        };
        let mut hint = Line::from(self.message.clone());
        if !self.message.is_empty() {
            hint.push_span(" ");
        }
        hint.extend(theme::shortcuts(&[("Enter", action), ("Esc", "cancel")]).spans);
        frame.render_widget(Paragraph::new(hint).style(theme::muted()), message);
    }
}

fn dialog_block<'a>(title: impl Into<Line<'a>>) -> Block<'a> {
    Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::dialog_title())
        .style(theme::dialog())
}

#[cfg(test)]
fn menu_area(screen: Rect) -> Rect {
    centered(
        screen,
        DIALOG_WIDTH,
        ACTIONS.len() as u16 + MENU_FRAME_HEIGHT,
    )
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
    fn selected_action_keeps_keycaps_distinct_from_its_description() -> Result<()> {
        use ratatui::{Terminal, backend::TestBackend};

        let mut menu = ActionMenu::open();
        let settings = ACTIONS
            .iter()
            .position(|(action, _, _)| matches!(action, Action::Settings))
            .unwrap();
        menu.state.select(Some(settings));
        let mut terminal = Terminal::new(TestBackend::new(80, 30))?;
        terminal.draw(|frame| menu.draw(frame, "ctrl-x"))?;
        let area = menu_area(terminal.backend().buffer().area);
        let row = area.y + 2 + settings as u16;
        let column = area.x + 2;
        let buffer = terminal.backend().buffer();

        assert_eq!(buffer[(column + 1, row)].symbol(), "c");
        assert_eq!(buffer[(column + 1, row)].bg, theme::KEYCAP);
        let description_column = column + "ctrl-x s".len() as u16 + 3;
        assert_eq!(buffer[(description_column, row)].symbol(), "S");
        assert_eq!(buffer[(description_column, row)].bg, theme::SELECTION);
        assert_eq!(
            buffer[(area.x + 1, area.y + 1)].bg,
            theme::DIALOG_BACKGROUND
        );
        assert_eq!(buffer[(area.x + 2, area.y)].fg, theme::ACCENT);
        let [_, _, help] = menu.layout(buffer.area);
        assert!(menu.click(buffer.area, help.x, help.y).is_none());
        assert!(matches!(
            menu.click(buffer.area, column, row),
            Some(Action::Settings)
        ));
        Ok(())
    }

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
