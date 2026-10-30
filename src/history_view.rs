use ratatui::{
    Frame,
    crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEventKind},
    layout::{Alignment, Constraint, Layout, Margin, Rect},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};
use ratatui_textarea::TextArea;
use texo::{preview::Preview, theme};
use uuid::Uuid;

use super::{App, Focus};
use crate::{
    history::{self, Revision},
    projects::Projects,
    render_job::RenderJob,
};

const SCROLL_ROWS: u16 = 3;
const DIALOG_WIDTH: u16 = 62;
const DIALOG_HEIGHT: u16 = 9;
const HISTORY_ERROR_ROWS: u16 = 3;
const REVISION_ROWS: u16 = 2;

#[derive(Default)]
pub(super) struct HistoryState {
    pub revisions: Vec<Revision>,
    pub selection: ListState,
    pub error: Option<String>,
    pub view: Option<HistoricalView>,
    pub dialog: Option<HistoryDialog>,
}

pub(super) struct HistoricalView {
    pub revision: Revision,
    scroll: (u16, u16),
    pub preview: Preview,
    pub job: Option<RenderJob>,
    build: Option<texo::sandbox::CompiledDraft>,
    error: Option<String>,
}

pub(super) enum HistoryDialog {
    Restore(Revision),
    Name {
        id: Uuid,
        input: Box<TextArea<'static>>,
    },
}

impl HistoricalView {
    fn source_lines(&self) -> Vec<Line<'static>> {
        self.revision
            .source
            .lines()
            .enumerate()
            .map(|(index, line)| {
                Line::from(vec![
                    Span::styled(format!("{:>4} ", index + 1), theme::muted()),
                    Span::raw(line.to_owned()),
                ])
            })
            .collect()
    }
}

impl App {
    pub(super) fn refresh_history(&mut self) {
        let selected = self.selected_revision().map(|revision| revision.id);
        match history::revisions(&self.session.editor.path) {
            Ok(revisions) => {
                self.session.history.revisions = revisions.into_iter().rev().collect();
                self.session.history.error = None;
                let index = selected
                    .and_then(|id| {
                        self.session
                            .history
                            .revisions
                            .iter()
                            .position(|revision| revision.id == id)
                    })
                    .map_or(0, |index| index + 1);
                self.session.history.selection.select(Some(index));
                if let Some(view) = &mut self.session.history.view
                    && let Some(revision) = self
                        .session
                        .history
                        .revisions
                        .iter()
                        .find(|revision| revision.id == view.revision.id)
                {
                    view.revision.name.clone_from(&revision.name);
                }
            }
            Err(error) => self.session.history.error = Some(format!("History: {error:#}")),
        }
    }

    pub(super) fn open_history(&mut self) {
        if self.session.empty_workspace {
            self.session.status = "Create or import a resume first.".into();
            return;
        }

        self.refresh_history();
        self.projects_visible = true;
        self.projects.history_focused = true;
        self.focus = Focus::Projects;
    }

    pub(super) fn return_to_working_copy(&mut self) {
        self.session.history.view = None;
        self.session.status.clear();
        self.session.history.selection.select(Some(0));
    }

    fn selected_revision(&self) -> Option<Revision> {
        self.session
            .history
            .selection
            .selected()
            .and_then(|index| index.checked_sub(1))
            .and_then(|index| self.session.history.revisions.get(index))
            .cloned()
    }

    pub(super) fn open_selected_revision(&mut self) {
        let Some(revision) = self.selected_revision() else {
            self.return_to_working_copy();
            self.focus = Focus::Editor;
            return;
        };

        self.session.status.clear();
        self.session.editor.cancel_pending_commands();
        self.session.history.view = Some(HistoricalView {
            revision,
            scroll: (0, 0),
            preview: self.session.preview.empty_like(),
            job: None,
            build: None,
            error: None,
        });

        self.build_history_preview();
    }

    pub(super) fn build_history_preview(&mut self) {
        self.workspace_expansion = None;
        let Some(view) = &mut self.session.history.view else {
            return;
        };
        view.job = None;
        view.preview.clear();
        view.build = None;
        view.error = None;
        match RenderJob::draft(
            view.revision.source.clone(),
            self.session.editor.path.clone(),
        ) {
            Ok(job) => view.job = Some(job),
            Err(error) => view.error = Some(error.to_string()),
        }
    }

    pub(super) fn history_preview_loading(&self) -> bool {
        self.session
            .history
            .view
            .as_ref()
            .is_some_and(|view| view.job.is_some())
    }

    pub(super) fn receive_history_preview(&mut self) {
        let Some(view) = &mut self.session.history.view else {
            return;
        };
        let Some(result) = view.job.as_ref().and_then(RenderJob::poll) else {
            return;
        };
        view.job = None;
        match result {
            Ok(document) => {
                view.build = document.build;
                if let Err(error) = view.preview.set_pages(document.pages) {
                    view.error = Some(error.to_string());
                }
            }
            Err(error) => view.error = Some(format!("{error:#}")),
        }
    }

    pub(super) fn handle_history_key(&mut self, key: KeyEvent) {
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return;
        }
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                let current = self.session.history.selection.selected().unwrap_or(0);
                self.session
                    .history
                    .selection
                    .select(Some(current.saturating_sub(1)));
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let current = self.session.history.selection.selected().unwrap_or(0);
                let last = self.session.history.revisions.len();
                self.session
                    .history
                    .selection
                    .select(Some((current + 1).min(last)));
            }
            KeyCode::Enter => self.open_selected_revision(),
            KeyCode::Esc | KeyCode::Char('w') => self.return_to_working_copy(),
            KeyCode::Char('r') => {
                if self.session.proposal_review.is_some() {
                    self.session.status =
                        "Resolve the AI proposal before restoring a revision.".into();
                } else if let Some(revision) = self.selected_revision() {
                    self.session.history.dialog = Some(HistoryDialog::Restore(revision));
                }
            }
            KeyCode::Char('n') => {
                if let Some(revision) = self.selected_revision() {
                    let mut input = TextArea::new(vec![revision.name.unwrap_or_default()]);
                    input.set_style(theme::dialog());
                    self.session.history.dialog = Some(HistoryDialog::Name {
                        id: revision.id,
                        input: Box::new(input),
                    });
                }
            }
            KeyCode::Tab => {
                self.projects.history_focused = false;
            }
            _ => {}
        }
    }

    pub(super) fn handle_historical_source_key(&mut self, key: KeyEvent) {
        if key.code == KeyCode::Esc {
            self.return_to_working_copy();
            return;
        }
        let Some(view) = &mut self.session.history.view else {
            return;
        };
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => view.scroll.0 = view.scroll.0.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => view.scroll.0 = view.scroll.0.saturating_add(1),
            KeyCode::PageUp => view.scroll.0 = view.scroll.0.saturating_sub(SCROLL_ROWS),
            KeyCode::PageDown => view.scroll.0 = view.scroll.0.saturating_add(SCROLL_ROWS),
            KeyCode::Left => view.scroll.1 = view.scroll.1.saturating_sub(SCROLL_ROWS),
            KeyCode::Right => view.scroll.1 = view.scroll.1.saturating_add(SCROLL_ROWS),
            KeyCode::Home => view.scroll = (0, 0),
            _ => {}
        }

        let lines = view.source_lines();
        let last_row = u16::try_from(lines.len().saturating_sub(1)).unwrap_or(u16::MAX);
        let last_column = lines.iter().map(Line::width).max().unwrap_or(0);
        view.scroll.0 = view.scroll.0.min(last_row);
        view.scroll.1 = view
            .scroll
            .1
            .min(u16::try_from(last_column).unwrap_or(u16::MAX));
    }

    pub(super) fn restore_history_revision(&mut self, revision: &Revision) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.session.proposal_review.is_none(),
            "Resolve the AI proposal before restoring"
        );

        let warning = self.session.editor.restore_revision(revision)?;

        self.session.assistant.cancel();
        self.session.job = None;
        self.return_to_working_copy();

        self.refresh_history();
        self.start_render();
        self.session.status = warning.unwrap_or_else(|| {
            format!(
                "Restored {}. Intervening revisions retained.",
                revision.label()
            )
        });
        Ok(())
    }

    pub(super) fn handle_history_dialog(&mut self, event: Event) {
        let Some(mut dialog) = self.session.history.dialog.take() else {
            return;
        };
        match event {
            Event::Key(key) if key.code == KeyCode::Esc => return,
            Event::Key(key) if key.code == KeyCode::Enter => {
                let result = match &dialog {
                    HistoryDialog::Restore(revision) => self.restore_history_revision(revision),
                    HistoryDialog::Name { id, input } => {
                        history::rename(&self.session.editor.path, *id, &input.lines().join(""))
                    }
                };
                match result {
                    Ok(()) => {
                        self.refresh_history();
                        return;
                    }
                    Err(error) => self.session.history.error = Some(format!("{error:#}")),
                }
            }
            Event::Key(key) => {
                if let HistoryDialog::Name { input, .. } = &mut dialog {
                    input.input(key);
                }
            }
            Event::Paste(text) => {
                if let HistoryDialog::Name { input, .. } = &mut dialog {
                    input.insert_str(text.replace(['\n', '\r'], ""));
                }
            }
            _ => {}
        }
        self.session.history.dialog = Some(dialog);
    }

    pub(super) fn draw_history_sidebar(&mut self, frame: &mut Frame, area: Rect) {
        frame.render_widget(Clear, area);
        frame.render_widget(Block::default().style(theme::text()), area);
        let inner = area.inner(Margin::new(1, 1));
        let filename = self
            .session
            .editor
            .path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();
        let header = Rect {
            height: inner.height.min(1),
            ..inner
        };
        let [title, filename_area] = Layout::horizontal([
            Constraint::Length("History ".len() as u16),
            Constraint::Min(0),
        ])
        .areas(header);
        frame.render_widget(
            Paragraph::new("History").style(theme::pane_title(
                self.focus == Focus::Projects && self.projects.history_focused,
            )),
            title,
        );
        frame.render_widget(
            Paragraph::new(crate::projects::sidebar_text(
                &filename,
                filename_area.width,
            ))
            .alignment(Alignment::Right)
            .style(theme::muted()),
            filename_area,
        );
        if self.session.empty_workspace {
            return;
        }

        let content = Projects::history_content_area(area);
        let [working, entries, error_area] = self.history_entry_areas(content);
        let selected = self.session.history.selection.selected().unwrap_or(0);
        let working_style = if selected == 0 {
            theme::text().bg(theme::SELECTION)
        } else {
            theme::text()
        };
        let working_status = if self.session.editor.is_modified() {
            "Unsaved changes"
        } else {
            "Saved"
        };
        frame.render_widget(
            Paragraph::new(vec![
                Line::from("Working copy"),
                Line::styled(working_status, theme::muted()),
            ])
            .style(working_style),
            working,
        );

        let count = self.session.history.revisions.len();
        let items = self
            .session
            .history
            .revisions
            .iter()
            .enumerate()
            .map(|(index, revision)| {
                let label = format!("revision-{}", count - index);
                let timestamp = time::OffsetDateTime::from_unix_timestamp(revision.timestamp);
                let date = timestamp.map_or_else(
                    |_| "Unknown date".into(),
                    |date| {
                        format!(
                            "{:02}-{:02} {:02}:{:02}",
                            date.month() as u8,
                            date.day(),
                            date.hour(),
                            date.minute()
                        )
                    },
                );
                let detail = format!("{} · {date}", revision.label());
                ListItem::new(vec![
                    Line::from(label),
                    Line::styled(detail, theme::muted()),
                ])
            });
        // Keep the working copy outside the scrolling list so it always remains reachable.
        let mut state = ListState::default()
            .with_selected(selected.checked_sub(1))
            .with_offset(self.session.history.selection.offset());
        frame.render_stateful_widget(
            List::new(items).highlight_style(theme::text().bg(theme::SELECTION)),
            entries,
            &mut state,
        );
        *self.session.history.selection.offset_mut() = state.offset();

        if let Some(error) = &self.session.history.error {
            frame.render_widget(
                Paragraph::new(error.as_str()).style(theme::muted()),
                error_area,
            );
        }
    }

    fn history_entry_areas(&self, content: Rect) -> [Rect; 3] {
        let error_rows = if self.session.history.error.is_some() {
            HISTORY_ERROR_ROWS
        } else {
            0
        };
        Layout::vertical([
            Constraint::Length(REVISION_ROWS),
            Constraint::Min(0),
            Constraint::Length(error_rows),
        ])
        .areas(content)
    }

    pub(super) fn history_mouse(
        &mut self,
        mouse: ratatui::crossterm::event::MouseEvent,
        area: Rect,
    ) -> bool {
        let layout = self.workspace_layout(area);
        if layout.files.contains((mouse.column, mouse.row).into()) {
            if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
                self.focus = Focus::Projects;
                self.projects.history_focused = true;
                let content = Projects::history_content_area(layout.files);
                let [working, entries, _] = self.history_entry_areas(content);
                let position = (mouse.column, mouse.row).into();
                if working.contains(position) {
                    self.return_to_working_copy();
                    self.focus = Focus::Editor;
                } else if entries.contains(position) {
                    let index = self.session.history.selection.offset()
                        + usize::from((mouse.row - entries.y) / REVISION_ROWS);
                    if index < self.session.history.revisions.len() {
                        self.session.history.selection.select(Some(index + 1));
                        self.open_selected_revision();
                    }
                }
            } else {
                let code = match mouse.kind {
                    MouseEventKind::ScrollUp => Some(KeyCode::Up),
                    MouseEventKind::ScrollDown => Some(KeyCode::Down),
                    _ => None,
                };
                if let Some(code) = code {
                    self.handle_history_key(KeyEvent::new(code, KeyModifiers::NONE));
                }
            }
            return true;
        }
        if self.session.history.view.is_some()
            && layout.editor.contains((mouse.column, mouse.row).into())
        {
            let code = match mouse.kind {
                MouseEventKind::ScrollUp => Some(KeyCode::PageUp),
                MouseEventKind::ScrollDown => Some(KeyCode::PageDown),
                MouseEventKind::Down(MouseButton::Left) => {
                    self.focus = Focus::Editor;
                    None
                }
                _ => None,
            };
            if let Some(code) = code {
                self.handle_historical_source_key(KeyEvent::new(code, KeyModifiers::NONE));
            }
            return true;
        }
        false
    }

    pub(super) fn draw_historical_source(&self, frame: &mut Frame, area: Rect) {
        let Some(view) = &self.session.history.view else {
            return;
        };
        let [title, body] =
            Layout::vertical([Constraint::Length(2), Constraint::Min(0)]).areas(area);
        let label = format!(
            "History · read-only · {} · {} UTC",
            view.revision.label(),
            view.revision.date_label()
        );
        let subtitle = "Esc Working copy";
        frame.render_widget(
            Paragraph::new(vec![
                Line::styled(label, theme::dialog_title()),
                Line::styled(subtitle, theme::muted()),
            ]),
            title,
        );
        let lines = view.source_lines();
        frame.render_widget(Paragraph::new(lines).scroll(view.scroll), body);
    }

    pub(super) fn draw_history_pdf(&mut self, frame: &mut Frame, area: Rect) {
        let overlay = self.overlay_open();
        let Some(view) = &mut self.session.history.view else {
            return;
        };
        let [title, body] =
            Layout::vertical([Constraint::Length(2), Constraint::Min(0)]).areas(area);
        frame.render_widget(
            Paragraph::new(vec![
                Line::styled(
                    format!(
                        "History PDF · {} · {}%",
                        view.revision.label(),
                        view.preview.zoom_percent()
                    ),
                    theme::pane_title(true),
                ),
                Line::styled("Current dependencies · source history only", theme::muted()),
            ]),
            title,
        );
        if let Some(error) = &view.error {
            frame.render_widget(
                Paragraph::new(format!("Historical build failed\n{error}"))
                    .wrap(Wrap { trim: false })
                    .style(theme::muted()),
                body,
            );
        } else if view.job.is_some() {
            view.preview.draw_loading(frame, body);
        } else if overlay {
            view.preview.draw_cached(frame, body);
        } else {
            view.preview.draw(frame, body);
        }
    }

    pub(super) fn draw_history_dialog(&self, frame: &mut Frame) {
        let Some(dialog) = &self.session.history.dialog else {
            return;
        };
        let screen = frame.area();
        let width = screen.width.min(DIALOG_WIDTH);
        let height = screen.height.min(DIALOG_HEIGHT);
        let area = Rect::new(
            (screen.width - width) / 2,
            (screen.height - height) / 2,
            width,
            height,
        );
        frame.render_widget(Clear, area);
        let title = match dialog {
            HistoryDialog::Restore(_) => "Restore revision?",
            HistoryDialog::Name { .. } => "Name revision",
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .title(title)
            .style(theme::dialog());
        let inner = block.inner(area).inner(Margin::new(1, 0));
        frame.render_widget(block, area);
        let [body, error, footer] = Layout::vertical([
            Constraint::Min(1),
            Constraint::Length(2),
            Constraint::Length(1),
        ])
        .areas(inner);
        match dialog {
            HistoryDialog::Restore(revision) => {
                let message = format!(
                    "Restore {}? Unsaved work will be kept as a recovery checkpoint. All revisions remain.",
                    revision.label()
                );
                frame.render_widget(Paragraph::new(message).wrap(Wrap { trim: false }), body);
            }
            HistoryDialog::Name { input, .. } => frame.render_widget(input.as_ref(), body),
        }
        frame.render_widget(
            Paragraph::new(self.session.history.error.as_deref().unwrap_or(""))
                .wrap(Wrap { trim: false })
                .style(theme::muted()),
            error,
        );
        frame.render_widget(
            Paragraph::new("Enter Confirm · Esc Cancel").style(theme::muted()),
            footer,
        );
    }
}
