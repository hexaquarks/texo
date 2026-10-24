use std::path::Path;

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    widgets::{Block, Clear, List, ListItem, ListState, Padding, Paragraph},
};
use ratatui_textarea::TextArea;
use texo::theme;

use crate::{
    actions::Action,
    resume_project::{ImportPlan, Project},
};

const DIALOG_WIDTH: u16 = 76;
const DIALOG_HEIGHT: u16 = 18;

pub struct ProjectDialog {
    pub action: Action,
    pub name: TextArea<'static>,
    pub source: TextArea<'static>,
    pub source_focused: bool,
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
            input.set_style(theme::text().bg(theme::ELEVATED));
            input.set_cursor_line_style(theme::text().bg(theme::ELEVATED));
            input.set_selection_style(theme::text().bg(theme::SELECTION));
        }

        Self {
            action,
            name,
            source,
            source_focused: matches!(action, Action::ImportResume),
            plan: None,
            selection: ListState::default().with_selected(Some(0)),
            error: String::new(),
        }
    }

    pub fn input(&mut self) -> &mut TextArea<'static> {
        if self.source_focused {
            &mut self.source
        } else {
            &mut self.name
        }
    }

    pub fn draw(&mut self, frame: &mut Frame) {
        let screen = frame.area();
        let width = screen.width.min(DIALOG_WIDTH);
        let height = screen.height.min(DIALOG_HEIGHT);
        let area = Rect::new(
            screen.x + (screen.width - width) / 2,
            screen.y + (screen.height - height) / 2,
            width,
            height,
        );
        let title = match self.action {
            Action::ImportResume => " Import resume ",
            Action::RenameResume => " Rename resume ",
            Action::DuplicateResume => " Duplicate resume ",
            Action::ReloadSource => " Reload source — discard local edits and pending AI? ",
            Action::RecoverSource => " Restore missing source from the editor? ",
            _ => " New resume ",
        };

        let block = Block::bordered()
            .title(title)
            .padding(Padding::uniform(1))
            .style(theme::text())
            .border_style(theme::pane_title(true));
        let inner = block.inner(area);
        frame.render_widget(Clear, area);
        frame.render_widget(block, area);

        let [form, error, help] = Layout::vertical([
            Constraint::Min(0),
            Constraint::Length(2),
            Constraint::Length(1),
        ])
        .areas(inner);

        for (input, focused) in [
            (&mut self.source, self.source_focused),
            (&mut self.name, !self.source_focused),
        ] {
            let cursor = if focused {
                theme::text().fg(theme::BACKGROUND).bg(theme::ACCENT)
            } else {
                theme::text().bg(theme::ELEVATED)
            };
            input.set_cursor_style(cursor);
        }

        if let Some(plan) = &self.plan {
            let items: Vec<_> = plan
                .candidates
                .iter()
                .map(|path| ListItem::new(path.display().to_string()))
                .collect();
            frame.render_stateful_widget(
                List::new(items)
                    .block(Block::default().title("Choose the TeX entrypoint"))
                    .highlight_style(theme::text().bg(theme::SELECTION)),
                form,
                &mut self.selection,
            );
        } else if matches!(self.action, Action::ImportResume) {
            let [source_label, source, name_label, name, note] = Layout::vertical([
                Constraint::Length(1),
                Constraint::Length(2),
                Constraint::Length(1),
                Constraint::Length(2),
                Constraint::Min(0),
            ])
            .areas(form);
            frame.render_widget(
                Paragraph::new("Source .tex file or project folder"),
                source_label,
            );
            frame.render_widget(&self.source, source);
            frame.render_widget(Paragraph::new("Display name"), name_label);
            frame.render_widget(&self.name, name);
            frame.render_widget(
                Paragraph::new(
                    "Copies the source folder and its supporting files. Originals stay untouched.",
                )
                .wrap(ratatui::widgets::Wrap { trim: false })
                .style(theme::muted()),
                note,
            );
        } else if !matches!(self.action, Action::ReloadSource | Action::RecoverSource) {
            frame.render_widget(
                &self.name,
                Rect {
                    height: form.height.min(1),
                    ..form
                },
            );
        }

        frame.render_widget(
            Paragraph::new(self.error.as_str())
                .style(theme::text().fg(theme::ERROR))
                .wrap(ratatui::widgets::Wrap { trim: false }),
            error,
        );
        frame.render_widget(
            Paragraph::new(theme::shortcuts(&[
                ("Enter", "confirm"),
                ("Tab", "field"),
                ("Esc", "cancel"),
            ])),
            help,
        );
    }
}
