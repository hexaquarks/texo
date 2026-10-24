use std::{path::PathBuf, time::Duration};

#[cfg(test)]
use anyhow::Context;

use ratatui::{
    DefaultTerminal, Frame,
    crossterm::{
        SynchronizedUpdate,
        event::{
            self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
        },
    },
    layout::{Constraint, Layout, Position, Rect},
    style::Style,
    widgets::{Block, Borders, Paragraph, Wrap},
};
use texo::{preview::Preview, theme};

use crate::{
    actions::{Action, ActionMenu, ExportDialog},
    assistant::{Assistant, ProposedEdit},
    config::Settings,
    editor::Editor,
    keybindings::{Prefix, PrefixInput},
    projects::Projects,
    proposal::ProposalReview,
    render_job::RenderJob,
    settings::SettingsDialog,
};

const SETTINGS_HINT: &str = "? Settings";

const EVENT_POLL_INTERVAL: Duration = Duration::from_millis(100);
const MULTIPANE_WIDTH: u16 = 100;
const MULTIPANE_HEIGHT: u16 = 20;
const SIDEBAR_WIDTH: u16 = 24;
const PREVIEW_PERCENT: u16 = 38;
const EDITOR_PERCENT: u16 = 55;
// Borders, padding, header, and composer must fit even with the status row visible.
const MIN_CHAT_HEIGHT: u16 = 11;
const DIAGNOSTICS_HEIGHT: u16 = 8;
const DIAGNOSTIC_SCROLL_LINES: u16 = 4;
const REVIEW_CONTROLS_HEIGHT: u16 = 2;
const REVIEW_BUTTON_WIDTH: u16 = 13;
const PREVIEW_TAB_WIDTH: u16 = 13;

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
    settings: Option<SettingsDialog>,
    settings_path: PathBuf,
    prefix: Prefix,
    export: Option<ExportDialog>,
    assistant: Assistant,
    chat_maximized: bool,
    proposal_review: Option<ProposalReview>,
    proposed_preview: Preview,
    proposed_job: Option<RenderJob>,
    proposed_build: Option<texo::sandbox::CompiledDraft>,
    show_proposed: bool,
    status: String,
    diagnostics: Option<String>,
    diagnostic_scroll: u16,
    focus: Focus,
}

pub fn run(terminal: &mut DefaultTerminal, editor: Editor, root: PathBuf) -> anyhow::Result<()> {
    let mut app = App::start(editor, Preview::detect(), root)?;
    app.prefix = Prefix::new(&Settings::load(&app.settings_path)?.keybindings.prefix)?;

    let mut repaint = false;

    loop {
        let was_proposed_loading = app.proposed_job.is_some();
        app.receive_render();
        app.receive_assistant();
        app.receive_proposed_render();
        repaint |= was_proposed_loading != app.proposed_job.is_some();

        // Keep graphics cleanup and the replacement frame invisible until both are complete.
        std::io::stdout().sync_update(|_| -> std::io::Result<()> {
            if repaint {
                terminal.clear()?;
            }
            terminal.draw(|frame| app.draw(frame))?;
            Ok(())
        })??;
        repaint = false;

        if event::poll(EVENT_POLL_INTERVAL)? {
            let area = terminal.get_frame().area();
            let preview_was_loading = app.job.is_some();
            let previous_preview_area = app.preview_area(area);
            let previous_version = app.show_proposed;
            let review_was_open = app.proposal_review.is_some();
            let proposed_was_loading = app.proposed_job.is_some();
            let previous_document = app.editor.path.clone();
            let projects_was_visible = app.projects_visible;
            let overlay_was_open = app.overlay_open();
            if app.handle_event(event::read()?, area) {
                return Ok(());
            }
            if previous_version != app.show_proposed
                || review_was_open != app.proposal_review.is_some()
                || proposed_was_loading != app.proposed_job.is_some()
                || previous_preview_area != app.preview_area(area)
                || projects_was_visible != app.projects_visible
                || overlay_was_open != app.overlay_open()
                || previous_document != app.editor.path
                || preview_was_loading != app.job.is_some()
            {
                // Repaint cells formerly occupied by the terminal's PDF image placement.
                repaint = true;
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
        let proposed_preview = preview.empty_like();
        Ok(Self {
            editor,
            projects,
            projects_visible: true,
            preview,
            job: None,
            rendered_source: None,
            rendered_pdf: None,
            actions: None,
            settings: None,
            settings_path: Settings::path()?,
            prefix: Prefix::default(),
            export: None,
            assistant: Assistant::new(),
            chat_maximized: false,
            proposal_review: None,
            proposed_preview,
            proposed_job: None,
            proposed_build: None,
            show_proposed: false,
            status: String::new(),
            diagnostics: None,
            diagnostic_scroll: 0,
            focus: Focus::Editor,
        })
    }

    fn start(editor: Editor, preview: Preview, root: PathBuf) -> anyhow::Result<Self> {
        let mut app = Self::with_workspace(editor, preview, root)?;

        if !app.editor.is_modified() {
            app.start_render();
        }

        Ok(app)
    }

    fn overlay_open(&self) -> bool {
        self.actions.is_some() || self.export.is_some() || self.settings.is_some()
    }

    fn send_message(&mut self) {
        if self.assistant.input.lines().join("\n").trim() == "/settings" {
            self.assistant.clear_input();
            self.open_settings();
            return;
        }

        let source = self.assistant_source();

        match self.assistant.submit(source, &self.editor.path) {
            Ok(()) => self.status.clear(),
            Err(error) => self.status = error.to_string(),
        }
    }

    fn assistant_source(&self) -> String {
        self.proposal_review
            .as_ref()
            .map(|review| review.edit.source_after.clone())
            .unwrap_or_else(|| self.editor.source())
    }

    fn receive_assistant(&mut self) {
        if let Some(result) = self.assistant.poll() {
            match result {
                Ok(()) if self.assistant.proposal().is_some() => {
                    let mut edit = self.assistant.proposal().cloned().unwrap();

                    // Refinements use the pending draft as context, but review against the original.
                    if let Some(review) = &self.proposal_review {
                        edit.source_before = review.edit.source_before.clone();
                    }

                    self.stage_proposal(edit);
                }
                Ok(()) => self.status = "Assistant replied without changing the document.".into(),
                Err(_) => self.status = "Assistant failed. See its message for details.".into(),
            }
        }
    }

    fn stage_proposal(&mut self, edit: ProposedEdit) {
        if edit.source_before != self.editor.source() {
            self.status = "Proposal is stale: your newer text is preserved.".into();
            self.assistant.take_proposal();
            return;
        }

        if edit.source_before == edit.source_after {
            self.discard_proposal();
            self.status = "No changes proposed.".into();
            return;
        }

        if self.proposal_review.is_none()
            && self.rendered_source.as_deref() != Some(edit.source_before.as_str())
        {
            self.job = None;
            self.start_render();
        }

        self.proposal_review = Some(ProposalReview::new(&edit));
        let build = self.assistant.take_compiled_draft();
        self.assistant.take_proposal();
        self.render_proposal(build);
        self.status = "Review changes · F7 accept · F8 modify · F9 discard".into();
    }

    fn start_proposed_render(&mut self) {
        self.render_proposal(None);
    }

    fn render_proposal(&mut self, build: Option<texo::sandbox::CompiledDraft>) {
        self.proposed_job = None;
        self.proposed_build = None;
        self.proposed_preview.clear();
        self.diagnostics = None;
        self.show_proposed = true;

        let Some(review) = &self.proposal_review else {
            return;
        };

        let source = review.edit.source_after.clone();
        let job = match build {
            Some(build) => RenderJob::compiled(source, build),
            None => RenderJob::draft(source, self.editor.path.clone()),
        };

        match job {
            Ok(job) => self.proposed_job = Some(job),
            Err(error) => self.report_error(error),
        }
    }

    fn receive_proposed_render(&mut self) {
        let Some(result) = self.proposed_job.as_ref().and_then(RenderJob::poll) else {
            return;
        };

        self.proposed_job = None;

        match result {
            Ok(document) => {
                if let Err(error) = self.proposed_preview.set_pages(document.pages) {
                    self.report_error(error);
                    return;
                }

                self.proposed_build = document.build;
                self.show_proposed = true;
            }
            Err(error) => self.report_error(error),
        }
    }

    fn apply_proposal(&mut self) {
        let Some(review) = &self.proposal_review else {
            return;
        };

        if self.assistant.is_running() || self.proposed_job.is_some() {
            self.status = "Wait for the proposed version to finish.".into();
            return;
        }

        if review.edit.source_before != self.editor.source() {
            self.status = "Proposal is stale: discard it to preserve your newer text.".into();
            return;
        }

        let Some(build) = &self.proposed_build else {
            self.status =
                "The proposal must build successfully before accepting. Ctrl+B retries.".into();
            return;
        };
        let edit = review.edit.clone();

        // Stop an older current build before publishing the accepted PDF.
        self.job = None;
        let pdf_path = match texo::compiler::publish_pdf(&build.pdf_path, &self.editor.path) {
            Ok(path) => path,
            Err(error) => {
                self.report_error(error);
                return;
            }
        };

        self.apply_edit(&edit);
        std::mem::swap(&mut self.preview, &mut self.proposed_preview);
        self.rendered_source = Some(edit.source_after);
        self.rendered_pdf = Some(pdf_path);
        self.clear_proposal();
        self.status = "Accepted · PDF updated · Ctrl+S to save TeX".into();
        self.assistant
            .report_edit_result("Changes accepted. PDF updated; Ctrl+S to save TeX.");
    }

    fn modify_proposal(&mut self) {
        if self.proposal_review.is_some() {
            self.focus = Focus::Assistant;
            self.status = "Describe your revisions in chat; the pending draft will be used.".into();
            self.assistant.report_edit_result(
                "Ready to revise. Tell me what to change in the proposed draft.",
            );
        }
    }

    fn discard_proposal(&mut self) {
        self.assistant.cancel();
        self.assistant.take_proposal();
        self.clear_proposal();
        self.status = "Proposal discarded. Original text and preview preserved.".into();
        self.assistant
            .report_edit_result("Proposal discarded; no changes accepted.");
    }

    fn clear_proposal(&mut self) {
        self.proposed_job = None;
        self.proposed_build = None;
        self.proposed_preview.clear();
        self.proposal_review = None;
        self.diagnostics = None;
        self.show_proposed = false;
    }

    fn active_preview(&mut self) -> &mut Preview {
        if self.show_proposed && self.proposal_review.is_some() {
            &mut self.proposed_preview
        } else {
            &mut self.preview
        }
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
        } else {
            self.status = "The returned resume is unchanged.".into();
            self.assistant
                .report_edit_result("No changes applied: the returned text matches the editor.");
        }
        true
    }

    fn start_render(&mut self) {
        if self.proposal_review.is_some() {
            self.start_proposed_render();
            return;
        }

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
                    if self.proposal_review.is_none() {
                        self.status.clear();
                        self.diagnostics = None;
                    }

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
        if let Event::Key(key) = &event {
            if key.kind == KeyEventKind::Release {
                return false;
            }
            if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('q') {
                return true;
            }
        }

        if self.settings.is_some() {
            self.handle_settings_event(event);
            return false;
        }

        if self.assistant.login_open() {
            let layout = self.workspace_layout(area);
            self.assistant
                .handle_login_event(event, pane_content(layout.assistant));
            return false;
        }
        if self.overlay_open() {
            return self.handle_overlay(event, area);
        }

        if let Event::Key(key) = event {
            match self.prefix.handle(key) {
                PrefixInput::Pending => return false,
                PrefixInput::Action(code) => {
                    self.handle_prefix_action(code);
                    return false;
                }
                PrefixInput::Pass => {}
            }

            if key.code == KeyCode::Char('?')
                && !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
            {
                self.open_settings();
                return false;
            }
        } else if !matches!(event, Event::Resize(_, _)) {
            self.prefix.armed = false;
        }

        match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => return self.handle_key(key),
            Event::Paste(text) if self.focus == Focus::Assistant => {
                self.assistant.input.insert_str(text);
            }
            Event::Paste(text) if self.focus == Focus::Editor && self.proposal_review.is_none() => {
                self.editor.text.insert_str(text);
            }
            Event::Mouse(mouse) => {
                let layout = self.workspace_layout(area);
                let position = Position::new(mouse.column, mouse.row);
                if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
                    let chat_body = self.assistant_body(pane_content(layout.assistant));
                    if Assistant::maximize_button(chat_body).contains(position) {
                        self.focus = Focus::Assistant;
                        self.chat_maximized = !self.chat_maximized;
                        return false;
                    }

                    if self.handle_review_click(position, &layout) {
                        return false;
                    }

                    if layout.projects.contains(position) {
                        self.focus = Focus::Projects;
                        if self.projects.select_at(layout.projects, mouse.row) {
                            self.activate_projects();
                        }
                    } else if Assistant::send_button(
                        self.assistant_body(pane_content(layout.assistant)),
                    )
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
                            self.assistant_body(pane_content(layout.assistant)),
                            mouse.column,
                            mouse.row,
                        );
                    } else if layout.preview.contains(position) {
                        self.focus = Focus::Preview;
                    } else if layout.editor.contains(position) {
                        self.focus = Focus::Editor;
                        if self.proposal_review.is_none() {
                            self.editor
                                .click(pane_content(layout.editor), mouse.column, mouse.row);
                        }
                    }
                } else if layout.editor.contains(position) {
                    if let Some(review) = &mut self.proposal_review {
                        match mouse.kind {
                            MouseEventKind::ScrollUp => review.scroll(true),
                            MouseEventKind::ScrollDown => review.scroll(false),
                            _ => {}
                        }
                    } else {
                        self.editor.text.input(mouse);
                    }
                } else if layout.assistant.contains(position) {
                    match mouse.kind {
                        MouseEventKind::ScrollDown => self.assistant.scroll(false),
                        MouseEventKind::ScrollUp => self.assistant.scroll(true),
                        _ => {}
                    }
                } else if layout.preview.contains(position) {
                    match mouse.kind {
                        MouseEventKind::ScrollDown => self.active_preview().scroll(false),
                        MouseEventKind::ScrollUp => self.active_preview().scroll(true),
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

    fn handle_prefix_action(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('u') if self.focus == Focus::Assistant => {
                self.chat_maximized = !self.chat_maximized;
            }
            KeyCode::Char('s' | '?') => self.open_settings(),
            _ => {}
        }
    }

    fn open_settings(&mut self) {
        if self.settings.is_some() {
            return;
        }

        self.actions = None;
        self.export = None;
        self.prefix.armed = false;
        self.settings = Some(SettingsDialog::new(
            self.projects.root(),
            &self.settings_path,
            self.prefix.label(),
        ));
    }

    fn handle_settings_event(&mut self, event: Event) {
        if let Event::Key(key) = event
            && key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('o' | 'r'))
        {
            let result = if key.code == KeyCode::Char('o') {
                self.open_config_with(crate::config::open_editor)
            } else {
                self.reload_settings()
            };
            if let Some(dialog) = &mut self.settings {
                match result {
                    Ok(()) => {
                        dialog.error.clear();
                        dialog.notice = if key.code == KeyCode::Char('o') {
                            "Opened config in your text editor."
                        } else {
                            "Configuration reloaded."
                        }
                        .into();
                    }
                    Err(error) => {
                        dialog.notice.clear();
                        dialog.error = error.to_string();
                    }
                }
            }
            return;
        }

        match event {
            Event::Key(key) if key.code == KeyCode::Esc => self.settings = None,
            Event::Key(key) if key.code == KeyCode::Enter => {
                if let Err(error) = self.apply_settings()
                    && let Some(dialog) = &mut self.settings
                {
                    dialog.error = error.to_string();
                }
            }
            Event::Key(key) => {
                if let Some(dialog) = &mut self.settings {
                    dialog.input.input(key);
                    dialog.error.clear();
                }
            }
            Event::Paste(text) => {
                if let Some(dialog) = &mut self.settings {
                    let text = text.trim();
                    if text.contains(['\r', '\n']) {
                        dialog.error = "Enter a single folder path".into();
                    } else {
                        dialog.input.insert_str(text);
                        dialog.error.clear();
                    }
                }
            }
            _ => {}
        }
    }

    fn open_config_with(
        &self,
        open_file: impl FnOnce(&std::path::Path) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        Settings::ensure_file(&self.settings_path)?;
        open_file(&self.settings_path).map_err(|_| {
            anyhow::anyhow!("Could not open config. Open the path above in a text editor.")
        })
    }

    fn reload_settings(&mut self) -> anyhow::Result<()> {
        let settings = Settings::load(&self.settings_path)?;
        let folder = settings
            .resume_folder
            .as_deref()
            .unwrap_or(std::path::Path::new("~/resumes"));
        let root = crate::config::resume_folder(&folder.to_string_lossy())?;
        let keep_settings_open = self.settings.is_some();
        self.replace_workspace(root, settings, false)?;

        if keep_settings_open {
            self.open_settings();
        }
        Ok(())
    }

    fn apply_settings(&mut self) -> anyhow::Result<()> {
        let Some(dialog) = &self.settings else {
            return Ok(());
        };
        let root = crate::config::resume_folder(&dialog.value())?;
        let mut settings = Settings::load(&self.settings_path)?;
        settings.resume_folder = Some(root.clone());
        self.replace_workspace(root, settings, true)
    }

    fn replace_workspace(
        &mut self,
        root: PathBuf,
        settings: Settings,
        save: bool,
    ) -> anyhow::Result<()> {
        let prefix = Prefix::new(&settings.keybindings.prefix)?;

        // Different spellings or symlinks to the current folder are not a switch.
        let unchanged = self
            .projects
            .root()
            .canonicalize()
            .is_ok_and(|current| current == root);
        if unchanged {
            if save {
                settings.save(&self.settings_path)?;
            }
            self.prefix = prefix;
            self.settings = None;
            return Ok(());
        }

        // Prepare the replacement before discarding anything. Failed reads or saves
        // leave the current editor, proposals, chat, and workers untouched.
        let workspace = crate::workspace::Workspace::resolve(Some(&root))?;
        let editor = Editor::open(workspace.document)?;
        let mut replacement = Self::with_workspace(editor, self.preview.empty_like(), root)?;
        replacement.settings_path = self.settings_path.clone();
        replacement.prefix = prefix;
        if save {
            settings.save(&self.settings_path)?;
        }

        // Join cancelled workers before exposing the replacement so old results
        // cannot reach the new workspace or publish over its preview.
        self.assistant.cancel();
        self.job = None;
        self.proposed_job = None;
        *self = replacement;

        if !self.editor.is_modified() {
            self.start_render();
        }
        Ok(())
    }

    fn handle_review_click(&mut self, position: Position, layout: &WorkspaceLayout) -> bool {
        if self.proposal_review.is_none() {
            return false;
        }

        let buttons = review_buttons(pane_content(layout.assistant));
        if let Some(index) = buttons.iter().position(|button| button.contains(position)) {
            match index {
                0 => self.apply_proposal(),
                1 => self.modify_proposal(),
                _ => self.discard_proposal(),
            }
            return true;
        }

        let [_, current, proposed] = preview_header(pane_content(layout.preview));
        if current.contains(position) || proposed.contains(position) {
            self.show_proposed = proposed.contains(position);
            self.focus = Focus::Preview;
            return true;
        }

        false
    }

    fn handle_key(&mut self, key: KeyEvent) -> bool {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('q') {
            return true;
        }

        if self.proposal_review.is_some() {
            match key.code {
                KeyCode::F(7) => {
                    self.apply_proposal();
                    return false;
                }
                KeyCode::F(8) => {
                    self.modify_proposal();
                    return false;
                }
                KeyCode::F(9) => {
                    self.discard_proposal();
                    return false;
                }
                KeyCode::Char('[') if self.focus == Focus::Preview => {
                    self.show_proposed = false;
                    return false;
                }
                KeyCode::Char(']') if self.focus == Focus::Preview => {
                    self.show_proposed = true;
                    return false;
                }
                _ => {}
            }

            if self.focus == Focus::Editor && key.modifiers.is_empty() {
                let review = self.proposal_review.as_mut().unwrap();
                match key.code {
                    KeyCode::Up | KeyCode::PageUp => review.scroll(true),
                    KeyCode::Down | KeyCode::PageDown => review.scroll(false),
                    KeyCode::Left => review.scroll_horizontal(true),
                    KeyCode::Right => review.scroll_horizontal(false),
                    _ => {}
                }

                if matches!(
                    key.code,
                    KeyCode::Up
                        | KeyCode::Down
                        | KeyCode::Left
                        | KeyCode::Right
                        | KeyCode::PageUp
                        | KeyCode::PageDown
                ) {
                    return false;
                }
            }
        }

        if self.focus == Focus::Assistant && self.assistant.complete_command(key) {
            return false;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            self.handle_control_key(key);
        } else if key.modifiers.contains(KeyModifiers::ALT) {
            self.handle_alt_key(key);
        } else {
            self.handle_navigation_key(key);
        }
        if self.focus == Focus::Editor {
            self.chat_maximized = false;
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
            KeyCode::Char('z') if self.focus == Focus::Editor && self.proposal_review.is_none() => {
                self.editor.text.undo();
            }
            KeyCode::Char('y') if self.focus == Focus::Editor && self.proposal_review.is_none() => {
                self.editor.text.redo();
            }
            _ if self.focus == Focus::Editor && self.proposal_review.is_none() => {
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
            KeyCode::Left => self.active_preview().previous_page(),
            KeyCode::Right => self.active_preview().next_page(),
            KeyCode::Up => {
                self.active_preview().scroll(true);
                Ok(())
            }
            KeyCode::Down => {
                self.active_preview().scroll(false);
                Ok(())
            }
            _ => {
                if self.focus == Focus::Editor && self.proposal_review.is_none() {
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
            _ if self.focus == Focus::Editor && self.proposal_review.is_none() => {
                self.editor.text.input(key);
            }
            _ => {}
        }
    }

    fn open_export(&mut self) {
        if self.proposal_review.is_some() {
            self.status = "Accept or discard the proposal before exporting.".into();
            return;
        }

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
            Action::Settings => {
                self.open_settings();
                return false;
            }
            Action::Build => 'b',
            Action::Export => 'e',
            Action::Save => 's',
            Action::ToggleProjects => 't',
            Action::Quit => 'q',
            Action::FocusAssistant => 'j',
            Action::ClearChat => {
                self.discard_proposal();
                self.assistant.reset();
                self.status = "Conversation cleared.".into();
                return false;
            }
            Action::ReviewEdit => {
                if self.proposal_review.is_some() {
                    self.focus = Focus::Editor;
                } else {
                    self.status = "No proposed edit to review.".into();
                }
                return false;
            }
            Action::ScrollUp | Action::ScrollDown => {
                self.active_preview()
                    .scroll(matches!(action, Action::ScrollUp));
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
                self.clear_proposal();
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

    fn workspace_layout(&self, area: Rect) -> WorkspaceLayout {
        let mut layout = WorkspaceLayout::new(
            area,
            self.diagnostics.is_some(),
            self.projects_visible,
            self.focus,
        );

        // Keep the PDF and sidebar in place; chat only takes over the editor's space.
        if self.chat_maximized && self.focus != Focus::Editor && !layout.editor.is_empty() {
            layout.assistant = layout.assistant.union(layout.editor);
            layout.editor = Rect::default();
        }

        layout
    }

    fn preview_area(&self, area: Rect) -> Rect {
        self.workspace_layout(area).preview
    }

    fn draw(&mut self, frame: &mut Frame) {
        // Opening another document can focus the editor without a navigation key.
        if self.focus == Focus::Editor {
            self.chat_maximized = false;
        }

        let area = frame.area();
        frame.render_widget(Block::default().style(theme::text()), area);

        let layout = self.workspace_layout(area);
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
        self.draw_assistant(frame, assistant);
        self.draw_preview(frame, preview);
        self.draw_diagnostics(frame, &layout);

        let settings_hint = if self.prefix.armed {
            if self.focus == Focus::Assistant {
                "Prefix · u Maximize/restore chat · s Settings · Esc Cancel".to_owned()
            } else {
                "Prefix · s Settings · Esc Cancel".to_owned()
            }
        } else {
            format!("{} Prefix · {SETTINGS_HINT} ", self.prefix.label())
        };
        frame.render_widget(
            Paragraph::new(settings_hint)
                .alignment(ratatui::layout::Alignment::Right)
                .style(theme::muted().bg(theme::ELEVATED)),
            layout.status,
        );

        if let Some(menu) = &mut self.actions {
            menu.draw(frame);
        }
        if let Some(export) = &mut self.export {
            export.draw(frame);
        }
        if let Some(settings) = &self.settings {
            settings.draw(frame);
        }
    }

    fn draw_editor(&mut self, frame: &mut Frame, area: Rect) {
        if let Some(review) = &self.proposal_review {
            review.draw(frame, area);
        } else {
            self.editor.draw(frame, area, self.focus == Focus::Editor);
        }
        if !area.is_empty() {
            let modified = if self.editor.is_modified() {
                " · unsaved"
            } else {
                ""
            };
            let label = if self.proposal_review.is_some() {
                format!(
                    "Unapproved changes · {}",
                    crate::workspace::display_path(&self.editor.path)
                )
            } else if self.status.is_empty() {
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

    fn assistant_body(&self, area: Rect) -> Rect {
        if self.proposal_review.is_some() && !self.assistant.login_open() {
            below_header(area, REVIEW_CONTROLS_HEIGHT)
        } else {
            area
        }
    }

    fn draw_assistant(&mut self, frame: &mut Frame, area: Rect) {
        let body = self.assistant_body(area);
        self.assistant.draw(
            frame,
            body,
            self.focus == Focus::Assistant,
            self.chat_maximized,
            &self.prefix.label(),
        );

        if self.proposal_review.is_some() && !self.assistant.login_open() {
            for (button, label) in
                review_buttons(area)
                    .into_iter()
                    .zip(["[F7 Accept]", "[F8 Modify]", "[F9 Discard]"])
            {
                frame.render_widget(Paragraph::new(label).style(theme::pane_title(true)), button);
            }
        }
    }

    fn draw_preview(&mut self, frame: &mut Frame, area: Rect) {
        if area.is_empty() {
            return;
        }

        let body = below_header(area, 1);
        let proposed = self.show_proposed && self.proposal_review.is_some();
        let loading = if proposed {
            self.proposed_job.as_ref()
        } else {
            self.job.as_ref()
        };

        if let Some(job) = loading {
            Preview::draw_loading(frame, body, job.elapsed());
        } else {
            self.active_preview().draw(frame, body);
        }

        let state = if !proposed
            && self
                .rendered_source
                .as_ref()
                .is_some_and(|source| *source != self.editor.source())
        {
            "PDF out of date"
        } else {
            "PDF"
        };
        let page = self.active_preview().page_status().unwrap_or_default();
        let [label, current_tab, proposed_tab] = preview_header(area);
        let title_area = if self.proposal_review.is_some() {
            label
        } else {
            Rect { height: 1, ..area }
        };
        frame.render_widget(
            Paragraph::new(format!(" {state}  {page}"))
                .style(theme::pane_title(self.focus == Focus::Preview)),
            title_area,
        );

        if self.proposal_review.is_some() {
            for (tab, text, selected) in [
                (current_tab, "[ Current ]", !proposed),
                (proposed_tab, "[ Proposed ]", proposed),
            ] {
                let style = if selected {
                    Style::default().fg(theme::BACKGROUND).bg(theme::LAVENDER)
                } else {
                    theme::muted()
                };
                frame.render_widget(Paragraph::new(text).style(style), tab);
            }
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

fn below_header(area: Rect, height: u16) -> Rect {
    let height = height.min(area.height);
    Rect {
        y: area.y + height,
        height: area.height - height,
        ..area
    }
}

fn review_buttons(area: Rect) -> [Rect; 3] {
    let row = Rect {
        x: area.x + theme::PANEL_PADDING.min(area.width),
        y: area.y + area.height.min(1),
        width: area.width.saturating_sub(theme::PANEL_PADDING * 2),
        height: area.height.saturating_sub(1).min(1),
    };

    Layout::horizontal([
        Constraint::Length(REVIEW_BUTTON_WIDTH),
        Constraint::Length(REVIEW_BUTTON_WIDTH),
        Constraint::Length(REVIEW_BUTTON_WIDTH),
    ])
    .areas(row)
}

fn preview_header(area: Rect) -> [Rect; 3] {
    Layout::horizontal([
        Constraint::Min(0),
        Constraint::Length(PREVIEW_TAB_WIDTH),
        Constraint::Length(PREVIEW_TAB_WIDTH),
    ])
    .areas(Rect {
        height: area.height.min(1),
        ..area
    })
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
    status: Rect,
}

impl WorkspaceLayout {
    fn new(area: Rect, show_diagnostics: bool, show_projects: bool, focus: Focus) -> Self {
        let [area, status] =
            Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(area);

        let diagnostics_height = if show_diagnostics {
            DIAGNOSTICS_HEIGHT.min(area.height / 2)
        } else {
            0
        };
        let [body, diagnostics] =
            Layout::vertical([Constraint::Min(0), Constraint::Length(diagnostics_height)])
                .areas(area);
        // A narrow terminal can still fit a vertical workspace. Keep chat visible and
        // use the upper pane for the editor, preview, or projects as focus changes.
        if area.width < MULTIPANE_WIDTH && body.height >= MULTIPANE_HEIGHT {
            let [upper, assistant] = Layout::vertical([
                Constraint::Percentage(EDITOR_PERCENT),
                Constraint::Min(MIN_CHAT_HEIGHT),
            ])
            .areas(body);
            let mut layout = Self {
                assistant,
                diagnostics,
                status,
                ..Self::default()
            };
            match focus {
                Focus::Preview => layout.preview = upper,
                Focus::Projects => layout.projects = upper,
                Focus::Editor | Focus::Assistant => layout.editor = upper,
            }
            return layout;
        }
        if area.width < MULTIPANE_WIDTH || body.height < MULTIPANE_HEIGHT {
            let mut layout = Self {
                diagnostics,
                status,
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
        let [editor, assistant] = Layout::vertical([
            Constraint::Percentage(EDITOR_PERCENT),
            Constraint::Min(MIN_CHAT_HEIGHT),
        ])
        .areas(center);
        Self {
            projects,
            editor,
            assistant,
            preview,
            diagnostics,
            status,
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
    fn chat_maximizes_and_restores_without_changing_documents_or_prompt() -> anyhow::Result<()> {
        let (_directory, mut app) = application()?;
        let source = app.editor.source();
        app.assistant.input.insert_str("Keep this draft");
        let toggle = |app: &mut App, area| {
            for (code, modifiers) in [
                (KeyCode::Char('x'), KeyModifiers::CONTROL),
                (KeyCode::Char('u'), KeyModifiers::NONE),
            ] {
                app.handle_event(Event::Key(KeyEvent::new(code, modifiers)), area);
            }
        };

        toggle(&mut app, Rect::new(0, 0, 160, 45));
        assert!(!app.chat_maximized);
        app.focus = Focus::Assistant;

        for (width, height) in [(160, 45), (80, 30)] {
            let area = Rect::new(0, 0, width, height);
            let original = app.workspace_layout(area);
            toggle(&mut app, area);
            let expanded = app.workspace_layout(area);

            assert!(expanded.editor.is_empty());
            assert_eq!(
                expanded.assistant,
                original.assistant.union(original.editor)
            );
            assert_eq!(expanded.preview, original.preview);
            assert_eq!(expanded.projects, original.projects);

            let mut terminal = Terminal::new(TestBackend::new(width, height))?;
            terminal.draw(|frame| app.draw(frame))?;
            let text: String = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect();
            assert!(text.contains(&format!("Maximized · {} u Restore", app.prefix.label())));

            for (small_width, small_height) in [(1, 1), (30, 10)] {
                let mut small_terminal =
                    Terminal::new(TestBackend::new(small_width, small_height))?;
                small_terminal.draw(|frame| app.draw(frame))?;
            }

            toggle(&mut app, area);
            let restored = app.workspace_layout(area);
            assert_eq!(restored.editor, original.editor);
            assert_eq!(restored.assistant, original.assistant);
            assert_eq!(app.editor.source(), source);
            assert_eq!(app.assistant.input.lines()[0], "Keep this draft");
        }
        Ok(())
    }

    #[test]
    fn chat_toggle_uses_reloaded_prefix_and_cancels_without_typing_action_keys()
    -> anyhow::Result<()> {
        let (directory, mut app) = application()?;
        app.settings_path = directory.path().join("config.toml");
        Settings {
            resume_folder: Some(directory.path().to_path_buf()),
            keybindings: crate::config::Keybindings {
                prefix: "ctrl+a".into(),
            },
        }
        .save(&app.settings_path)?;
        app.reload_settings()?;
        app.focus = Focus::Assistant;
        let area = Rect::new(0, 0, 160, 45);
        let prefix = Event::Key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL));
        let action = Event::Key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::NONE));

        app.handle_event(prefix.clone(), area);
        app.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            area,
        );
        assert!(!app.chat_maximized && !app.prefix.armed);

        app.handle_event(prefix.clone(), area);
        app.handle_event(action.clone(), area);
        assert!(app.chat_maximized);
        assert_eq!(app.assistant.input.lines()[0], "");

        let mut terminal = Terminal::new(TestBackend::new(area.width, area.height))?;
        terminal.draw(|frame| app.draw(frame))?;
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains(&format!("{} u Restore", app.prefix.label())));
        assert!(!text.contains("Alt+Z"));

        app.handle_event(prefix, area);
        app.handle_event(action, area);
        assert!(!app.chat_maximized);
        app.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::ALT));
        assert!(!app.chat_maximized);
        Ok(())
    }

    #[test]
    fn chat_header_click_toggles_and_editor_focus_restores_split() -> anyhow::Result<()> {
        let (_directory, mut app) = application()?;
        let area = Rect::new(0, 0, 160, 45);

        for expected in [true, false, true] {
            let layout = app.workspace_layout(area);
            let button = Assistant::maximize_button(pane_content(layout.assistant));
            app.handle_event(
                Event::Mouse(event::MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column: button.right() - 1,
                    row: button.y,
                    modifiers: KeyModifiers::NONE,
                }),
                area,
            );

            assert_eq!(app.chat_maximized, expected);
            assert!(app.focus == Focus::Assistant);
        }

        app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL));
        assert!(app.focus == Focus::Editor);
        assert!(!app.chat_maximized);
        assert!(!app.workspace_layout(area).editor.is_empty());
        Ok(())
    }

    #[test]
    fn reload_shortcut_keeps_settings_open_for_same_new_and_invalid_folders() -> anyhow::Result<()>
    {
        let (directory, mut app) = application()?;
        app.settings_path = directory.path().join("config.toml");
        let mut settings = Settings {
            resume_folder: Some(directory.path().to_path_buf()),
            ..Settings::default()
        };
        settings.save(&app.settings_path)?;
        app.editor.text.insert_str("unsaved text");
        let source = app.editor.source();
        app.open_settings();
        let area = Rect::new(0, 0, 160, 45);
        let reload = Event::Key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));

        app.settings.as_mut().unwrap().error = "Previous error".into();
        assert!(!app.handle_event(reload.clone(), area));
        let dialog = app.settings.as_ref().expect("Settings stays open");
        assert_eq!(dialog.notice, "Configuration reloaded.");
        assert!(dialog.error.is_empty());
        assert_eq!(app.editor.source(), source);

        let target = directory.path().join("new resumes");
        std::fs::create_dir(&target)?;
        settings.resume_folder = Some(target.clone());
        settings.keybindings.prefix = "ctrl+a".into();
        settings.save(&app.settings_path)?;
        assert!(!app.handle_event(reload.clone(), area));
        let dialog = app
            .settings
            .as_ref()
            .expect("Settings stays open after switching folders");
        assert_eq!(
            dialog.value(),
            crate::workspace::display_path(&target.canonicalize()?)
        );
        assert_eq!(dialog.prefix_label, Prefix::new("ctrl+a")?.label());
        assert_eq!(dialog.notice, "Configuration reloaded.");

        let saved_value = dialog.value();
        std::fs::write(&app.settings_path, "broken TOML")?;
        assert!(!app.handle_event(reload, area));
        let dialog = app.settings.as_ref().expect("Settings stays open on error");
        assert!(!dialog.error.is_empty());
        assert!(dialog.notice.is_empty());
        assert_eq!(dialog.value(), saved_value);
        assert_eq!(app.projects.root(), target.canonicalize()?);
        Ok(())
    }

    #[test]
    fn open_config_failure_has_a_concise_error_and_preserves_the_dialog() -> anyhow::Result<()> {
        let (directory, mut app) = application()?;
        app.settings_path = directory.path().join("config.toml");
        app.open_settings();
        let error = app
            .open_config_with(|_| anyhow::bail!("Launcher details and exit status"))
            .unwrap_err();

        assert_eq!(
            error.to_string(),
            "Could not open config. Open the path above in a text editor."
        );
        assert!(app.settings.is_some());
        assert!(app.settings_path.is_file());
        Ok(())
    }

    #[test]
    fn config_open_reload_and_folder_apply_preserve_prefix_and_reject_invalid_edits()
    -> anyhow::Result<()> {
        let (directory, mut app) = application()?;
        app.settings_path = directory.path().join("config.toml");
        app.open_settings();
        app.open_config_with(|path| {
            assert_eq!(path, app.settings_path);
            assert!(path.is_file());
            Ok(())
        })?;

        let mut settings = Settings::load(&app.settings_path)?;
        settings.resume_folder = Some(directory.path().to_path_buf());
        settings.keybindings.prefix = "ctrl+a".into();
        settings.save(&app.settings_path)?;
        app.reload_settings()?;
        assert_eq!(app.prefix.label(), Prefix::new("ctrl+a")?.label());

        let area = Rect::new(0, 0, 160, 45);
        for (code, modifiers) in [
            (KeyCode::Char('a'), KeyModifiers::CONTROL),
            (KeyCode::Char('s'), KeyModifiers::NONE),
        ] {
            app.handle_event(Event::Key(KeyEvent::new(code, modifiers)), area);
        }
        assert!(app.settings.is_some());

        let target = directory.path().join("new");
        std::fs::create_dir(&target)?;
        app.settings.as_mut().unwrap().input =
            ratatui_textarea::TextArea::new(vec![target.display().to_string()]);
        app.apply_settings()?;
        assert_eq!(
            Settings::load(&app.settings_path)?.keybindings.prefix,
            "ctrl+a"
        );
        assert_eq!(app.prefix.label(), Prefix::new("ctrl+a")?.label());

        app.editor.text.insert_str("unsaved");
        let source = app.editor.source();
        app.open_settings();
        std::fs::write(&app.settings_path, "invalid TOML")?;
        assert!(app.reload_settings().is_err());
        assert_eq!(app.editor.source(), source);
        assert_eq!(app.projects.root(), target.canonicalize()?);
        assert_eq!(app.prefix.label(), Prefix::new("ctrl+a")?.label());
        // The Open action must still work so a broken file can be repaired.
        app.open_config_with(|_| Ok(()))?;
        Ok(())
    }

    #[test]
    fn settings_switch_discards_pending_state_and_refreshes_an_empty_workspace()
    -> anyhow::Result<()> {
        let (directory, mut app) = application()?;
        let original = app.editor.source();
        app.editor.text.insert_str("unsaved");
        app.proposal_review = Some(ProposalReview::new(&ProposedEdit {
            source_before: app.editor.source(),
            source_after: "pending".into(),
        }));
        app.assistant.input.insert_str("old prompt");
        app.show_proposed = true;
        app.diagnostics = Some("old error".into());
        app.settings_path = directory.path().join("config/settings.json");
        let target = directory.path().join("New resumes");
        std::fs::create_dir(&target)?;
        app.open_settings();
        app.settings.as_mut().unwrap().input =
            ratatui_textarea::TextArea::new(vec![target.display().to_string()]);
        app.apply_settings()?;

        assert_eq!(app.projects.root(), target.canonicalize()?);
        assert_eq!(app.editor.path, target.canonicalize()?.join("main.tex"));
        assert!(!app.editor.path.exists());
        assert!(app.proposal_review.is_none() && app.proposed_build.is_none());
        assert!(app.job.is_none() && app.proposed_job.is_none());
        assert!(!app.show_proposed && app.diagnostics.is_none());
        assert!(app.rendered_pdf.is_none() && app.rendered_source.is_none());
        assert!(app.assistant.input.lines()[0].is_empty());
        assert_eq!(
            std::fs::read_to_string(directory.path().join("resume.tex"))?,
            original
        );
        assert_eq!(
            Settings::load(&app.settings_path)?.resume_folder,
            Some(target.canonicalize()?)
        );
        Ok(())
    }

    #[test]
    fn unchanged_folder_preserves_unsaved_text_and_pending_review() -> anyhow::Result<()> {
        let (directory, mut app) = application()?;
        app.settings_path = directory.path().join("settings.json");
        app.editor.text.insert_str("unsaved");
        let source = app.editor.source();
        app.proposal_review = Some(ProposalReview::new(&ProposedEdit {
            source_before: source.clone(),
            source_after: "pending".into(),
        }));
        app.open_settings();
        app.apply_settings()?;

        assert!(app.settings.is_none());
        assert_eq!(app.editor.source(), source);
        assert!(app.proposal_review.is_some());
        assert!(Settings::load(&app.settings_path)?.resume_folder.is_some());
        Ok(())
    }

    #[test]
    fn invalid_or_unsavable_settings_preserve_pending_work() -> anyhow::Result<()> {
        let (directory, mut app) = application()?;
        app.editor.text.insert_str("unsaved");
        let source = app.editor.source();
        app.proposal_review = Some(ProposalReview::new(&ProposedEdit {
            source_before: source.clone(),
            source_after: "pending".into(),
        }));
        let root = app.projects.root().to_path_buf();
        let target = directory.path().join("target");
        std::fs::create_dir(&target)?;
        app.settings_path = directory.path().join("resume.tex/settings.json");
        app.open_settings();

        for path in [
            directory.path().join("missing"),
            directory.path().join("resume.tex"),
            target,
        ] {
            app.settings.as_mut().unwrap().input =
                ratatui_textarea::TextArea::new(vec![path.display().to_string()]);
            assert!(app.apply_settings().is_err());
            assert_eq!(app.editor.source(), source);
            assert_eq!(app.projects.root(), root);
            assert!(app.settings.is_some());
            assert!(app.proposal_review.is_some());
        }
        app.handle_settings_event(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
        assert!(app.settings.is_none());
        assert_eq!(app.editor.source(), source);
        Ok(())
    }

    #[test]
    fn settings_shortcut_command_and_footer_are_available() -> anyhow::Result<()> {
        let (_directory, mut app) = application()?;
        let area = Rect::new(0, 0, 160, 45);
        app.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE)),
            area,
        );
        assert!(app.settings.is_some());
        let mut terminal = Terminal::new(TestBackend::new(area.width, area.height))?;
        terminal.draw(|frame| app.draw(frame))?;
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains("Settings") && text.contains("Resume folder"));
        app.handle_settings_event(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
        app.assistant.input.insert_str("/set");
        app.assistant
            .complete_command(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(app.assistant.input.lines()[0], "/settings");
        app.send_message();
        assert!(app.settings.is_some());
        app.settings = None;
        app.perform_action(Action::Settings);
        assert!(app.settings.is_some());
        app.settings = None;
        terminal.draw(|frame| app.draw(frame))?;
        let footer: String = (0..area.width)
            .map(|x| terminal.backend().buffer()[(x, area.height - 1)].symbol())
            .collect();
        assert!(footer.contains(SETTINGS_HINT));
        assert_eq!(
            terminal.backend().buffer()[(0, area.height - 1)].bg,
            theme::ELEVATED
        );

        let layout = WorkspaceLayout::new(area, false, true, app.focus);
        assert_eq!(layout.preview.bottom(), layout.status.y);
        assert_eq!(layout.assistant.bottom(), layout.status.y);
        assert_eq!(layout.status.height, 1);
        Ok(())
    }

    #[test]
    #[ignore = "requires macOS, cached Tectonic, and Poppler"]
    fn switching_folders_cancels_old_builds_and_displays_the_new_pdf() -> anyhow::Result<()> {
        let (directory, mut app) = application()?;
        app.settings_path = directory.path().join("settings.json");
        let looping = r"\documentclass{article}\begin{document}\loop\iftrue\repeat\end{document}";
        app.job = Some(RenderJob::start(looping.into(), app.editor.path.clone())?);
        app.proposed_job = Some(RenderJob::draft(looping.into(), app.editor.path.clone())?);
        let target = directory.path().join("new");
        std::fs::create_dir(&target)?;
        let source = r"\documentclass{article}\begin{document}New workspace\end{document}";
        std::fs::write(target.join("main.tex"), source)?;
        app.open_settings();
        app.settings.as_mut().unwrap().input =
            ratatui_textarea::TextArea::new(vec![target.display().to_string()]);
        app.apply_settings()?;

        assert!(app.proposed_job.is_none());
        assert_eq!(app.editor.source(), source);
        let mut terminal = Terminal::new(TestBackend::new(160, 45))?;
        wait_for_visible_pdf(&mut app, &mut terminal)?;
        assert!(target.join("main.pdf").is_file());
        assert!(!directory.path().join("resume.pdf").exists());
        Ok(())
    }

    #[test]
    #[ignore = "requires macOS, Tectonic cache, and Poppler"]
    fn startup_displays_pdf_and_manual_build_recovers_after_failure() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("resume.tex");
        let source = r"\documentclass{article}\begin{document}Startup resume\end{document}";
        std::fs::write(&path, source)?;
        let editor = Editor::open(path.clone())?;
        let mut app = App::start(editor, Preview::default(), directory.path().to_path_buf())?;
        let mut terminal = Terminal::new(TestBackend::new(160, 45))?;

        assert!(app.job.is_some(), "Startup must build without a key press");
        wait_for_visible_pdf(&mut app, &mut terminal)?;
        assert!(path.with_extension("pdf").is_file());
        assert_eq!(app.preview.page_status().as_deref(), Some("1 / 1"));

        app.editor.replace_source(
            r"\documentclass{missing-texo-test-class}\begin{document}Invalid\end{document}",
        );
        app.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
        assert!(finish_test_render(&mut app).is_err());
        assert!(
            app.diagnostics.is_some(),
            "Failed builds must show diagnostics"
        );
        terminal.draw(|frame| app.draw(frame))?;
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains("missing-texo-test-class"));

        app.editor.replace_source(source);
        app.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
        wait_for_visible_pdf(&mut app, &mut terminal)?;
        assert!(app.diagnostics.is_none());
        Ok(())
    }

    fn wait_for_visible_pdf(
        app: &mut App,
        terminal: &mut Terminal<TestBackend>,
    ) -> anyhow::Result<()> {
        const PREVIEW_TIMEOUT: Duration = Duration::from_secs(30);
        const PREVIEW_POLL_INTERVAL: Duration = Duration::from_millis(10);
        let started = std::time::Instant::now();

        loop {
            app.receive_render();
            anyhow::ensure!(
                app.diagnostics.is_none(),
                "Build failed: {:?}",
                app.diagnostics
            );
            terminal.draw(|frame| app.draw(frame))?;
            let visible = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .any(|cell| cell.symbol() == "▀");
            if app.job.is_none() && visible {
                return Ok(());
            }

            anyhow::ensure!(
                started.elapsed() < PREVIEW_TIMEOUT,
                "Startup PDF never became visible"
            );
            std::thread::sleep(PREVIEW_POLL_INTERVAL);
        }
    }

    #[test]
    fn focus_changes_only_invalidate_preview_placement_when_layout_changes() -> anyhow::Result<()> {
        let (_directory, mut app) = application()?;
        let wide = Rect::new(0, 0, 160, 45);
        let preview_area = app.preview_area(wide);
        for focus in [
            Focus::Editor,
            Focus::Assistant,
            Focus::Preview,
            Focus::Projects,
        ] {
            app.focus = focus;
            assert_eq!(app.preview_area(wide), preview_area);
        }

        let narrow = Rect::new(0, 0, 80, 45);
        app.focus = Focus::Editor;
        assert!(app.preview_area(narrow).is_empty());
        app.focus = Focus::Preview;
        assert!(!app.preview_area(narrow).is_empty());
        Ok(())
    }

    #[test]
    fn pending_review_protects_the_editor_and_exposes_controls() -> anyhow::Result<()> {
        let (_directory, mut app) = application()?;
        let original = app.editor.source();
        app.proposal_review = Some(ProposalReview::new(&ProposedEdit {
            source_before: original.clone(),
            source_after: "Changed bullet".into(),
        }));
        app.show_proposed = true;

        let mut terminal = Terminal::new(TestBackend::new(160, 45))?;
        terminal.draw(|frame| app.draw(frame))?;
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains("F7 Accept"));
        assert!(text.contains("F8 Modify"));
        assert!(text.contains("F9 Discard"));
        assert!(text.contains("[ Current ]"));
        assert!(text.contains("[ Proposed ]"));
        assert!(text.contains("Changed bullet"));
        assert!(text.contains("Unapproved changes"));
        assert!(!text.contains("Original document"));

        app.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
        app.handle_event(Event::Paste("hidden edit".into()), Rect::new(0, 0, 160, 45));
        assert_eq!(app.editor.source(), original);

        app.apply_proposal();
        assert_eq!(app.editor.source(), original);
        assert!(app.status.contains("build successfully"));

        app.modify_proposal();
        assert!(app.focus == Focus::Assistant);
        assert!(app.proposal_review.is_some());
        assert_eq!(app.assistant_source(), "Changed bullet");

        app.discard_proposal();
        assert!(app.proposal_review.is_none());
        assert!(!app.show_proposed);
        assert_eq!(app.editor.source(), original);
        Ok(())
    }

    #[test]
    fn review_tabs_and_buttons_use_their_visible_hit_areas() -> anyhow::Result<()> {
        let (_directory, mut app) = application()?;
        app.proposal_review = Some(ProposalReview::new(&ProposedEdit {
            source_before: app.editor.source(),
            source_after: "Pending".into(),
        }));
        let area = Rect::new(0, 0, 160, 45);
        let layout = WorkspaceLayout::new(area, false, true, Focus::Editor);
        let [_, current, proposed] = preview_header(pane_content(layout.preview));

        for (tab, expected) in [(proposed, true), (current, false)] {
            app.handle_event(
                Event::Mouse(event::MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column: tab.x,
                    row: tab.y,
                    modifiers: KeyModifiers::NONE,
                }),
                area,
            );
            assert_eq!(app.show_proposed, expected);
        }

        let discard = review_buttons(pane_content(layout.assistant))[2];
        app.handle_event(
            Event::Mouse(event::MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: discard.x,
                row: discard.y,
                modifiers: KeyModifiers::NONE,
            }),
            area,
        );
        assert!(app.proposal_review.is_none());
        Ok(())
    }

    #[test]
    #[ignore = "requires macOS, Tectonic cache, and Poppler"]
    fn review_keeps_original_files_and_promotes_the_cached_pdf_only_on_accept() -> anyhow::Result<()>
    {
        let (directory, mut app) = application()?;
        let original = r"\documentclass{article}\begin{document}Original\end{document}";
        let proposed = r"\documentclass{article}\begin{document}Revised\end{document}";
        app.editor.replace_source(original);
        app.editor.save()?;
        app.start_render();
        wait_for_reviews(&mut app)?;
        let original_pdf = std::fs::read(app.editor.path.with_extension("pdf"))?;

        app.stage_proposal(ProposedEdit {
            source_before: original.into(),
            source_after: proposed.into(),
        });
        wait_for_reviews(&mut app)?;
        assert!(app.show_proposed);
        assert_eq!(app.editor.source(), original);
        assert_eq!(std::fs::read_to_string(&app.editor.path)?, original);
        assert_eq!(
            std::fs::read(app.editor.path.with_extension("pdf"))?,
            original_pdf
        );
        let proposed_pdf = std::fs::read(&app.proposed_build.as_ref().unwrap().pdf_path)?;

        app.show_proposed = false;
        assert!(app.active_preview().page_status().is_some());
        app.show_proposed = true;
        assert!(app.active_preview().page_status().is_some());
        assert!(app.job.is_none() && app.proposed_job.is_none());

        app.apply_proposal();
        assert_eq!(app.editor.source(), proposed);
        assert_eq!(std::fs::read_to_string(&app.editor.path)?, original);
        assert_eq!(
            std::fs::read(app.editor.path.with_extension("pdf"))?,
            proposed_pdf
        );
        assert_eq!(app.rendered_source.as_deref(), Some(proposed));
        assert!(app.proposal_review.is_none());
        assert!(app.job.is_none() && app.proposed_job.is_none());

        // A discarded follow-up leaves the accepted preview and unsaved buffer intact.
        app.stage_proposal(ProposedEdit {
            source_before: proposed.into(),
            source_after: original.into(),
        });
        app.discard_proposal();
        assert_eq!(app.editor.source(), proposed);
        assert_eq!(
            std::fs::read(app.editor.path.with_extension("pdf"))?,
            proposed_pdf
        );
        drop(directory);
        Ok(())
    }

    #[test]
    #[ignore = "requires macOS, Tectonic cache, and Poppler"]
    fn failed_or_stale_proposals_cannot_replace_current_content() -> anyhow::Result<()> {
        let (_directory, mut app) = application()?;
        let original = r"\documentclass{article}\begin{document}Original\end{document}";
        app.editor.replace_source(original);
        app.editor.save()?;
        app.stage_proposal(ProposedEdit {
            source_before: original.into(),
            source_after: r"\undefinedcommand".into(),
        });
        wait_for_reviews(&mut app)?;
        assert!(app.proposed_build.is_none());
        app.apply_proposal();
        assert_eq!(app.editor.source(), original);

        app.editor.text.insert_str("newer ");
        let newer = app.editor.source();
        app.apply_proposal();
        assert_eq!(app.editor.source(), newer);
        assert!(app.status.contains("stale"));
        app.discard_proposal();
        assert_eq!(app.editor.source(), newer);
        Ok(())
    }

    fn wait_for_reviews(app: &mut App) -> anyhow::Result<()> {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while app.job.is_some() || app.proposed_job.is_some() {
            anyhow::ensure!(
                std::time::Instant::now() < deadline,
                "Review build did not finish"
            );
            app.receive_render();
            app.receive_proposed_render();
            std::thread::sleep(Duration::from_millis(10));
        }
        Ok(())
    }

    #[test]
    fn login_captures_typing_paste_and_cancel_without_editing_tex() -> anyhow::Result<()> {
        let (_directory, mut app) = application()?;
        let original = app.editor.source();
        let area = Rect::new(0, 0, 160, 50);
        app.focus = Focus::Assistant;
        app.assistant.input.insert_str("/logi");
        app.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            area,
        );
        assert!(app.assistant.login_open());
        app.handle_event(
            Event::Paste("text should not reach the editor".into()),
            area,
        );
        app.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE)),
            area,
        );
        let mut terminal = Terminal::new(TestBackend::new(area.width, area.height))?;
        terminal.draw(|frame| app.draw(frame))?;
        app.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            area,
        );
        assert!(!app.assistant.login_open());
        assert_eq!(app.editor.source(), original);
        assert!(!app.assistant.is_running());
        Ok(())
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
        assert_eq!(layout.assistant.y, layout.editor.bottom());

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
                let color = if index == selected {
                    theme::ACCENT
                } else {
                    theme::DIVIDER
                };
                for position in [
                    (pane.x, pane.y),
                    (pane.right() - 1, pane.y),
                    (pane.x, pane.bottom() - 1),
                    (pane.right() - 1, pane.bottom() - 1),
                ] {
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
        for (width, height) in [(1, 1), (20, 5), (40, 12)] {
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
                assert_eq!(visible.height, area.height.saturating_sub(1));
                assert_eq!(visible.width, area.width);
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
    fn narrow_workspace_keeps_chat_below_the_upper_pane() -> anyhow::Result<()> {
        let (_directory, mut app) = application()?;
        for (width, height) in [(80, 24), (90, 42)] {
            let area = Rect::new(0, 0, width, height);
            for focus in [
                Focus::Editor,
                Focus::Assistant,
                Focus::Preview,
                Focus::Projects,
            ] {
                app.focus = focus;
                let layout = WorkspaceLayout::new(area, false, true, focus);
                let upper = match focus {
                    Focus::Preview => layout.preview,
                    Focus::Projects => layout.projects,
                    _ => layout.editor,
                };
                assert_eq!(upper.bottom(), layout.assistant.y);
                assert_eq!(layout.assistant.bottom(), layout.status.y);
                assert_eq!(layout.assistant.width, area.width);
                let mut terminal = Terminal::new(TestBackend::new(width, height))?;
                terminal.draw(|frame| app.draw(frame))?;
                let text: String = terminal
                    .backend()
                    .buffer()
                    .content
                    .iter()
                    .map(|cell| cell.symbol())
                    .collect();
                assert!(text.contains("texo / chat"));
                assert!(text.contains("Ask Texo"));
                if matches!(focus, Focus::Editor | Focus::Assistant) {
                    assert!(text.contains("Original document"));
                }
            }
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
        assert!(text.contains(app.projects.root().file_name().unwrap().to_str().unwrap()));
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
