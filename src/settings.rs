use std::path::{Path, PathBuf};

use ratatui::{
    Frame,
    crossterm::event::{Event, KeyCode, KeyModifiers, MouseButton, MouseEventKind},
    layout::{Alignment, Margin, Position, Rect},
    style::Modifier,
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};
use ratatui_textarea::TextArea;
use texo::theme;

use crate::config::{EditingMode, EditorPreferences};

const DIALOG_WIDTH: u16 = 76;
const DIALOG_HEIGHT: u16 = 29;
const PAGE_ROWS: u16 = 25;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SettingsControl {
    Mode,
    Wrap,
    ChangeFolder,
    OpenConfig,
    ReloadConfig,
    Cancel,
    Apply,
}

const CONTROLS: [SettingsControl; 7] = [
    SettingsControl::Mode,
    SettingsControl::Wrap,
    SettingsControl::ChangeFolder,
    SettingsControl::OpenConfig,
    SettingsControl::ReloadConfig,
    SettingsControl::Cancel,
    SettingsControl::Apply,
];

#[derive(Clone, Copy)]
pub enum SettingsAction {
    Apply,
    Cancel,
    ChangeFolder,
    OpenConfig,
    ReloadConfig,
    PrepareFolder,
    SwitchFolder,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FolderControl {
    Destination,
    Cancel,
    Continue,
}

const FOLDER_CONTROLS: [FolderControl; 3] = [
    FolderControl::Destination,
    FolderControl::Cancel,
    FolderControl::Continue,
];
const STANDARD_CHOICE_WIDTH: u16 = 11;

pub struct FolderConfirmation {
    pub destination: PathBuf,
    pub input: String,
    pub losses: Vec<&'static str>,
}

pub struct SettingsDialog {
    pub input: TextArea<'static>,
    pub error: String,
    pub notice: String,
    pub config_path: PathBuf,
    pub prefix_label: String,
    pub editor_preferences: EditorPreferences,
    pub original_preferences: EditorPreferences,
    pub folder_open: bool,
    pub confirmation: Option<FolderConfirmation>,
    pub reload_folder: bool,
    pub focus: SettingsControl,
    root: PathBuf,
    folder_focus: FolderControl,
}

impl SettingsDialog {
    pub fn new(root: &Path, config_path: &Path, prefix_label: String) -> Self {
        let mut input = TextArea::new(vec![crate::workspace::display_path(root)]);
        input.set_style(theme::dialog());
        input.set_cursor_line_style(theme::dialog());
        input.set_selection_style(theme::dialog().bg(theme::SELECTION));
        input.select_all();
        Self {
            input,
            error: String::new(),
            notice: String::new(),
            config_path: config_path.to_path_buf(),
            prefix_label,
            editor_preferences: EditorPreferences::default(),
            original_preferences: EditorPreferences::default(),
            folder_open: false,
            confirmation: None,
            reload_folder: false,
            focus: SettingsControl::Mode,
            root: root.to_path_buf(),
            folder_focus: FolderControl::Destination,
        }
    }

    pub fn value(&self) -> String {
        self.input.lines().join("\n")
    }

    pub fn has_staged_preferences(&self) -> bool {
        self.editor_preferences != self.original_preferences
    }

    pub fn toggle_mode(&mut self) {
        self.editor_preferences.mode = match self.editor_preferences.mode {
            EditingMode::Standard => EditingMode::Vim,
            EditingMode::Vim => EditingMode::Standard,
        };
    }

    pub fn begin_folder_change(&mut self) {
        self.folder_open = true;
        self.confirmation = None;
        self.folder_focus = FolderControl::Destination;
        self.error.clear();
    }

    pub fn confirm_folder(&mut self, destination: PathBuf, losses: Vec<&'static str>) {
        self.confirmation = Some(FolderConfirmation {
            destination,
            input: self.value(),
            losses,
        });
        // Enter after validation must cancel, not confirm a switch.
        self.folder_focus = FolderControl::Cancel;
    }

    pub fn area(screen: Rect) -> Rect {
        let width = DIALOG_WIDTH.min(screen.width);
        let height = DIALOG_HEIGHT.min(screen.height);
        Rect::new(
            screen.x + (screen.width - width) / 2,
            screen.y + (screen.height - height) / 2,
            width,
            height,
        )
    }

    fn inner(screen: Rect) -> Rect {
        Self::area(screen).inner(Margin::new(2, 2))
    }

    fn control_row(control: SettingsControl) -> u16 {
        match control {
            SettingsControl::Mode => 2,
            SettingsControl::Wrap => 4,
            SettingsControl::ChangeFolder => 9,
            SettingsControl::OpenConfig | SettingsControl::ReloadConfig => 17,
            SettingsControl::Cancel | SettingsControl::Apply => 22,
        }
    }

    fn row(&self, screen: Rect, row: u16, height: u16) -> Rect {
        let inner = Self::inner(screen);
        let focus_row = if self.folder_open {
            if self.folder_focus == FolderControl::Destination {
                2
            } else {
                12
            }
        } else {
            Self::control_row(self.focus)
        };
        let scroll = focus_row
            .saturating_add(2)
            .saturating_sub(inner.height)
            .min(PAGE_ROWS.saturating_sub(inner.height));
        if row < scroll {
            return Rect::default();
        }
        Rect::new(
            inner.x,
            inner.y.saturating_add(row - scroll),
            inner.width,
            height,
        )
        .intersection(inner)
    }

    fn control_area(&self, screen: Rect, control: SettingsControl) -> Rect {
        let row = self.row(screen, Self::control_row(control), 1);
        let (offset, width) = match control {
            SettingsControl::Mode => (row.width.saturating_sub(18), 18),
            SettingsControl::Wrap => (row.width.saturating_sub(13), 13),
            SettingsControl::ChangeFolder => (row.width.saturating_sub(11), 11),
            SettingsControl::OpenConfig => (0, 15),
            SettingsControl::ReloadConfig => (17, 17),
            SettingsControl::Cancel => (row.width.saturating_sub(28), 10),
            SettingsControl::Apply => (row.width.saturating_sub(16), 16),
        };
        Rect::new(row.x.saturating_add(offset), row.y, width, row.height).intersection(row)
    }

    fn activate(&mut self, control: SettingsControl) -> Option<SettingsAction> {
        match control {
            SettingsControl::Mode => self.toggle_mode(),
            SettingsControl::Wrap => {
                self.editor_preferences.soft_wrap = !self.editor_preferences.soft_wrap
            }
            SettingsControl::ChangeFolder => return Some(SettingsAction::ChangeFolder),
            SettingsControl::OpenConfig => return Some(SettingsAction::OpenConfig),
            SettingsControl::ReloadConfig => return Some(SettingsAction::ReloadConfig),
            SettingsControl::Cancel => return Some(SettingsAction::Cancel),
            SettingsControl::Apply => return Some(SettingsAction::Apply),
        }
        None
    }

    fn navigate(&mut self, direction: char) {
        use SettingsControl::*;

        if self.focus == Mode && matches!(direction, 'h' | 'l') {
            self.editor_preferences.mode = if direction == 'h' {
                EditingMode::Standard
            } else {
                EditingMode::Vim
            };
            return;
        }

        self.focus = match (self.focus, direction) {
            (Mode, 'j') => Wrap,
            (Wrap, 'j') => ChangeFolder,
            (ChangeFolder, 'j') => OpenConfig,
            (OpenConfig, 'j') => Cancel,
            (ReloadConfig, 'j') => Apply,
            (Wrap, 'k') => Mode,
            (ChangeFolder, 'k') => Wrap,
            (OpenConfig | ReloadConfig, 'k') => ChangeFolder,
            (Cancel, 'k') => OpenConfig,
            (Apply, 'k') => ReloadConfig,
            (OpenConfig, 'l') => ReloadConfig,
            (ReloadConfig, 'h') => OpenConfig,
            (Cancel, 'l') => Apply,
            (Apply, 'h') => Cancel,
            _ => self.focus,
        };
    }

    pub fn handle_event(&mut self, event: Event, screen: Rect) -> Option<SettingsAction> {
        if self.folder_open {
            return self.handle_folder_event(event, screen);
        }
        match event {
            Event::Key(key) if key.modifiers.contains(KeyModifiers::CONTROL) => match key.code {
                KeyCode::Char('o') => return Some(SettingsAction::OpenConfig),
                KeyCode::Char('r') => return Some(SettingsAction::ReloadConfig),
                _ => {}
            },
            Event::Key(key) => match key.code {
                KeyCode::Esc => return Some(SettingsAction::Cancel),
                KeyCode::Tab | KeyCode::BackTab => {
                    let current = CONTROLS
                        .iter()
                        .position(|control| *control == self.focus)
                        .unwrap();
                    let next = if key.code == KeyCode::BackTab {
                        (current + CONTROLS.len() - 1) % CONTROLS.len()
                    } else {
                        (current + 1) % CONTROLS.len()
                    };
                    self.focus = CONTROLS[next];
                }
                KeyCode::Up | KeyCode::Char('k') => self.navigate('k'),
                KeyCode::Down | KeyCode::Char('j') => self.navigate('j'),
                KeyCode::Left | KeyCode::Char('h') => self.navigate('h'),
                KeyCode::Right | KeyCode::Char('l') => self.navigate('l'),
                KeyCode::Enter | KeyCode::Char(' ') => return self.activate(self.focus),
                _ => {}
            },
            Event::Mouse(mouse) if mouse.kind == MouseEventKind::Down(MouseButton::Left) => {
                let position = Position::new(mouse.column, mouse.row);
                for control in CONTROLS {
                    let area = self.control_area(screen, control);
                    if area.contains(position) {
                        self.focus = control;
                        if control == SettingsControl::Mode {
                            self.editor_preferences.mode =
                                if mouse.column < area.x + STANDARD_CHOICE_WIDTH {
                                    EditingMode::Standard
                                } else {
                                    EditingMode::Vim
                                };
                            return None;
                        }
                        return self.activate(control);
                    }
                }
            }
            _ => {}
        }
        None
    }

    fn folder_buttons(&self, screen: Rect) -> [Rect; 2] {
        let row = self.row(screen, 12, 1);
        [
            Rect::new(row.x, row.y, 10, row.height).intersection(row),
            Rect::new(row.x.saturating_add(12), row.y, 22, row.height).intersection(row),
        ]
    }

    fn handle_folder_event(&mut self, event: Event, screen: Rect) -> Option<SettingsAction> {
        match event {
            Event::Key(key) if key.code == KeyCode::Esc => {
                self.folder_open = false;
                self.confirmation = None;
                self.error.clear();
            }
            Event::Key(key) if matches!(key.code, KeyCode::Tab | KeyCode::BackTab) => {
                let current = FOLDER_CONTROLS
                    .iter()
                    .position(|control| *control == self.folder_focus)
                    .unwrap();
                let next = if key.code == KeyCode::BackTab {
                    (current + FOLDER_CONTROLS.len() - 1) % FOLDER_CONTROLS.len()
                } else {
                    (current + 1) % FOLDER_CONTROLS.len()
                };
                self.folder_focus = FOLDER_CONTROLS[next];
            }
            // Vim navigation is only active on buttons; the destination keeps literal typing.
            Event::Key(key)
                if self.folder_focus != FolderControl::Destination
                    && key.modifiers.is_empty()
                    && matches!(
                        key.code,
                        KeyCode::Char('h' | 'j' | 'k' | 'l')
                            | KeyCode::Left
                            | KeyCode::Right
                            | KeyCode::Up
                            | KeyCode::Down
                    ) =>
            {
                self.folder_focus = match key.code {
                    KeyCode::Char('h') | KeyCode::Left => FolderControl::Cancel,
                    KeyCode::Char('l') | KeyCode::Right => FolderControl::Continue,
                    KeyCode::Char('k') | KeyCode::Up => FolderControl::Destination,
                    _ => self.folder_focus,
                };
            }
            Event::Key(key) if key.code == KeyCode::Enter => return self.activate_folder_control(),
            Event::Mouse(mouse) if mouse.kind == MouseEventKind::Down(MouseButton::Left) => {
                let position = (mouse.column, mouse.row).into();
                if self.row(screen, 2, 2).contains(position) {
                    self.folder_focus = FolderControl::Destination;
                }
                for (index, button) in self.folder_buttons(screen).into_iter().enumerate() {
                    if button.contains(position) {
                        self.folder_focus = FOLDER_CONTROLS[index + 1];
                        return self.activate_folder_control();
                    }
                }
            }
            Event::Key(key) if self.folder_focus == FolderControl::Destination => {
                let before = self.value();
                self.input.input(key);
                if self.value() != before {
                    self.confirmation = None;
                    self.reload_folder = false;
                    self.error.clear();
                }
            }
            Event::Paste(text) if self.folder_focus == FolderControl::Destination => {
                if text.trim().contains(['\n', '\r']) {
                    self.error = "Enter a single folder path".into();
                } else {
                    self.input.insert_str(text.trim());
                    self.confirmation = None;
                    self.reload_folder = false;
                    self.error.clear();
                }
            }
            _ => {}
        }
        None
    }

    fn activate_folder_control(&mut self) -> Option<SettingsAction> {
        if self.folder_focus == FolderControl::Cancel {
            self.folder_open = false;
            self.confirmation = None;
            self.error.clear();
            return None;
        }
        if self.confirmation.is_some() && self.folder_focus == FolderControl::Continue {
            Some(SettingsAction::SwitchFolder)
        } else {
            Some(SettingsAction::PrepareFolder)
        }
    }

    pub fn draw(&self, frame: &mut Frame) {
        let screen = frame.area();
        let area = Self::area(screen);
        frame.render_widget(Clear, area);
        frame.render_widget(
            Block::default()
                .title(if self.folder_open {
                    " Change workspace "
                } else {
                    " Settings "
                })
                .borders(Borders::ALL)
                .border_style(theme::dialog_title())
                .style(theme::dialog()),
            area,
        );
        if self.folder_open {
            self.draw_folder(frame);
            return;
        }

        for (row, title) in [(0, "EDITOR"), (7, "WORKSPACE"), (12, "ADVANCED")] {
            frame.render_widget(
                Paragraph::new(title).style(theme::muted()),
                self.row(screen, row, 1),
            );
        }
        for (row, text) in [(2, "Input mode"), (4, "Soft wrap"), (14, "Prefix key")] {
            frame.render_widget(Paragraph::new(text), self.row(screen, row, 1));
        }

        let folder_row = self.row(screen, 9, 1);
        let folder_text = crate::projects::sidebar_text(
            &crate::workspace::display_path(&self.root),
            folder_row.width.saturating_sub(12),
        );
        frame.render_widget(Paragraph::new(folder_text), folder_row);
        frame.render_widget(
            Paragraph::new(format!("{} · config file", self.prefix_label))
                .alignment(Alignment::Right)
                .style(theme::muted()),
            self.row(screen, 14, 1),
        );
        frame.render_widget(
            Paragraph::new(crate::workspace::display_path(&self.config_path)).style(theme::muted()),
            self.row(screen, 16, 1),
        );

        for control in CONTROLS {
            if control == SettingsControl::Mode {
                self.draw_mode(frame);
                continue;
            }
            let label = match control {
                SettingsControl::Mode => unreachable!(),
                SettingsControl::Wrap => {
                    if self.editor_preferences.soft_wrap {
                        "[x] Enabled"
                    } else {
                        "[ ] Disabled"
                    }
                }
                SettingsControl::ChangeFolder => "Change…",
                SettingsControl::OpenConfig => "Open config",
                SettingsControl::ReloadConfig => "Reload config",
                SettingsControl::Cancel => "Cancel",
                SettingsControl::Apply => "Apply changes",
            };
            let style = if self.focus == control {
                theme::dialog().fg(theme::BACKGROUND).bg(theme::ACCENT)
            } else {
                theme::dialog().bg(theme::SELECTION)
            };
            frame.render_widget(
                Paragraph::new(label)
                    .alignment(Alignment::Center)
                    .style(style),
                self.control_area(screen, control),
            );
        }

        let notice = if self.has_staged_preferences() {
            "Preference changes are not applied yet."
        } else {
            &self.notice
        };
        let message = if self.error.is_empty() {
            notice
        } else {
            &self.error
        };
        frame.render_widget(
            Paragraph::new(message)
                .wrap(Wrap { trim: false })
                .style(theme::dialog().fg(if self.error.is_empty() {
                    theme::GREEN
                } else {
                    theme::ERROR
                })),
            self.row(screen, 19, 2),
        );
        frame.render_widget(
            Paragraph::new("h/j/k/l / Tab move · Space toggle · Esc cancel").style(theme::muted()),
            self.row(screen, 24, 1),
        );
    }

    fn draw_mode(&self, frame: &mut Frame) {
        let area = self.control_area(frame.area(), SettingsControl::Mode);
        for (offset, width, label, mode) in [
            (0, STANDARD_CHOICE_WIDTH, "Standard", EditingMode::Standard),
            (STANDARD_CHOICE_WIDTH, 7, "Vim", EditingMode::Vim),
        ] {
            let choice = Rect::new(area.x.saturating_add(offset), area.y, width, area.height)
                .intersection(area);
            let mut style = if self.editor_preferences.mode == mode {
                theme::dialog().fg(theme::BACKGROUND).bg(theme::ACCENT)
            } else {
                theme::dialog().bg(theme::SELECTION)
            };
            if self.focus == SettingsControl::Mode && self.editor_preferences.mode == mode {
                style = style.add_modifier(Modifier::UNDERLINED);
            }
            frame.render_widget(
                Paragraph::new(label)
                    .alignment(Alignment::Center)
                    .style(style),
                choice,
            );
        }
    }

    fn draw_folder(&self, frame: &mut Frame) {
        let screen = frame.area();
        let destination_label = if self.confirmation.is_some() {
            "Destination · validated"
        } else {
            "Destination"
        };
        frame.render_widget(Paragraph::new(destination_label), self.row(screen, 0, 1));
        frame.render_widget(&self.input, self.row(screen, 2, 2));
        let message = if let Some(confirmation) = &self.confirmation {
            if confirmation.losses.is_empty() {
                "Switch to this workspace?".into()
            } else {
                format!("This discards: {}.", confirmation.losses.join(", "))
            }
        } else {
            "Choose the destination folder.".into()
        };
        frame.render_widget(
            Paragraph::new(message).wrap(Wrap { trim: false }),
            self.row(screen, 5, 5),
        );
        frame.render_widget(
            Paragraph::new(self.error.as_str())
                .wrap(Wrap { trim: false })
                .style(theme::dialog().fg(theme::ERROR)),
            self.row(screen, 10, 2),
        );
        let action = match &self.confirmation {
            Some(confirmation) if !confirmation.losses.is_empty() => "Switch & discard",
            Some(_) => "Switch workspace",
            None => "Continue",
        };
        for (index, (area, label)) in self
            .folder_buttons(screen)
            .into_iter()
            .zip(["Cancel", action])
            .enumerate()
        {
            let style = if self.folder_focus == FOLDER_CONTROLS[index + 1] {
                theme::dialog().fg(theme::BACKGROUND).bg(theme::ACCENT)
            } else {
                theme::dialog().bg(theme::SELECTION)
            };
            frame.render_widget(
                Paragraph::new(label)
                    .alignment(Alignment::Center)
                    .style(style),
                area,
            );
        }
    }
}
