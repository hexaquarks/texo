use std::{path::PathBuf, time::Duration};

#[cfg(test)]
use anyhow::Context;

use ratatui::{
    DefaultTerminal, Frame,
    crossterm::event::{
        self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
    },
    layout::{Constraint, Layout, Position, Rect, Spacing},
    style::Style,
    widgets::{Block, Borders, Paragraph, Wrap},
};
use texo::{preview::Preview, theme};

use crate::{
    actions::{Action, ActionMenu, ExportDialog},
    assistant::{Assistant, ProposedEdit},
    editor::Editor,
    projects::Projects,
    proposal::ProposalReview,
    render_job::RenderJob,
};

const EVENT_POLL_INTERVAL: Duration = Duration::from_millis(100);
const MULTIPANE_WIDTH: u16 = 100;
const MULTIPANE_HEIGHT: u16 = 20;
const SIDEBAR_WIDTH: u16 = 24;
const PREVIEW_PERCENT: u16 = 38;
const EDITOR_PERCENT: u16 = 55;
const DIAGNOSTICS_HEIGHT: u16 = 8;
const DIAGNOSTIC_SCROLL_LINES: u16 = 4;

#[derive(Clone, Copy, PartialEq)]
enum Focus {
    Projects,
    Editor,
    Preview,
    Assistant,
}

struct App {
    editor: Editor,
    projects: Projects,
    projects_visible: bool,
    preview: Preview,
    job: Option<RenderJob>,
    rendered_source: Option<String>,
    rendered_pdf: Option<PathBuf>,
    actions: Option<ActionMenu>,
    export: Option<ExportDialog>,
    assistant: Assistant,
    proposal_review: Option<ProposalReview>,
    status: String,
    diagnostics: Option<String>,
    diagnostic_scroll: u16,
    focus: Focus,
}

pub fn run(terminal: &mut DefaultTerminal, editor: Editor, root: PathBuf) -> anyhow::Result<()> {
    let mut app = App::with_workspace(editor, Preview::detect(), root)?;
    if !app.editor.is_modified() {
        app.start_render();
    }

    loop {
        app.receive_render();
        app.receive_assistant();
        terminal.draw(|frame| app.draw(frame))?;

        if event::poll(EVENT_POLL_INTERVAL)? {
            let area = terminal.get_frame().area();
            let preview_was_loading = app.job.is_some();
            let previous_focus = app.focus;
            let previous_document = app.editor.path.clone();
            let projects_was_visible = app.projects_visible;
            let overlay_was_open = app.overlay_open();
            if app.handle_event(event::read()?, area) {
                return Ok(());
            }
            if previous_focus != app.focus
                || projects_was_visible != app.projects_visible
                || overlay_was_open != app.overlay_open()
                || previous_document != app.editor.path
                || preview_was_loading != app.job.is_some()
            {
                // Repaint cells formerly occupied by the terminal's PDF image placement.
                terminal.clear()?;
            }
        }
    }
}

impl App {
    #[cfg(test)]
    fn new(editor: Editor, preview: Preview) -> anyhow::Result<Self> {
        let root = editor
            .path
            .parent()
            .context("The document needs a workspace directory")?
            .to_path_buf();
        Self::with_workspace(editor, preview, root)
    }

    fn with_workspace(editor: Editor, preview: Preview, root: PathBuf) -> anyhow::Result<Self> {
        let projects = Projects::new(root, &editor.path)?;
        Ok(Self {
            editor,
            projects,
            projects_visible: true,
            preview,
            job: None,
            rendered_source: None,
            rendered_pdf: None,
            actions: None,
            export: None,
            assistant: Assistant::new(),
            proposal_review: None,
            status: String::new(),
            diagnostics: None,
            diagnostic_scroll: 0,
            focus: Focus::Editor,
        })
    }

    fn overlay_open(&self) -> bool {
        self.actions.is_some() || self.export.is_some() || self.proposal_review.is_some()
    }

    fn send_message(&mut self) {
        match self
            .assistant
            .submit(self.editor.source(), &self.editor.path)
        {
            Ok(()) => self.status.clear(),
            Err(error) => self.status = error.to_string(),
        }
    }

    fn receive_assistant(&mut self) {
        if let Some(result) = self.assistant.poll() {
            match result {
                Ok(()) if self.assistant.proposal().is_some() => self.apply_proposal(),
                Ok(()) => self.status = "Assistant replied without changing the document.".into(),
                Err(_) => self.status = "Assistant failed. See its message for details.".into(),
            }
        }
    }

    fn apply_proposal(&mut self) {
        let Some(proposal) = self.assistant.proposal().cloned() else {
            return;
        };
        if self.apply_edit(&proposal) {
            self.assistant.take_proposal();
        }
        self.proposal_review = None;
    }

    fn apply_edit(&mut self, proposal: &ProposedEdit) -> bool {
        if proposal.source_before != self.editor.source() {
            self.status =
                "Edit not applied: the document changed during the request. Ask Texo again.".into();
            self.assistant.report_edit_result(
                "Not applied: you changed the editor while I was responding. Your newer text is preserved.",
            );
            return false;
        }

        if self.editor.replace_source(&proposal.source_after) {
            self.status =
                "Edit applied. Ctrl+Z twice to undo; Ctrl+S to save; Ctrl+B to build.".into();
            self.assistant
                .report_edit_result("Applied to the open editor. Not yet saved or rebuilt.");
        } else {
            self.status = "The returned resume is unchanged.".into();
            self.assistant
                .report_edit_result("No changes applied: the returned text matches the editor.");
        }
        true
    }

    fn start_render(&mut self) {
        if self.job.is_some() {
            return;
        }

        match RenderJob::start(self.editor.source(), self.editor.path.clone()) {
            Ok(job) => {
                self.job = Some(job);
                self.diagnostics = None;
                self.status = "Building…".into();
            }
            Err(error) => self.report_error(error),
        }
    }

    fn receive_render(&mut self) {
        let Some(result) = self.job.as_ref().and_then(RenderJob::poll) else {
            return;
        };
        self.job = None;

        match result {
            Ok(document) => match self.preview.set_pages(document.pages) {
                Ok(()) => {
                    self.rendered_source = Some(document.source);
                    self.rendered_pdf = Some(document.pdf_path);
                    self.status.clear();
                    self.diagnostics = None;
                    self.refresh_projects();
                }
                Err(error) => self.report_error(error),
            },
            Err(error) => self.report_error(error),
        }
    }

    fn report_error(&mut self, error: anyhow::Error) {
        self.status = "Operation failed. See diagnostics below; PgUp/PgDn to scroll.".into();
        self.diagnostics = Some(format!("{error:#}"));
        self.diagnostic_scroll = 0;
    }

    fn handle_event(&mut self, event: Event, area: Rect) -> bool {
        if self.overlay_open() {
            return self.handle_overlay(event, area);
        }
        match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => return self.handle_key(key),
            Event::Paste(text) if self.focus == Focus::Assistant => {
                self.assistant.input.insert_str(text);
            }
            Event::Paste(text) if self.focus == Focus::Editor => {
                self.editor.text.insert_str(text);
            }
            Event::Mouse(mouse) => {
                let layout = WorkspaceLayout::new(
                    area,
                    self.diagnostics.is_some(),
                    self.projects_visible,
                    self.focus,
                );
                let position = Position::new(mouse.column, mouse.row);
                if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
                    if layout.projects.contains(position) {
                        self.focus = Focus::Projects;
                        if self.projects.select_at(layout.projects, mouse.row) {
                            self.activate_projects();
                        }
                    } else if Assistant::send_button(pane_content(layout.assistant))
                        .contains(position)
                    {
                        self.focus = Focus::Assistant;
                        if self.assistant.is_running() {
                            self.assistant.cancel();
                        } else {
                            self.send_message();
                        }
                    } else if layout.assistant.contains(position) {
                        self.focus = Focus::Assistant;
                        self.assistant.click(
                            pane_content(layout.assistant),
                            mouse.column,
                            mouse.row,
                        );
                    } else if layout.preview.contains(position) {
                        self.focus = Focus::Preview;
                    } else if layout.editor.contains(position) {
                        self.focus = Focus::Editor;
                        self.editor
                            .click(pane_content(layout.editor), mouse.column, mouse.row);
                    }
                } else if layout.editor.contains(position) {
                    self.editor.text.input(mouse);
                } else if layout.assistant.contains(position) {
                    match mouse.kind {
                        MouseEventKind::ScrollDown => self.assistant.scroll(false),
                        MouseEventKind::ScrollUp => self.assistant.scroll(true),
                        _ => {}
                    }
                } else if layout.preview.contains(position) {
                    match mouse.kind {
                        MouseEventKind::ScrollDown => self.preview.scroll(false),
                        MouseEventKind::ScrollUp => self.preview.scroll(true),
                        _ => {}
                    }
                } else if layout.projects.contains(position) {
                    match mouse.kind {
                        MouseEventKind::ScrollDown => self.projects.move_selection(true),
                        MouseEventKind::ScrollUp => self.projects.move_selection(false),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
        false
    }

    fn handle_key(&mut self, key: KeyEvent) -> bool {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('q') {
            return true;
        }

        if key.modifiers.contains(KeyModifiers::CONTROL) {
            self.handle_control_key(key);
        } else if key.modifiers.contains(KeyModifiers::ALT) {
            self.handle_alt_key(key);
        } else {
            self.handle_navigation_key(key);
        }
        false
    }

    fn handle_control_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('j') => {
                self.focus = if self.focus == Focus::Assistant {
                    Focus::Editor
                } else {
                    Focus::Assistant
                };
            }
            KeyCode::Char('b') => self.start_render(),
            KeyCode::Char('p') => self.actions = Some(ActionMenu::open()),
            KeyCode::Char('e') => self.open_export(),
            KeyCode::Char('t') => {
                self.projects_visible = !self.projects_visible;
                if self.projects_visible {
                    self.focus = Focus::Projects;
                } else if self.focus == Focus::Projects {
                    self.focus = Focus::Editor;
                }
            }
            KeyCode::Char('s') => match self.editor.save() {
                Ok(()) => {
                    self.status = "Saved TeX document.".into();
                    self.refresh_projects();
                    self.projects.select_path(&self.editor.path);
                }
                Err(error) => self.report_error(error),
            },
            KeyCode::Char('z') if self.focus == Focus::Editor => {
                self.editor.text.undo();
            }
            KeyCode::Char('y') if self.focus == Focus::Editor => {
                self.editor.text.redo();
            }
            _ if self.focus == Focus::Editor => {
                self.editor.text.input(key);
            }
            _ if self.focus == Focus::Assistant => {
                self.assistant.input.input(key);
            }
            _ => {}
        }
    }

    fn handle_alt_key(&mut self, key: KeyEvent) {
        if self.focus == Focus::Assistant && key.code == KeyCode::Enter {
            self.assistant.input.insert_newline();
            return;
        }
        let result = match key.code {
            KeyCode::Left => self.preview.previous_page(),
            KeyCode::Right => self.preview.next_page(),
            KeyCode::Up => {
                self.preview.scroll(true);
                Ok(())
            }
            KeyCode::Down => {
                self.preview.scroll(false);
                Ok(())
            }
            _ => {
                if self.focus == Focus::Editor {
                    self.editor.text.input(key);
                }
                Ok(())
            }
        };
        if let Err(error) = result {
            self.report_error(error);
        }
    }

    fn handle_navigation_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::F(5) => self.refresh_projects(),
            KeyCode::F(6) => {
                self.focus = match self.focus {
                    Focus::Editor => Focus::Assistant,
                    Focus::Assistant => Focus::Preview,
                    Focus::Preview if self.projects_visible => Focus::Projects,
                    Focus::Preview | Focus::Projects => Focus::Editor,
                };
            }
            KeyCode::Esc if self.focus == Focus::Assistant && self.assistant.is_running() => {
                self.assistant.cancel()
            }
            KeyCode::Enter if self.focus == Focus::Assistant => self.send_message(),
            KeyCode::PageUp if self.focus == Focus::Assistant => self.assistant.scroll(true),
            KeyCode::PageDown if self.focus == Focus::Assistant => self.assistant.scroll(false),
            KeyCode::Esc if self.diagnostics.is_some() => self.diagnostics = None,
            KeyCode::PageUp if self.diagnostics.is_some() => {
                self.diagnostic_scroll = self
                    .diagnostic_scroll
                    .saturating_sub(DIAGNOSTIC_SCROLL_LINES);
            }
            KeyCode::PageDown if self.diagnostics.is_some() => {
                self.diagnostic_scroll = self
                    .diagnostic_scroll
                    .saturating_add(DIAGNOSTIC_SCROLL_LINES);
            }
            _ if self.focus == Focus::Assistant => {
                self.assistant.input.input(key);
            }
            _ if self.focus == Focus::Projects => self.handle_projects_key(key),
            _ if self.focus == Focus::Editor => {
                self.editor.text.input(key);
            }
            _ => {}
        }
    }

    fn open_export(&mut self) {
        if self.job.is_some() {
            self.status = "Wait for the build to finish before exporting.".into();
            return;
        }
        if self.rendered_source.as_deref() != Some(self.editor.source().as_str())
            || self.rendered_pdf.is_none()
        {
            self.status = "Build the current document with Ctrl+B before exporting.".into();
            return;
        }
        self.export = Some(ExportDialog::new(&self.editor.path));
    }

    fn handle_overlay(&mut self, event: Event, area: Rect) -> bool {
        match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('q') {
                    return self.handle_key(key);
                }
                if key.code == KeyCode::Esc {
                    self.actions = None;
                    self.export = None;
                    self.proposal_review = None;
                } else if self.proposal_review.is_some() {
                    match key.code {
                        KeyCode::Enter => self.apply_proposal(),
                        KeyCode::PageUp | KeyCode::Up => {
                            self.proposal_review.as_mut().unwrap().scroll(true)
                        }
                        KeyCode::PageDown | KeyCode::Down => {
                            self.proposal_review.as_mut().unwrap().scroll(false)
                        }
                        KeyCode::Left => self
                            .proposal_review
                            .as_mut()
                            .unwrap()
                            .scroll_horizontal(true),
                        KeyCode::Right => self
                            .proposal_review
                            .as_mut()
                            .unwrap()
                            .scroll_horizontal(false),
                        _ => {}
                    }
                } else if let Some(menu) = &mut self.actions {
                    match key.code {
                        KeyCode::Up => menu.move_selection(false),
                        KeyCode::Down => menu.move_selection(true),
                        KeyCode::Enter => {
                            let action = menu.selected();
                            self.actions = None;
                            return self.perform_action(action);
                        }
                        _ => {}
                    }
                } else if key.code == KeyCode::Enter {
                    self.finish_export();
                } else if let Some(dialog) = &mut self.export {
                    dialog.input.input(key);
                    dialog.edited();
                }
            }
            Event::Paste(text) => {
                if let Some(dialog) = &mut self.export {
                    dialog.edited();
                    dialog.input.insert_str(text.replace(['\n', '\r'], ""));
                }
            }
            Event::Mouse(mouse) if mouse.kind == MouseEventKind::Down(MouseButton::Left) => {
                if let Some(action) = self
                    .actions
                    .as_mut()
                    .and_then(|menu| menu.click(area, mouse.column, mouse.row))
                {
                    self.actions = None;
                    return self.perform_action(action);
                }
            }
            _ => {}
        }
        false
    }

    fn perform_action(&mut self, action: Action) -> bool {
        let key = match action {
            Action::Build => 'b',
            Action::Export => 'e',
            Action::Save => 's',
            Action::ToggleProjects => 't',
            Action::Quit => 'q',
            Action::FocusAssistant => 'j',
            Action::ClearChat => {
                self.assistant.reset();
                self.status = "Conversation cleared.".into();
                return false;
            }
            Action::ReviewEdit => {
                if let Some(proposal) = self.assistant.proposal() {
                    self.proposal_review = Some(ProposalReview::new(proposal));
                } else {
                    self.status =
                        "No suggested edit to review. Ask Texo to revise your resume.".into();
                }
                return false;
            }
            Action::ScrollUp | Action::ScrollDown => {
                self.preview.scroll(matches!(action, Action::ScrollUp));
                return false;
            }
            Action::PreviousPage | Action::NextPage => {
                let code = if matches!(action, Action::PreviousPage) {
                    KeyCode::Left
                } else {
                    KeyCode::Right
                };
                self.handle_alt_key(KeyEvent::new(code, KeyModifiers::ALT));
                return false;
            }
        };
        self.handle_key(KeyEvent::new(KeyCode::Char(key), KeyModifiers::CONTROL))
    }

    fn finish_export(&mut self) {
        if self.rendered_source.as_deref() != Some(self.editor.source().as_str()) {
            self.export = None;
            self.status =
                "PDF changed while exporting. Build the current document and try again.".into();
            return;
        }
        let Some(dialog) = &mut self.export else {
            return;
        };
        let Some(source) = &self.rendered_pdf else {
            return;
        };
        match dialog.export(source, &self.editor.path) {
            Ok(Some(path)) => {
                self.status = format!("Exported {}", path.display());
                self.export = None;
                self.refresh_projects();
            }
            Ok(None) => {}
            Err(error) => dialog.message = format!("{error:#}"),
        }
    }

    fn handle_projects_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.projects.move_selection(false),
            KeyCode::Down | KeyCode::Char('j') => self.projects.move_selection(true),
            KeyCode::Enter => self.activate_projects(),
            _ => {}
        }
    }

    fn activate_projects(&mut self) {
        let Some(path) = self.projects.selected_path() else {
            return;
        };
        if path.is_dir() {
            match crate::workspace::project_document(&path) {
                Ok(Some(document)) => self.open_document(document),
                Ok(None) => self.status = format!("No TeX document found in {}.", path.display()),
                Err(error) => self.report_error(error),
            }
        } else {
            self.open_document(path);
        }
    }

    fn refresh_projects(&mut self) {
        if let Err(error) = self.projects.refresh() {
            self.report_error(error);
        }
    }

    fn open_document(&mut self, path: PathBuf) {
        if path == self.editor.path {
            self.focus = Focus::Editor;
            return;
        }
        if self.editor.is_modified() {
            self.status = "Save your changes with Ctrl+S before opening another document.".into();
            return;
        }
        if path.extension().is_none_or(|extension| extension != "tex") {
            self.status =
                "Select a .tex document to edit. Other files are shown for context.".into();
            return;
        }
        match Editor::open(path) {
            Ok(editor) => {
                // Drop joins the cancelled worker before a new document can receive a result.
                self.job = None;
                self.assistant.reset();
                self.proposal_review = None;
                self.editor = editor;
                self.preview.clear();
                self.rendered_source = None;
                self.rendered_pdf = None;
                self.diagnostics = None;
                self.focus = Focus::Editor;
                self.status.clear();
                if !self.editor.is_modified() {
                    self.start_render();
                }
            }
            Err(error) => self.report_error(error),
        }
    }

    fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();
        frame.render_widget(Block::default().style(theme::text()), area);

        let layout = WorkspaceLayout::new(
            area,
            self.diagnostics.is_some(),
            self.projects_visible,
            self.focus,
        );
        if self.projects_visible {
            self.projects.draw(
                frame,
                layout.projects,
                &self.editor.path,
                self.focus == Focus::Projects,
            );
        }
        let editor = draw_pane_border(frame, layout.editor, self.focus == Focus::Editor);
        let assistant = draw_pane_border(frame, layout.assistant, self.focus == Focus::Assistant);
        let preview = draw_pane_border(frame, layout.preview, self.focus == Focus::Preview);

        self.draw_editor(frame, editor);
        self.assistant
            .draw(frame, assistant, self.focus == Focus::Assistant);
        self.draw_preview(frame, preview);
        // The editor and chat share a border row; keep the active editor's edge visible.
        if self.focus == Focus::Editor {
            draw_pane_border(frame, layout.editor, true);
        }
        self.draw_diagnostics(frame, &layout);
        if let Some(menu) = &mut self.actions {
            menu.draw(frame);
        }
        if let Some(export) = &mut self.export {
            export.draw(frame);
        }
        if let Some(review) = &mut self.proposal_review {
            review.draw(frame);
        }
    }

    fn draw_editor(&mut self, frame: &mut Frame, area: Rect) {
        self.editor.draw(frame, area, self.focus == Focus::Editor);
        if !area.is_empty() {
            let modified = if self.editor.is_modified() {
                " · unsaved"
            } else {
                ""
            };
            let label = if self.status.is_empty() {
                format!(
                    "{}{}",
                    crate::workspace::display_path(&self.editor.path),
                    modified
                )
            } else {
                self.status.clone()
            };
            let title = Rect::new(
                area.x + theme::PANEL_PADDING,
                area.y,
                area.width.saturating_sub(theme::PANEL_PADDING * 2),
                1,
            );
            frame.render_widget(
                Paragraph::new(label).style(theme::pane_title(self.focus == Focus::Editor)),
                title,
            );
        }
    }

    fn draw_preview(&mut self, frame: &mut Frame, area: Rect) {
        if let Some(job) = &self.job {
            Preview::draw_loading(frame, area, job.elapsed());
        } else {
            self.preview.draw(frame, area);
        }
        if !area.is_empty() && self.job.is_none() {
            let state = match &self.rendered_source {
                Some(source) if *source != self.editor.source() => "PDF out of date",
                Some(_) => "PDF",
                None => "Preview",
            };
            let page = self.preview.page_status().unwrap_or_default();
            let quality = if self.preview.uses_basic_rendering() {
                " · basic preview"
            } else {
                ""
            };
            let title = Rect::new(
                area.x + theme::PANEL_PADDING,
                area.y,
                area.width.saturating_sub(theme::PANEL_PADDING * 2),
                1,
            );
            frame.render_widget(
                Paragraph::new(format!("{state}  {page}{quality}"))
                    .style(theme::pane_title(self.focus == Focus::Preview)),
                title,
            );
        }
    }

    fn draw_diagnostics(&self, frame: &mut Frame, layout: &WorkspaceLayout) {
        if let Some(diagnostics) = &self.diagnostics {
            let log = Paragraph::new(diagnostics.as_str())
                .block(Block::default().title(" Build diagnostics · PgUp/PgDn scroll · Esc close "))
                .style(Style::default().fg(theme::ERROR).bg(theme::ELEVATED))
                .wrap(Wrap { trim: false })
                .scroll((self.diagnostic_scroll, 0));
            frame.render_widget(log, layout.diagnostics);
        }
    }
}

fn pane_content(area: Rect) -> Rect {
    Block::bordered().inner(area)
}

fn draw_pane_border(frame: &mut Frame, area: Rect, focused: bool) -> Rect {
    let color = if focused {
        theme::ACCENT
    } else {
        theme::DIVIDER
    };
    frame.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(color)),
        area,
    );
    pane_content(area)
}

#[derive(Default)]
struct WorkspaceLayout {
    projects: Rect,
    editor: Rect,
    assistant: Rect,
    preview: Rect,
    diagnostics: Rect,
}

impl WorkspaceLayout {
    fn new(area: Rect, show_diagnostics: bool, show_projects: bool, focus: Focus) -> Self {
        let diagnostics_height = if show_diagnostics {
            DIAGNOSTICS_HEIGHT.min(area.height / 2)
        } else {
            0
        };
        let [body, diagnostics] =
            Layout::vertical([Constraint::Min(0), Constraint::Length(diagnostics_height)])
                .areas(area);
        if area.width < MULTIPANE_WIDTH || area.height < MULTIPANE_HEIGHT {
            let mut layout = Self {
                diagnostics,
                ..Self::default()
            };
            match focus {
                Focus::Editor => layout.editor = body,
                Focus::Assistant => layout.assistant = body,
                Focus::Projects => layout.projects = body,
                Focus::Preview => layout.preview = body,
            }
            return layout;
        }
        let sidebar_width = if show_projects { SIDEBAR_WIDTH } else { 0 };
        let [projects, center, preview] = Layout::horizontal([
            Constraint::Length(sidebar_width),
            Constraint::Min(1),
            Constraint::Percentage(PREVIEW_PERCENT),
        ])
        .areas(body);
        let [editor, assistant] =
            Layout::vertical([Constraint::Percentage(EDITOR_PERCENT), Constraint::Min(1)])
                .spacing(Spacing::Overlap(1))
                .areas(center);
        Self {
            projects,
            editor,
            assistant,
            preview,
            diagnostics,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    fn application() -> anyhow::Result<(tempfile::TempDir, App)> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("resume.tex");
        std::fs::write(&path, "Original document")?;
        let editor = Editor::open(path)?;
        Ok((directory, App::new(editor, Preview::default())?))
    }

    #[test]
    fn assistant_edits_refresh_buffer_and_preserve_focus() -> anyhow::Result<()> {
        let (_directory, mut app) = application()?;
        app.editor.text.insert_str("Unsaved ");
        let original = app.editor.source();
        app.rendered_source = Some(original.clone());
        app.focus = Focus::Assistant;
        let edit = ProposedEdit {
            source_before: original.clone(),
            source_after: "Revised resume".into(),
        };
        assert!(app.apply_edit(&edit));
        assert_eq!(app.editor.source(), "Revised resume");
        assert!(app.editor.is_modified());
        assert!(app.focus == Focus::Assistant);
        assert_eq!(
            std::fs::read_to_string(&app.editor.path)?,
            "Original document"
        );

        let mut terminal = Terminal::new(TestBackend::new(160, 50))?;
        terminal.draw(|frame| app.draw(frame))?;
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains("Revised resume"));
        assert!(text.contains("PDF out of date"));
        app.editor.text.undo();
        app.editor.text.undo();
        assert_eq!(app.editor.source(), original);
        Ok(())
    }

    #[test]
    fn stale_and_unchanged_assistant_edits_preserve_buffer() -> anyhow::Result<()> {
        let (_directory, mut app) = application()?;
        let original = app.editor.source();
        let mut edit = ProposedEdit {
            source_before: original.clone(),
            source_after: original.clone(),
        };
        assert!(app.apply_edit(&edit));
        assert!(!app.editor.is_modified());
        assert!(app.status.contains("unchanged"));

        app.editor.text.insert_str("Newer typing ");
        let newer = app.editor.source();
        edit.source_after = "AI revision".into();
        assert!(!app.apply_edit(&edit));
        assert_eq!(app.editor.source(), newer);
        assert!(app.status.contains("not applied"));
        Ok(())
    }

    #[test]
    fn typing_q_edits_and_ctrl_q_quits_immediately() -> anyhow::Result<()> {
        let (_directory, mut app) = application()?;
        assert!(!app.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)));
        assert!(app.editor.source().starts_with('q'));
        let quit = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL);
        assert!(app.handle_key(quit));
        app.actions = Some(ActionMenu::open());
        assert!(app.handle_event(Event::Key(quit), Rect::new(0, 0, 160, 50)));
        Ok(())
    }

    #[test]
    fn preview_focus_keeps_keystrokes_out_of_the_editor() -> anyhow::Result<()> {
        let (_directory, mut app) = application()?;
        let original = app.editor.source();
        app.focus = Focus::Preview;
        app.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
        assert_eq!(app.editor.source(), original);
        app.focus = Focus::Editor;
        app.handle_event(
            Event::Paste("Pasted\ntext".into()),
            Rect::new(0, 0, 100, 30),
        );
        assert!(app.editor.source().starts_with("Pasted\ntext"));
        Ok(())
    }

    #[test]
    fn switching_files_requires_saving_and_clears_the_previous_preview() -> anyhow::Result<()> {
        let (directory, mut app) = application()?;
        let original_path = app.editor.path.clone();
        let second = directory.path().join("second.tex");
        std::fs::write(&second, "Second document")?;
        app.editor.text.insert_str("Unsaved edit ");
        app.open_document(second.clone());
        assert_eq!(app.editor.path, original_path);
        assert!(app.editor.source().contains("Unsaved edit"));

        app.editor.save()?;
        app.rendered_source = Some(app.editor.source());
        app.open_document(second.clone());
        assert_eq!(app.editor.path, second);
        assert_eq!(app.editor.source(), "Second document");
        assert!(app.rendered_source.is_none());
        Ok(())
    }

    #[test]
    fn project_selection_preserves_unsaved_and_missing_documents() -> anyhow::Result<()> {
        let (directory, mut app) = application()?;
        let project = directory.path().join("Backend");
        std::fs::create_dir(&project)?;
        std::fs::write(project.join("main.tex"), "Backend resume")?;
        app.refresh_projects();
        app.editor.text.insert_str("Unsaved ");
        let original_path = app.editor.path.clone();
        let original_source = app.editor.source();
        app.projects.select_path(&project);
        app.activate_projects();
        assert_eq!(app.editor.path, original_path);
        assert_eq!(app.editor.source(), original_source);
        assert!(app.status.contains("Save your changes"));

        let empty = directory.path().join("Empty");
        std::fs::create_dir(&empty)?;
        app.refresh_projects();
        app.projects.select_path(&empty);
        app.activate_projects();
        assert_eq!(app.editor.path, original_path);
        assert!(app.status.contains("No TeX document"));
        assert!(!empty.join("main.tex").exists());
        Ok(())
    }

    #[test]
    #[ignore = "requires Tectonic and Poppler"]
    fn project_selection_replaces_editor_and_preview_during_a_build() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let first_project = directory.path().join("AI engineering");
        let second_project = directory.path().join("Backend");
        std::fs::create_dir(&first_project)?;
        std::fs::create_dir(&second_project)?;
        let first = first_project.join("main.tex");
        let second = second_project.join("main.tex");
        std::fs::write(
            &first,
            "\\documentclass{article}\n\\begin{document}First resume\\end{document}\n",
        )?;
        let second_source = include_str!("../tests/fixtures/two-pages.tex");
        std::fs::write(&second, second_source)?;
        let editor = Editor::open(first)?;
        let mut app =
            App::with_workspace(editor, Preview::default(), directory.path().to_path_buf())?;
        app.start_render();
        finish_test_render(&mut app)?;
        assert_eq!(app.preview.page_status().as_deref(), Some("1 / 1"));

        app.assistant.input.insert_str("Old project question");
        app.start_render();
        assert!(app.job.is_some());
        app.projects.select_path(&second_project);
        app.activate_projects();
        assert_eq!(app.editor.path, second);
        assert_eq!(app.editor.source(), second_source);
        assert!(app.preview.page_status().is_none());
        assert!(app.rendered_pdf.is_none());
        assert_eq!(app.assistant.input.lines(), &[""]);
        assert!(app.job.is_some());
        finish_test_render(&mut app)?;
        assert_eq!(app.preview.page_status().as_deref(), Some("1 / 2"));
        assert_eq!(app.rendered_source.as_deref(), Some(second_source));
        assert_eq!(app.rendered_pdf, Some(second.with_extension("pdf")));
        Ok(())
    }

    fn finish_test_render(app: &mut App) -> anyhow::Result<()> {
        const BUILD_TIMEOUT: Duration = Duration::from_secs(30);
        let started = std::time::Instant::now();
        while app.job.is_some() {
            anyhow::ensure!(started.elapsed() < BUILD_TIMEOUT, "Build did not finish");
            app.receive_render();
            std::thread::sleep(EVENT_POLL_INTERVAL);
        }
        anyhow::ensure!(
            app.diagnostics.is_none(),
            "Build failed: {:?}",
            app.diagnostics
        );
        Ok(())
    }

    #[test]
    fn export_rejects_stale_buffers_and_modal_typing_does_not_edit_source() -> anyhow::Result<()> {
        let (_directory, mut app) = application()?;
        app.rendered_source = Some(app.editor.source());
        app.rendered_pdf = Some(app.editor.path.with_extension("pdf"));
        app.open_export();
        assert!(app.export.is_some());
        let original = app.editor.source();
        app.handle_event(
            Event::Paste("new destination".into()),
            Rect::new(0, 0, 160, 50),
        );
        assert_eq!(app.editor.source(), original);
        app.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            Rect::new(0, 0, 160, 50),
        );
        app.editor.text.insert_str("changed");
        app.open_export();
        assert!(app.export.is_none());
        assert!(app.status.contains("Build the current"));
        Ok(())
    }

    #[test]
    fn assistant_focus_keeps_chat_input_out_of_the_document() -> anyhow::Result<()> {
        let (_directory, mut app) = application()?;
        let original = app.editor.source();
        app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL));
        app.handle_event(
            Event::Paste("Shorten these bullets".into()),
            Rect::new(0, 0, 160, 50),
        );
        assert_eq!(app.editor.source(), original);
        assert_eq!(app.assistant.input.lines(), &["Shorten these bullets"]);
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT));
        assert_eq!(app.assistant.input.lines().len(), 2);
        app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL));
        assert!(app.focus == Focus::Editor);
        Ok(())
    }

    #[test]
    fn export_rechecks_snapshot_before_writing() -> anyhow::Result<()> {
        let (_directory, mut app) = application()?;
        app.rendered_source = Some(app.editor.source());
        app.rendered_pdf = Some(app.editor.path.with_extension("pdf"));
        app.open_export();
        app.rendered_source = Some("A different completed build".into());
        app.finish_export();
        assert!(app.export.is_none());
        assert!(app.status.contains("PDF changed"));

        Ok(())
    }

    #[test]
    fn pane_selection_highlights_only_the_selected_border() -> anyhow::Result<()> {
        let (_directory, mut app) = application()?;
        let area = Rect::new(0, 0, 160, 50);
        let mut terminal = Terminal::new(TestBackend::new(area.width, area.height))?;
        let layout = WorkspaceLayout::new(area, false, true, app.focus);
        let panes = [layout.editor, layout.assistant, layout.preview];
        assert_eq!(layout.preview.x, layout.editor.right());
        assert_eq!(layout.assistant.y + 1, layout.editor.bottom());

        for (selected, pane) in panes.iter().enumerate() {
            // Border clicks select the pane without activating its content controls.
            app.handle_event(
                Event::Mouse(event::MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column: pane.x,
                    row: pane.y,
                    modifiers: KeyModifiers::NONE,
                }),
                area,
            );
            terminal.draw(|frame| app.draw(frame))?;
            let buffer = terminal.backend().buffer();
            let border_row = layout.editor.y + 1;
            assert_eq!(
                buffer[(layout.editor.right() - 1, border_row)].symbol(),
                "│"
            );
            assert_eq!(buffer[(layout.preview.x, border_row)].symbol(), "│");

            for (index, pane) in panes.iter().enumerate() {
                for position in [
                    (pane.x, pane.y),
                    (pane.right() - 1, pane.y),
                    (pane.x, pane.bottom() - 1),
                    (pane.right() - 1, pane.bottom() - 1),
                ] {
                    let shared_edge = index < 2 && position.1 == layout.assistant.y;
                    let highlighted = index == selected || (shared_edge && selected < 2);
                    let color = if highlighted {
                        theme::ACCENT
                    } else {
                        theme::DIVIDER
                    };
                    assert_eq!(buffer[position].fg, color);
                }
            }
        }

        app.focus = Focus::Editor;
        app.handle_key(KeyEvent::new(KeyCode::F(6), KeyModifiers::NONE));
        terminal.draw(|frame| app.draw(frame))?;
        assert_eq!(
            terminal.backend().buffer()[(layout.assistant.x, layout.assistant.y)].fg,
            theme::ACCENT
        );
        assert_eq!(
            terminal.backend().buffer()[(layout.editor.x, layout.editor.y)].fg,
            theme::DIVIDER
        );
        Ok(())
    }

    #[test]
    fn narrow_layout_shows_each_focused_pane_and_tolerates_tiny_sizes() -> anyhow::Result<()> {
        let (_directory, mut app) = application()?;
        for (width, height) in [(1, 1), (20, 5), (40, 12), (80, 24)] {
            for focus in [
                Focus::Editor,
                Focus::Assistant,
                Focus::Projects,
                Focus::Preview,
            ] {
                app.focus = focus;
                let area = Rect::new(0, 0, width, height);
                let layout = WorkspaceLayout::new(area, false, true, focus);
                let visible = match focus {
                    Focus::Editor => layout.editor,
                    Focus::Assistant => layout.assistant,
                    Focus::Projects => layout.projects,
                    Focus::Preview => layout.preview,
                };
                assert_eq!(visible, area);
                let mut terminal = Terminal::new(TestBackend::new(width, height))?;
                terminal.draw(|frame| app.draw(frame))?;
            }
        }
        app.focus = Focus::Editor;
        for expected in [
            Focus::Assistant,
            Focus::Preview,
            Focus::Projects,
            Focus::Editor,
        ] {
            app.handle_key(KeyEvent::new(KeyCode::F(6), KeyModifiers::NONE));
            assert!(app.focus == expected);
        }
        Ok(())
    }

    #[test]
    fn layout_renders_both_panes_and_handles_small_terminals() -> anyhow::Result<()> {
        let (_directory, mut app) = application()?;
        let mut terminal = Terminal::new(TestBackend::new(100, 30))?;
        terminal.draw(|frame| app.draw(frame))?;
        let buffer = terminal.backend().buffer();
        let text: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
        assert!(text.contains("resumes"));
        assert!(text.contains("texo / chat"));
        assert!(text.contains("Original document"));

        let mut small_terminal = Terminal::new(TestBackend::new(30, 8))?;
        small_terminal.draw(|frame| app.draw(frame))?;
        let text: String = small_terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains("Original document"));
        assert!(!text.contains("Resize to at least"));
        Ok(())
    }
}
