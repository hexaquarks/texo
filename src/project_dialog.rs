use std::path::Path;

use ratatui::{
    Frame,
    crossterm::event::{Event, KeyCode, MouseButton, MouseEventKind},
    layout::{Position, Rect},
    widgets::{List, ListItem, ListState, Paragraph, Wrap},
};
use ratatui_textarea::TextArea;
use texo::theme;

use crate::{
    actions::Action,
    dialog_ui::{ButtonStyle, DialogAction, FormLayout, button, field},
    resume_project::{ImportPlan, Project},
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Control {
    Source,
    Name,
    Choices,
    Cancel,
    Submit,
}

pub struct ProjectDialog {
    pub action: Action,
    pub name: TextArea<'static>,
    pub source: TextArea<'static>,
    focus: Control,
    context: String,
    pub losses: Vec<&'static str>,
    pub plan: Option<ImportPlan>,
    pub selection: ListState,
    pub error: String,
}

impl ProjectDialog {
    pub fn new(action: Action, active: Option<&Project>, loose: Option<&Path>) -> Self {
        let name = match action {
            Action::RenameResume => active
                .map(|project| project.manifest.name.clone())
                .unwrap_or_default(),
            Action::DuplicateResume => active
                .map(|project| format!("{} copy", project.manifest.name))
                .unwrap_or_default(),
            Action::ImportResume => "Imported resume".into(),
            _ => "New resume".into(),
        };
        let mut name = TextArea::new(vec![name]);
        name.select_all();
        let mut source = TextArea::new(vec![
            loose
                .map(crate::workspace::display_path)
                .unwrap_or_default(),
        ]);
        source.select_all();
        for input in [&mut name, &mut source] {
            input.set_style(theme::dialog());
            input.set_cursor_line_style(theme::dialog());
            input.set_selection_style(theme::dialog().bg(theme::SELECTION));
        }

        Self {
            action,
            name,
            source,
            focus: match action {
                Action::ReloadSource | Action::RecoverSource => Control::Cancel,
                Action::ImportResume => Control::Source,
                _ => Control::Name,
            },
            context: active
                .map(|project| {
                    format!(
                        "{} · {}",
                        project.manifest.name,
                        project.manifest.entrypoint.display()
                    )
                })
                .unwrap_or_default(),
            losses: Vec::new(),
            plan: None,
            selection: ListState::default().with_selected(Some(0)),
            error: String::new(),
        }
    }

    fn confirmation(&self) -> bool {
        matches!(self.action, Action::ReloadSource | Action::RecoverSource)
    }

    fn controls(&self) -> &[Control] {
        if self.confirmation() {
            &[Control::Cancel, Control::Submit]
        } else if self.plan.is_some() {
            &[Control::Choices, Control::Cancel, Control::Submit]
        } else if matches!(self.action, Action::ImportResume) {
            &[
                Control::Source,
                Control::Name,
                Control::Cancel,
                Control::Submit,
            ]
        } else {
            &[Control::Name, Control::Cancel, Control::Submit]
        }
    }

    pub fn input(&mut self) -> &mut TextArea<'static> {
        if self.focus == Control::Source {
            &mut self.source
        } else {
            &mut self.name
        }
    }

    fn action_label(&self) -> &str {
        match self.action {
            Action::ImportResume if self.plan.is_none() => "Review import",
            Action::ImportResume => "Import resume",
            Action::RenameResume => "Rename",
            Action::DuplicateResume => "Duplicate",
            Action::ReloadSource if !self.losses.is_empty() => "Reload & discard",
            Action::ReloadSource => "Reload",
            Action::RecoverSource => "Restore source",
            _ => "Create resume",
        }
    }

    fn content_rows(&self) -> u16 {
        if self.confirmation() || matches!(self.action, Action::ImportResume) {
            15
        } else {
            12
        }
    }

    fn layout(&self, screen: Rect) -> FormLayout {
        let focus_row = match self.focus {
            Control::Name if matches!(self.action, Action::ImportResume) => 8,
            Control::Source | Control::Name => 3,
            _ => 0,
        };
        FormLayout::new(screen, self.content_rows(), focus_row, self.action_label())
    }

    pub fn handle_event(&mut self, event: Event, screen: Rect) -> Option<DialogAction> {
        match event {
            Event::Key(key) if key.code == KeyCode::Esc => {
                if self.plan.take().is_some() {
                    self.focus = Control::Source;
                } else {
                    return Some(DialogAction::Cancel);
                }
            }
            Event::Key(key) if matches!(key.code, KeyCode::Tab | KeyCode::BackTab) => {
                let controls = self.controls();
                let current = controls
                    .iter()
                    .position(|control| *control == self.focus)
                    .unwrap_or(0);
                let offset = if key.code == KeyCode::BackTab {
                    controls.len() - 1
                } else {
                    1
                };
                self.focus = controls[(current + offset) % controls.len()];
            }
            Event::Key(key) if key.code == KeyCode::Enter => {
                return Some(if self.focus == Control::Cancel {
                    DialogAction::Cancel
                } else {
                    DialogAction::Submit
                });
            }
            Event::Key(key) if self.focus == Control::Choices => {
                let count = self.plan.as_ref().map_or(0, |plan| plan.candidates.len());
                let current = self.selection.selected().unwrap_or(0);
                match key.code {
                    KeyCode::Up | KeyCode::Char('k') => {
                        self.selection.select(Some(current.saturating_sub(1)))
                    }
                    KeyCode::Down | KeyCode::Char('j') => self
                        .selection
                        .select(Some((current + 1).min(count.saturating_sub(1)))),
                    _ => {}
                }
            }
            Event::Key(key) if matches!(self.focus, Control::Cancel | Control::Submit) => {
                match key.code {
                    KeyCode::Left | KeyCode::Char('h') => self.focus = Control::Cancel,
                    KeyCode::Right | KeyCode::Char('l') => self.focus = Control::Submit,
                    KeyCode::Up | KeyCode::Char('k') => self.focus = self.controls()[0],
                    KeyCode::Char(' ') => {
                        return Some(if self.focus == Control::Cancel {
                            DialogAction::Cancel
                        } else {
                            DialogAction::Submit
                        });
                    }
                    _ => {}
                }
            }
            Event::Key(key) => {
                self.input().input(key);
            }
            Event::Paste(text) if matches!(self.focus, Control::Source | Control::Name) => {
                self.input().insert_str(text.replace(['\n', '\r'], ""));
            }
            Event::Mouse(mouse) if mouse.kind == MouseEventKind::Down(MouseButton::Left) => {
                let position = Position::new(mouse.column, mouse.row);
                let layout = self.layout(screen);
                for (index, area) in layout.buttons.iter().enumerate() {
                    if area.contains(position) {
                        return Some(if index == 0 {
                            DialogAction::Cancel
                        } else {
                            DialogAction::Submit
                        });
                    }
                }
                if let Some(plan) = &self.plan {
                    let choices = layout.row(3, 8);
                    if choices.contains(position) {
                        let index = self.selection.offset() + usize::from(position.y - choices.y);
                        if index < plan.candidates.len() {
                            self.selection.select(Some(index));
                            self.focus = Control::Choices;
                        }
                    }
                } else if !self.confirmation() {
                    if layout.row(3, 3).contains(position) {
                        self.focus = if matches!(self.action, Action::ImportResume) {
                            Control::Source
                        } else {
                            Control::Name
                        };
                    } else if matches!(self.action, Action::ImportResume)
                        && layout.row(8, 3).contains(position)
                    {
                        self.focus = Control::Name;
                    }
                }
            }
            _ => {}
        }
        None
    }

    pub fn reset_confirmation(&mut self) {
        self.focus = Control::Cancel;
        self.error = "Session changed. Review the consequences before reloading.".into();
    }

    pub fn review_import(&mut self) {
        self.focus = Control::Choices;
    }

    pub fn edit_import(&mut self) {
        self.plan = None;
        self.selection.select(Some(0));
        self.focus = Control::Source;
    }

    pub fn draw(&mut self, frame: &mut Frame) {
        let layout = self.layout(frame.area());
        let title = match self.action {
            Action::ImportResume => "Import resume",
            Action::RenameResume => "Rename resume",
            Action::DuplicateResume => "Duplicate resume",
            Action::ReloadSource => "Reload from disk?",
            Action::RecoverSource => "Restore missing source?",
            _ => "New resume",
        };
        layout.draw(frame, title);
        let intro = if matches!(self.action, Action::ImportResume) {
            "Choose a .tex file or a project folder."
        } else {
            &self.context
        };
        frame.render_widget(
            Paragraph::new(intro).style(theme::muted()),
            layout.row(0, 1),
        );

        if let Some(plan) = &self.plan {
            frame.render_widget(
                Paragraph::new("Choose the TeX entrypoint"),
                layout.row(2, 1),
            );
            let items = plan
                .candidates
                .iter()
                .map(|path| ListItem::new(path.display().to_string()));
            frame.render_stateful_widget(
                List::new(items)
                    .highlight_style(theme::dialog().bg(theme::SELECTION))
                    .highlight_symbol("› "),
                layout.row(3, 8),
                &mut self.selection,
            );
            frame.render_widget(
                Paragraph::new("Supporting files are copied; originals stay untouched.")
                    .style(theme::muted()),
                layout.row(11, 2),
            );
        } else if self.confirmation() {
            let prompt = if matches!(self.action, Action::ReloadSource) {
                "Replace the working copy with the file on disk?"
            } else {
                "Restore the missing source file from the working copy?"
            };
            frame.render_widget(
                Paragraph::new(prompt).wrap(Wrap { trim: false }),
                layout.row(2, 2),
            );
            frame.render_widget(
                Paragraph::new(self.losses.join("\n\n")).wrap(Wrap { trim: false }),
                layout.row(5, 6),
            );
            if matches!(self.action, Action::ReloadSource) {
                frame.render_widget(
                    Paragraph::new("The on-disk file will not be changed.").style(theme::muted()),
                    layout.row(11, 1),
                );
            }
        } else {
            let importing = matches!(self.action, Action::ImportResume);
            frame.render_widget(
                Paragraph::new(if importing { "Source" } else { "Resume name" }),
                layout.row(2, 1),
            );
            if importing {
                field(
                    frame,
                    layout.row(3, 3),
                    &self.source,
                    self.focus == Control::Source,
                );
                frame.render_widget(Paragraph::new("Resume name"), layout.row(7, 1));
                field(
                    frame,
                    layout.row(8, 3),
                    &self.name,
                    self.focus == Control::Name,
                );
                frame.render_widget(
                    Paragraph::new("Review the files before copying.").style(theme::muted()),
                    layout.row(11, 1),
                );
            } else {
                field(
                    frame,
                    layout.row(3, 3),
                    &self.name,
                    self.focus == Control::Name,
                );
                let note = if matches!(self.action, Action::DuplicateResume) {
                    "Copies the working copy and supporting files into a new resume."
                } else {
                    ""
                };
                frame.render_widget(
                    Paragraph::new(note)
                        .wrap(Wrap { trim: false })
                        .style(theme::muted()),
                    layout.row(7, 3),
                );
            }
        }
        frame.render_widget(
            Paragraph::new(self.error.as_str())
                .style(theme::dialog().fg(theme::ERROR))
                .wrap(Wrap { trim: false }),
            layout.row(self.content_rows() - 2, 2),
        );
        button(
            frame,
            layout.buttons[0],
            "Cancel",
            self.focus == Control::Cancel,
            ButtonStyle::Secondary,
        );
        button(
            frame,
            layout.buttons[1],
            self.action_label(),
            self.focus == Control::Submit,
            if matches!(self.action, Action::ReloadSource) && !self.losses.is_empty() {
                ButtonStyle::Destructive
            } else {
                ButtonStyle::Primary
            },
        );
    }
}
