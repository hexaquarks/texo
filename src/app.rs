use std::{path::PathBuf, time::Duration};

#[cfg(test)]
use anyhow::Context;

use ratatui::{
    DefaultTerminal, Frame,
    crossterm::event::{
        self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
    },
    layout::{Constraint, Layout, Position, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Wrap},
};
use texo::{preview::Preview, theme};

use crate::{
    actions::{Action, ActionMenu, ExportDialog},
    assistant::{Assistant, ProposedEdit},
    editor::Editor,
    explorer::Explorer,
    proposal::ProposalReview,
    render_job::RenderJob,
};

const EVENT_POLL_INTERVAL: Duration = Duration::from_millis(100);
const MIN_TERMINAL_WIDTH: u16 = 100;
const MIN_TERMINAL_HEIGHT: u16 = 16;
const TOOLBAR_HEIGHT: u16 = 2;
const FOOTER_HEIGHT: u16 = 2;
const BUILD_BUTTON_WIDTH: u16 = 13;
const EXPORT_BUTTON_WIDTH: u16 = 10;
const ACTIONS_BUTTON_WIDTH: u16 = 16;
const EXPLORER_PERCENT: u16 = 16;
const PREVIEW_PERCENT: u16 = 38;
const EDITOR_PERCENT: u16 = 67;
const DIAGNOSTICS_HEIGHT: u16 = 8;
const DIAGNOSTIC_SCROLL_LINES: u16 = 4;
const PAGE_STATUS_WIDTH: u16 = 22;

#[derive(Clone, Copy, PartialEq)]
enum Focus {
    Explorer,
    Editor,
    Render,
    Export,
    Actions,
    Assistant,
}

struct App {
    editor: Editor,
    explorer: Explorer,
    explorer_visible: bool,
    workspace_path: String,
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
    confirm_quit: bool,
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
            let previous_document = app.editor.path.clone();
            let explorer_was_visible = app.explorer_visible;
            let overlay_was_open = app.overlay_open();
            if app.handle_event(event::read()?, area) {
                return Ok(());
            }
            if explorer_was_visible != app.explorer_visible
                || overlay_was_open != app.overlay_open()
                || previous_document != app.editor.path
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
        let workspace_path = crate::workspace::display_path(&root);
        let explorer = Explorer::new(root, &editor.path)?;
        Ok(Self {
            editor,
            explorer,
            explorer_visible: true,
            workspace_path,
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
            confirm_quit: false,
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
                    self.refresh_explorer();
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
        let confirms_quit = matches!(&event, Event::Key(key)
            if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('q'));
        if !confirms_quit {
            self.confirm_quit = false;
        }
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
                self.confirm_quit = false;
            }
            Event::Mouse(mouse) => {
                let layout =
                    WorkspaceLayout::new(area, self.diagnostics.is_some(), self.explorer_visible);
                let position = Position::new(mouse.column, mouse.row);
                if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
                    self.confirm_quit = false;
                    if layout.render_button.contains(position) {
                        self.focus = Focus::Render;
                        self.start_render();
                    } else if layout.export_button.contains(position) {
                        self.focus = Focus::Export;
                        self.open_export();
                    } else if layout.actions_button.contains(position) {
                        self.focus = Focus::Actions;
                        self.actions = Some(ActionMenu::open());
                    } else if layout.explorer.contains(position) {
                        self.focus = Focus::Explorer;
                        if self.explorer.select_at(layout.explorer, mouse.row) {
                            self.activate_explorer();
                        }
                    } else if Assistant::send_button(layout.assistant).contains(position) {
                        self.focus = Focus::Assistant;
                        if self.assistant.is_running() {
                            self.assistant.cancel();
                        } else {
                            self.send_message();
                        }
                    } else if layout.assistant.contains(position) {
                        self.focus = Focus::Assistant;
                        self.assistant
                            .click(layout.assistant, mouse.column, mouse.row);
                    } else if layout.editor.contains(position) {
                        self.focus = Focus::Editor;
                        self.editor.click(layout.editor, mouse.column, mouse.row);
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
                } else if layout.explorer.contains(position) {
                    match mouse.kind {
                        MouseEventKind::ScrollDown => self.explorer.move_selection(true),
                        MouseEventKind::ScrollUp => self.explorer.move_selection(false),
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
            if !self.editor.is_modified() || self.confirm_quit {
                return true;
            }
            self.confirm_quit = true;
            self.status = "Unsaved changes. Ctrl+Q again to discard, or Ctrl+S to save.".into();
            return false;
        }
        self.confirm_quit = false;

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
                self.explorer_visible = !self.explorer_visible;
                if !self.explorer_visible && self.focus == Focus::Explorer {
                    self.focus = Focus::Editor;
                }
            }
            KeyCode::Char('s') => match self.editor.save() {
                Ok(()) => {
                    self.status = "Saved TeX document.".into();
                    self.refresh_explorer();
                    self.explorer.select_path(&self.editor.path);
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
            KeyCode::F(5) => self.refresh_explorer(),
            KeyCode::F(6) => {
                self.focus = match self.focus {
                    Focus::Editor => Focus::Assistant,
                    Focus::Assistant => Focus::Render,
                    Focus::Render => Focus::Export,
                    Focus::Export => Focus::Actions,
                    Focus::Actions if self.explorer_visible => Focus::Explorer,
                    Focus::Actions | Focus::Explorer => Focus::Editor,
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
            KeyCode::Enter | KeyCode::Char(' ') if self.focus == Focus::Render => {
                self.start_render()
            }
            KeyCode::Enter | KeyCode::Char(' ') if self.focus == Focus::Export => {
                self.open_export()
            }
            KeyCode::Enter | KeyCode::Char(' ') if self.focus == Focus::Actions => {
                self.actions = Some(ActionMenu::open())
            }
            _ if self.focus == Focus::Assistant => {
                self.assistant.input.input(key);
            }
            _ if self.focus == Focus::Explorer => self.handle_explorer_key(key),
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
            Action::ToggleExplorer => 't',
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
                self.refresh_explorer();
            }
            Ok(None) => {}
            Err(error) => dialog.message = format!("{error:#}"),
        }
    }

    fn handle_explorer_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.explorer.move_selection(false),
            KeyCode::Down | KeyCode::Char('j') => self.explorer.move_selection(true),
            KeyCode::Enter => self.activate_explorer(),
            KeyCode::Right => {
                if let Err(error) = self.explorer.expand_selected() {
                    self.report_error(error);
                }
            }
            KeyCode::Left => {
                if let Err(error) = self.explorer.collapse_selected() {
                    self.report_error(error);
                }
            }
            _ => {}
        }
    }

    fn activate_explorer(&mut self) {
        match self.explorer.activate() {
            Ok(Some(path)) if path.is_dir() => match crate::workspace::project_document(&path) {
                Ok(Some(document)) => self.open_document(document),
                Ok(None) => self.status = format!("No TeX document found in {}.", path.display()),
                Err(error) => self.report_error(error),
            },
            Ok(Some(path)) => self.open_document(path),
            Ok(None) => {}
            Err(error) => self.report_error(error),
        }
    }

    fn refresh_explorer(&mut self) {
        if let Err(error) = self.explorer.refresh() {
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
        frame.render_widget(
            Block::default()
                .style(theme::text())
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(theme::DIVIDER)),
            area,
        );

        if area.width < MIN_TERMINAL_WIDTH || area.height < MIN_TERMINAL_HEIGHT {
            frame.render_widget(
                Paragraph::new(format!(
                    "Resize to at least {MIN_TERMINAL_WIDTH}×{MIN_TERMINAL_HEIGHT}. Ctrl+Q quits."
                ))
                .wrap(Wrap { trim: false }),
                area,
            );
            return;
        }

        let layout = WorkspaceLayout::new(area, self.diagnostics.is_some(), self.explorer_visible);
        self.draw_toolbar(frame, &layout);
        if self.explorer_visible {
            self.explorer.draw(
                frame,
                layout.explorer,
                &self.editor.path,
                self.focus == Focus::Explorer,
            );
        }
        frame.render_widget(
            Block::default()
                .borders(Borders::RIGHT)
                .border_style(theme::muted().fg(theme::DIVIDER)),
            layout.center,
        );
        self.editor
            .draw(frame, layout.editor, self.focus == Focus::Editor);
        self.assistant
            .draw(frame, layout.assistant, self.focus == Focus::Assistant);
        self.preview.draw(frame, layout.preview);
        self.draw_footer(frame, &layout);
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

    fn draw_toolbar(&self, frame: &mut Frame, layout: &WorkspaceLayout) {
        let heading = Paragraph::new(Line::from(vec![
            Span::styled(" texo  ", Style::default().fg(theme::ACCENT).bold()),
            Span::styled(self.workspace_path.as_str(), theme::muted()),
        ]))
        .block(
            Block::default()
                .borders(Borders::BOTTOM)
                .border_style(Style::default().fg(theme::DIVIDER)),
        );
        frame.render_widget(heading, layout.heading);

        let build_label = if self.job.is_some() {
            "[Building…]"
        } else {
            "[Build ^B]"
        };
        for (area, label, focused) in [
            (
                layout.render_button,
                build_label,
                self.focus == Focus::Render,
            ),
            (
                layout.export_button,
                "[Export]",
                self.focus == Focus::Export,
            ),
            (
                layout.actions_button,
                "[Actions ^P]",
                self.focus == Focus::Actions,
            ),
        ] {
            let background = if focused {
                theme::SELECTION
            } else {
                theme::BACKGROUND
            };
            frame.render_widget(
                Paragraph::new(label)
                    .centered()
                    .style(Style::default().fg(theme::ACCENT).bg(background))
                    .block(
                        Block::default()
                            .borders(Borders::BOTTOM)
                            .border_style(Style::default().fg(theme::DIVIDER)),
                    ),
                area,
            );
        }
    }

    fn draw_footer(&self, frame: &mut Frame, layout: &WorkspaceLayout) {
        if let Some(diagnostics) = &self.diagnostics {
            let log = Paragraph::new(diagnostics.as_str())
                .block(Block::default().title(" Build diagnostics · PgUp/PgDn scroll · Esc close "))
                .style(Style::default().fg(theme::ERROR).bg(theme::ELEVATED))
                .wrap(Wrap { trim: false })
                .scroll((self.diagnostic_scroll, 0));
            frame.render_widget(log, layout.diagnostics);
        }

        let saved = if self.editor.is_modified() {
            "Unsaved"
        } else {
            "Saved"
        };
        let pdf = match &self.rendered_source {
            Some(source) if *source == self.editor.source() => "PDF up to date",
            Some(_) => "PDF out of date",
            None => "PDF not built",
        };
        let footer = Line::from(vec![
            Span::styled(format!(" {saved}  "), Style::default().fg(theme::ACCENT)),
            Span::styled(format!("│  {pdf}    {}", self.status), theme::muted()),
        ]);
        let [status, pages] =
            Layout::horizontal([Constraint::Min(1), Constraint::Length(PAGE_STATUS_WIDTH)])
                .areas(layout.footer);
        frame.render_widget(
            Paragraph::new(footer).block(
                Block::default()
                    .borders(Borders::TOP)
                    .border_style(Style::default().fg(theme::DIVIDER)),
            ),
            status,
        );
        let page_status = self
            .preview
            .page_status()
            .map(|page| format!("{page}  Alt+←/→ "))
            .unwrap_or_default();
        frame.render_widget(
            Paragraph::new(page_status)
                .right_aligned()
                .style(theme::muted())
                .block(
                    Block::default()
                        .borders(Borders::TOP)
                        .border_style(Style::default().fg(theme::DIVIDER)),
                ),
            pages,
        );
    }
}

struct WorkspaceLayout {
    explorer: Rect,
    heading: Rect,
    render_button: Rect,
    export_button: Rect,
    actions_button: Rect,
    center: Rect,
    editor: Rect,
    assistant: Rect,
    preview: Rect,
    diagnostics: Rect,
    footer: Rect,
}

impl WorkspaceLayout {
    fn new(area: Rect, show_diagnostics: bool, show_explorer: bool) -> Self {
        let border = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded);
        let area = border.inner(area);
        let diagnostics_height = if show_diagnostics {
            DIAGNOSTICS_HEIGHT
        } else {
            0
        };
        let [toolbar, body, diagnostics, footer] = Layout::vertical([
            Constraint::Length(TOOLBAR_HEIGHT),
            Constraint::Min(1),
            Constraint::Length(diagnostics_height),
            Constraint::Length(FOOTER_HEIGHT),
        ])
        .areas(area);
        let [heading, render_button, export_button, actions_button] = Layout::horizontal([
            Constraint::Min(1),
            Constraint::Length(BUILD_BUTTON_WIDTH),
            Constraint::Length(EXPORT_BUTTON_WIDTH),
            Constraint::Length(ACTIONS_BUTTON_WIDTH),
        ])
        .areas(toolbar);
        let sidebar_width = if show_explorer {
            area.width * EXPLORER_PERCENT / 100
        } else {
            0
        };
        let [explorer, center, preview] = Layout::horizontal([
            Constraint::Length(sidebar_width),
            Constraint::Min(1),
            Constraint::Percentage(PREVIEW_PERCENT),
        ])
        .areas(body);

        let center_content = Rect {
            width: center.width.saturating_sub(1),
            ..center
        };
        let [editor, assistant] =
            Layout::vertical([Constraint::Percentage(EDITOR_PERCENT), Constraint::Min(1)])
                .areas(center_content);

        Self {
            explorer,
            heading,
            render_button,
            export_button,
            actions_button,
            center,
            editor,
            assistant,
            preview,
            diagnostics,
            footer,
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
    fn typing_q_edits_and_modified_documents_require_quit_confirmation() -> anyhow::Result<()> {
        let (_directory, mut app) = application()?;
        assert!(!app.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)));
        assert!(app.editor.source().starts_with('q'));
        let quit = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL);
        assert!(!app.handle_key(quit));
        assert!(app.handle_key(quit));
        Ok(())
    }

    #[test]
    fn pane_focus_prevents_button_keystrokes_from_editing_text() -> anyhow::Result<()> {
        let (_directory, mut app) = application()?;
        let original = app.editor.source();
        app.focus = Focus::Render;
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
        app.refresh_explorer();
        app.editor.text.insert_str("Unsaved ");
        let original_path = app.editor.path.clone();
        let original_source = app.editor.source();
        app.explorer.select_path(&project);
        app.activate_explorer();
        assert_eq!(app.editor.path, original_path);
        assert_eq!(app.editor.source(), original_source);
        assert!(app.status.contains("Save your changes"));

        let empty = directory.path().join("Empty");
        std::fs::create_dir(&empty)?;
        app.refresh_explorer();
        app.explorer.select_path(&empty);
        app.activate_explorer();
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
        app.explorer.select_path(&second_project);
        app.activate_explorer();
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
    fn export_rechecks_snapshot_and_modal_input_disarms_quit() -> anyhow::Result<()> {
        let (_directory, mut app) = application()?;
        app.rendered_source = Some(app.editor.source());
        app.rendered_pdf = Some(app.editor.path.with_extension("pdf"));
        app.open_export();
        app.rendered_source = Some("A different completed build".into());
        app.finish_export();
        assert!(app.export.is_none());
        assert!(app.status.contains("PDF changed"));

        app.actions = Some(ActionMenu::open());
        app.confirm_quit = true;
        app.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
            Rect::new(0, 0, 160, 50),
        );
        assert!(!app.confirm_quit);
        Ok(())
    }

    #[test]
    fn layout_renders_both_panes_and_handles_small_terminals() -> anyhow::Result<()> {
        let (_directory, mut app) = application()?;
        let mut terminal = Terminal::new(TestBackend::new(100, 30))?;
        terminal.draw(|frame| app.draw(frame))?;
        let buffer = terminal.backend().buffer();
        let text: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
        assert!(text.contains("[Build ^B]"));
        assert!(text.contains("ASSISTANT"));
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
        assert!(text.contains("Resize to at least"));
        Ok(())
    }
}
