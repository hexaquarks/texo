use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::dialog_ui::{ButtonStyle, DialogAction, FormLayout, button, field};
use anyhow::{Context, Result, ensure};
use ratatui::{
    Frame,
    crossterm::event::{Event, KeyCode, MouseButton, MouseEventKind},
    layout::{Constraint, Layout, Rect},
    style::Style,
    text::Line,
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState, Padding, Paragraph},
};
use ratatui_textarea::TextArea;
use texo::theme;

const DIALOG_WIDTH: u16 = 64;
const RESUME_MENU_WIDTH: u16 = 76;
const RESUME_MENU_HEIGHT: u16 = 15;
const MENU_FRAME_HEIGHT: u16 = 6;

#[derive(Clone, Copy)]
pub enum Action {
    NewResume,
    ImportResume,
    RenameResume,
    LabelResume,
    DuplicateResume,
    ReloadSource,
    RecoverSource,
    OpenRevision,
    RestoreRevision,
    RenameRevision,
}

const RESUME_ACTIONS: [(Action, &str); 4] = [
    (Action::RenameResume, "Rename"),
    (Action::LabelResume, "Change label"),
    (Action::DuplicateResume, "Duplicate"),
    (Action::ReloadSource, "Reload from disk"),
];

pub struct ActionMenu {
    state: ListState,
    entries: &'static [(Action, &'static str)],
    title: String,
    pub context: Option<String>,
}

const REVISION_ACTIONS: [(Action, &str); 3] = [
    (Action::OpenRevision, "Open"),
    (Action::RestoreRevision, "Restore"),
    (Action::RenameRevision, "Rename"),
];

impl ActionMenu {
    pub fn for_revision(number: usize) -> Self {
        Self {
            state: ListState::default().with_selected(Some(0)),
            entries: &REVISION_ACTIONS,
            title: format!("revision-{number}"),
            context: None,
        }
    }

    pub fn for_resume(name: &str) -> Self {
        Self {
            state: ListState::default().with_selected(Some(0)),
            entries: &RESUME_ACTIONS,
            title: "Manage resume".into(),
            context: Some(name.into()),
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
        if self.context.is_some() {
            let area = centered(screen, RESUME_MENU_WIDTH, RESUME_MENU_HEIGHT);
            let inner = dialog_block("").padding(Padding::uniform(1)).inner(area);
            let [_, choices, _, help] = Layout::vertical([
                Constraint::Length(3),
                Constraint::Min(0),
                Constraint::Length(3),
                Constraint::Length(1),
            ])
            .areas(inner);
            return [area, choices, help];
        }

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

    pub fn draw(&mut self, frame: &mut Frame) {
        let [area, choices, help] = self.layout(frame.area());
        frame.render_widget(Clear, area);
        let title = Line::styled(format!(" {} ", self.title), theme::dialog_title());
        frame.render_widget(dialog_block(title), area);
        if let Some(context) = &self.context {
            let inner = dialog_block("").padding(Padding::uniform(1)).inner(area);
            frame.render_widget(
                Paragraph::new(context.as_str()).style(theme::muted()),
                Rect { height: 2, ..inner },
            );
            if matches!(self.selected(), Action::ReloadSource) {
                frame.render_widget(
                    Paragraph::new("Replace the working copy with the source currently on disk.")
                        .style(theme::muted())
                        .wrap(ratatui::widgets::Wrap { trim: false }),
                    Rect::new(choices.x, choices.bottom(), choices.width, 2),
                );
            }
        }
        let selected = self.state.selected();
        let rows = self
            .entries
            .iter()
            .enumerate()
            .map(|(index, (_, description))| {
                let line = Line::from(format!(" {description}"));
                let style = if selected == Some(index) {
                    theme::dialog().fg(theme::ACCENT).bg(theme::SELECTION)
                } else {
                    theme::dialog()
                };

                ListItem::new(line).style(style)
            });

        let list = List::new(rows)
            .style(theme::dialog())
            .highlight_symbol("› ");
        frame.render_stateful_widget(list, choices, &mut self.state);
        if self.context.is_some() {
            frame.render_widget(
                Block::default()
                    .borders(Borders::TOP)
                    .border_style(theme::muted()),
                Rect::new(help.x, help.y.saturating_sub(1), help.width, 1),
            );
        }

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

#[derive(Clone, Copy, PartialEq, Eq)]
enum ExportControl {
    Folder,
    Filename,
    Cancel,
    Submit,
}

pub struct ExportDialog {
    pub input: TextArea<'static>,
    pub folder: TextArea<'static>,
    pub context: String,
    focus: ExportControl,
    pub message: String,
    overwrite_path: Option<PathBuf>,
}

impl ExportDialog {
    pub fn new(document: &Path) -> Self {
        let stem = document.file_stem().unwrap_or_default().to_string_lossy();
        let mut input = TextArea::new(vec![format!("{stem}-export.pdf")]);
        input.set_style(theme::dialog());
        input.set_cursor_line_style(Style::default());
        input.set_cursor_style(Style::default().bg(theme::ACCENT).fg(theme::BACKGROUND));
        input.select_all();

        let mut folder = TextArea::new(vec![crate::workspace::display_path(
            document.parent().unwrap_or(Path::new(".")),
        )]);
        folder.select_all();

        Self {
            input,
            folder,
            context: document
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            focus: ExportControl::Folder,
            message: String::new(),
            overwrite_path: None,
        }
    }

    pub fn edited(&mut self) {
        self.overwrite_path = None;
        self.message.clear();
    }

    pub fn export(&mut self, source: &Path, document: &Path) -> Result<Option<PathBuf>> {
        let filename = self.input.lines().join("");
        ensure!(!filename.trim().is_empty(), "Enter a filename");
        ensure!(
            Path::new(&filename)
                .file_name()
                .is_some_and(|name| name == filename.as_str()),
            "Filename must not contain a folder path"
        );
        let path = self.folder.lines().join("");
        ensure!(!path.trim().is_empty(), "Enter a destination folder");
        let destination = if let Some(relative) = path.strip_prefix("~/") {
            PathBuf::from(std::env::var_os("HOME").context("Could not locate home")?).join(relative)
        } else {
            document
                .parent()
                .context("Document has no directory")?
                .join(path)
        };
        let destination = destination.join(filename);
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

    fn action_label(&self) -> &str {
        if self.overwrite_path.is_some() {
            "Replace PDF"
        } else {
            "Export PDF"
        }
    }

    fn layout(&self, screen: Rect) -> FormLayout {
        let row = if self.focus == ExportControl::Filename {
            8
        } else {
            3
        };
        FormLayout::new(screen, 14, row, self.action_label())
    }

    pub fn handle_event(&mut self, event: Event, screen: Rect) -> Option<DialogAction> {
        let before = (self.folder.lines().join(""), self.input.lines().join(""));
        match event {
            Event::Key(key) if key.code == KeyCode::Esc => return Some(DialogAction::Cancel),
            Event::Key(key) if matches!(key.code, KeyCode::Tab | KeyCode::BackTab) => {
                let controls = [
                    ExportControl::Folder,
                    ExportControl::Filename,
                    ExportControl::Cancel,
                    ExportControl::Submit,
                ];
                let current = controls
                    .iter()
                    .position(|control| *control == self.focus)
                    .unwrap();
                let offset = if key.code == KeyCode::BackTab {
                    controls.len() - 1
                } else {
                    1
                };
                self.focus = controls[(current + offset) % controls.len()];
            }
            Event::Key(key) if key.code == KeyCode::Enter => {
                return Some(if self.focus == ExportControl::Cancel {
                    DialogAction::Cancel
                } else {
                    DialogAction::Submit
                });
            }
            Event::Key(key)
                if matches!(self.focus, ExportControl::Cancel | ExportControl::Submit) =>
            {
                match key.code {
                    KeyCode::Char('h') | KeyCode::Left => self.focus = ExportControl::Cancel,
                    KeyCode::Char('l') | KeyCode::Right => self.focus = ExportControl::Submit,
                    KeyCode::Char('k') | KeyCode::Up => self.focus = ExportControl::Filename,
                    KeyCode::Char(' ') => {
                        return Some(if self.focus == ExportControl::Cancel {
                            DialogAction::Cancel
                        } else {
                            DialogAction::Submit
                        });
                    }
                    _ => {}
                }
            }
            Event::Key(key) => {
                let input = if self.focus == ExportControl::Folder {
                    &mut self.folder
                } else {
                    &mut self.input
                };
                input.input(key);
            }
            Event::Paste(text)
                if matches!(self.focus, ExportControl::Folder | ExportControl::Filename) =>
            {
                let input = if self.focus == ExportControl::Folder {
                    &mut self.folder
                } else {
                    &mut self.input
                };
                input.insert_str(text.replace(['\n', '\r'], ""));
            }
            Event::Mouse(mouse) if mouse.kind == MouseEventKind::Down(MouseButton::Left) => {
                let position = (mouse.column, mouse.row).into();
                let layout = self.layout(screen);
                if layout.row(2, 4).contains(position) {
                    self.focus = ExportControl::Folder;
                }
                if layout.row(7, 4).contains(position) {
                    self.focus = ExportControl::Filename;
                }
                for (index, area) in layout.buttons.iter().enumerate() {
                    if area.contains(position) {
                        return Some(if index == 0 {
                            DialogAction::Cancel
                        } else {
                            DialogAction::Submit
                        });
                    }
                }
            }
            _ => {}
        }
        if before != (self.folder.lines().join(""), self.input.lines().join("")) {
            self.edited();
        }
        None
    }

    pub fn draw(&mut self, frame: &mut Frame) {
        let layout = self.layout(frame.area());
        layout.draw(frame, "Export PDF");
        frame.render_widget(
            Paragraph::new(self.context.as_str()).style(theme::muted()),
            layout.row(0, 1),
        );
        frame.render_widget(Paragraph::new("Folder"), layout.row(2, 1));
        field(
            frame,
            layout.row(3, 3),
            &self.folder,
            self.focus == ExportControl::Folder,
        );
        frame.render_widget(Paragraph::new("Filename"), layout.row(7, 1));
        field(
            frame,
            layout.row(8, 3),
            &self.input,
            self.focus == ExportControl::Filename,
        );
        frame.render_widget(
            Paragraph::new(self.message.as_str())
                .wrap(ratatui::widgets::Wrap { trim: false })
                .style(theme::dialog().fg(theme::ERROR)),
            layout.row(11, 3),
        );
        button(
            frame,
            layout.buttons[0],
            "Cancel",
            self.focus == ExportControl::Cancel,
            ButtonStyle::Secondary,
        );
        button(
            frame,
            layout.buttons[1],
            self.action_label(),
            self.focus == ExportControl::Submit,
            if self.overwrite_path.is_some() {
                ButtonStyle::Destructive
            } else {
                ButtonStyle::Primary
            },
        );
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
    fn resume_menu_selects_recovery_without_a_global_actions_menu() -> Result<()> {
        use ratatui::{Terminal, backend::TestBackend};

        let mut menu = ActionMenu::for_resume("Example");
        let mut terminal = Terminal::new(TestBackend::new(80, 30))?;
        terminal.draw(|frame| menu.draw(frame))?;
        let [_, choices, help] = menu.layout(terminal.backend().buffer().area);

        assert!(
            menu.click(terminal.backend().buffer().area, help.x, help.y)
                .is_none()
        );
        assert!(matches!(
            menu.click(terminal.backend().buffer().area, choices.x, choices.y + 3),
            Some(Action::ReloadSource)
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

    #[test]
    fn export_navigation_preserves_confirmation_but_field_edits_invalidate_it() -> Result<()> {
        use ratatui::crossterm::event::{KeyEvent, KeyModifiers};

        let directory = tempfile::tempdir()?;
        let source = directory.path().join("main.pdf");
        let document = directory.path().join("main.tex");
        fs::write(&source, "pdf")?;
        fs::write(directory.path().join("main-export.pdf"), "existing")?;
        let mut dialog = ExportDialog::new(&document);
        assert!(dialog.export(&source, &document)?.is_none());

        let screen = Rect::new(0, 0, 100, 30);
        let key = |code| Event::Key(KeyEvent::new(code, KeyModifiers::NONE));
        dialog.handle_event(key(KeyCode::Right), screen);
        assert!(dialog.overwrite_path.is_some());
        dialog.handle_event(key(KeyCode::Tab), screen);
        assert!(dialog.overwrite_path.is_some());
        dialog.handle_event(Event::Paste("other.pdf".into()), screen);
        assert!(dialog.overwrite_path.is_none());
        assert!(dialog.message.is_empty());

        dialog.input.select_all();
        dialog.handle_event(Event::Paste("../outside.pdf".into()), screen);
        assert!(
            dialog
                .export(&source, &document)
                .unwrap_err()
                .to_string()
                .contains("folder path")
        );
        assert_eq!(
            fs::read_to_string(directory.path().join("main-export.pdf"))?,
            "existing"
        );
        Ok(())
    }

    #[test]
    fn manage_and_export_fit_tiny_terminals_without_activating_the_footer() -> Result<()> {
        use ratatui::{Terminal, backend::TestBackend};

        let mut menu = ActionMenu::for_resume("Example");
        let mut export = ExportDialog::new(Path::new("/tmp/main.tex"));
        for (width, height) in [(100, 30), (40, 16), (20, 8), (1, 1)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height))?;
            terminal.draw(|frame| menu.draw(frame))?;
            terminal.draw(|frame| export.draw(frame))?;
        }

        let screen = Rect::new(0, 0, 100, 30);
        let mut terminal = Terminal::new(TestBackend::new(screen.width, screen.height))?;
        terminal.draw(|frame| menu.draw(frame))?;
        let [_, choices, _] = menu.layout(screen);
        assert!(menu.click(screen, choices.x, choices.bottom()).is_none());
        Ok(())
    }
}
