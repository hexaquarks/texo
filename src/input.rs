use ratatui::{
    crossterm::event::{
        Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent,
        MouseEventKind,
    },
    layout::{Position, Rect},
};

use super::{
    App, Focus,
    workspace_view::{WorkspaceLayout, below_header, pane_content, preview_header, review_buttons},
};
use crate::{
    actions::{Action, ActionMenu},
    assistant::Assistant,
    keybindings::PrefixInput,
    projects::Projects,
};

const DIAGNOSTIC_SCROLL_LINES: u16 = 4;

impl App {
    pub(super) fn handle_event(&mut self, event: Event, area: Rect) -> bool {
        if !matches!(&event, Event::Mouse(mouse) if mouse.kind == MouseEventKind::Drag(MouseButton::Left))
        {
            self.session.preview.end_drag();
            self.session.proposed_preview.end_drag();
        }

        if let Event::Key(key) = &event {
            if key.kind == KeyEventKind::Release {
                return false;
            }
            if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('q') {
                return true;
            }
        }

        if self.project_dialog.is_some() {
            self.handle_project_dialog(event);
            return false;
        }

        if self.settings.is_some() {
            self.handle_settings_event(event);
            return false;
        }

        if self.session.assistant.login_open() {
            let layout = self.workspace_layout(area);
            self.session
                .assistant
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
            Event::Key(key)
                if self.focus == Focus::Preview
                    && !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                    && matches!(key.code, KeyCode::Char('+' | '=' | '-' | '0')) =>
            {
                let body = below_header(pane_content(self.preview_area(area)), 1);
                if key.code == KeyCode::Char('0') {
                    self.active_preview().reset_zoom(body);
                } else {
                    let center = Position::new(body.x + body.width / 2, body.y + body.height / 2);
                    self.active_preview()
                        .zoom_at(key.code != KeyCode::Char('-'), body, center);
                }
            }
            Event::Key(key) if key.kind != KeyEventKind::Release => return self.handle_key(key),
            Event::Paste(text) if self.focus == Focus::Assistant => {
                self.session.assistant.input.insert_str(text);
            }
            Event::Paste(text)
                if self.focus == Focus::Editor
                    && self.session.proposal_review.is_none()
                    && !self.session.empty_workspace =>
            {
                self.session.editor.text.insert_str(text);
            }
            Event::Mouse(mouse) => self.handle_mouse(mouse, area),
            _ => {}
        }
        false
    }

    fn handle_mouse(&mut self, mouse: MouseEvent, area: Rect) {
        let layout = self.workspace_layout(area);
        let position = Position::new(mouse.column, mouse.row);
        if self.focus == Focus::Preview && mouse.kind == MouseEventKind::Drag(MouseButton::Left) {
            self.active_preview().drag_to(position);
            return;
        }

        if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
            let chat_body = self.assistant_body(pane_content(layout.assistant));
            if Assistant::maximize_button(
                chat_body,
                self.session.chat_maximized,
                &self.prefix.label(),
            )
            .contains(position)
            {
                self.focus = Focus::Assistant;
                self.session.chat_maximized = !self.session.chat_maximized;
                return;
            }

            if self.handle_review_click(position, &layout) {
                return;
            }

            if Projects::new_button(layout.projects).contains(position) {
                self.open_project_dialog(Action::NewResume);
            } else if layout.files.contains(position) {
                self.focus = Focus::Projects;
                self.projects.files_focused = true;
            } else if layout.projects.contains(position) {
                self.focus = Focus::Projects;
                if self.projects.select_at(layout.projects, mouse.row) {
                    self.activate_projects();
                }
            } else if layout.assistant.contains(position) {
                self.focus = Focus::Assistant;
                self.session.assistant.click(
                    self.assistant_body(pane_content(layout.assistant)),
                    mouse.column,
                    mouse.row,
                );
            } else if layout.preview.contains(position) {
                self.focus = Focus::Preview;
                let body = below_header(pane_content(layout.preview), 1);
                self.active_preview().begin_drag(body, position);
            } else if layout.editor.contains(position) {
                self.focus = Focus::Editor;
                if self.session.proposal_review.is_none() {
                    self.session
                        .editor
                        .click(pane_content(layout.editor), mouse.column, mouse.row);
                }
            }
        } else if layout.editor.contains(position) {
            if let Some(review) = &mut self.session.proposal_review {
                match mouse.kind {
                    MouseEventKind::ScrollUp => review.scroll(true),
                    MouseEventKind::ScrollDown => review.scroll(false),
                    _ => {}
                }
            } else {
                self.session.editor.text.input(mouse);
            }
        } else if layout.assistant.contains(position) {
            match mouse.kind {
                MouseEventKind::ScrollDown => self.session.assistant.scroll(false),
                MouseEventKind::ScrollUp => self.session.assistant.scroll(true),
                _ => {}
            }
        } else if layout.preview.contains(position) {
            let body = below_header(pane_content(layout.preview), 1);
            match mouse.kind {
                MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
                    if mouse.modifiers.contains(KeyModifiers::CONTROL) =>
                {
                    self.active_preview().zoom_from_wheel(
                        mouse.kind == MouseEventKind::ScrollUp,
                        body,
                        position,
                    );
                }
                MouseEventKind::ScrollDown => self.active_preview().scroll(false),
                MouseEventKind::ScrollUp => self.active_preview().scroll(true),
                MouseEventKind::ScrollLeft => self.active_preview().pan_horizontal(true),
                MouseEventKind::ScrollRight => self.active_preview().pan_horizontal(false),
                _ => {}
            }
        } else if layout.projects.contains(position)
            && matches!(
                mouse.kind,
                MouseEventKind::ScrollDown | MouseEventKind::ScrollUp
            )
        {
            self.focus = Focus::Projects;
            self.projects.files_focused = false;
            self.projects
                .move_selection(mouse.kind == MouseEventKind::ScrollDown);
            self.activate_projects();
        }
    }

    pub(super) fn handle_prefix_action(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('u') if self.focus == Focus::Assistant => {
                self.session.chat_maximized = !self.session.chat_maximized;
            }
            KeyCode::Char('n') => self.open_project_dialog(Action::NewResume),
            KeyCode::Char('i') => self.open_project_dialog(Action::ImportResume),
            KeyCode::Char('r') => {
                self.projects_visible = true;
                self.focus = Focus::Projects;
                self.projects.files_focused = false;
            }
            KeyCode::Char('f') => {
                self.projects_visible = true;
                self.focus = Focus::Projects;
                self.projects.files_focused = true;
            }
            KeyCode::Char('s' | '?') => self.open_settings(),
            _ => {}
        }
    }

    pub(super) fn handle_review_click(
        &mut self,
        position: Position,
        layout: &WorkspaceLayout,
    ) -> bool {
        if self.session.proposal_review.is_none() {
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
            self.session.show_proposed = proposed.contains(position);
            self.focus = Focus::Preview;
            return true;
        }

        false
    }

    pub(super) fn handle_key(&mut self, key: KeyEvent) -> bool {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('q') {
            return true;
        }

        if self.session.proposal_review.is_some() {
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
                    self.session.show_proposed = false;
                    return false;
                }
                KeyCode::Char(']') if self.focus == Focus::Preview => {
                    self.session.show_proposed = true;
                    return false;
                }
                _ => {}
            }

            if self.focus == Focus::Editor && key.modifiers.is_empty() {
                let review = self.session.proposal_review.as_mut().unwrap();
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

        if self.focus == Focus::Assistant && self.session.assistant.complete_command(key) {
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
            self.session.chat_maximized = false;
        }

        false
    }

    pub(super) fn handle_control_key(&mut self, key: KeyEvent) {
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
            KeyCode::Char('s') if self.session.empty_workspace => {
                self.open_project_dialog(Action::NewResume)
            }
            KeyCode::Char('s')
                if self
                    .projects
                    .project_for(&self.session.editor.path)
                    .is_some()
                    && !self.session.editor.path.exists() =>
            {
                self.open_project_dialog(Action::RecoverSource)
            }
            KeyCode::Char('s') => match self.session.editor.save() {
                Ok(()) => {
                    self.session.status = "Saved TeX document.".into();
                    self.refresh_projects();
                    self.projects.select_path(&self.session.editor.path);
                }
                Err(error) => self.report_error(error),
            },
            KeyCode::Char('z')
                if self.focus == Focus::Editor
                    && self.session.proposal_review.is_none()
                    && !self.session.empty_workspace =>
            {
                self.session.editor.text.undo();
            }
            KeyCode::Char('y')
                if self.focus == Focus::Editor
                    && self.session.proposal_review.is_none()
                    && !self.session.empty_workspace =>
            {
                self.session.editor.text.redo();
            }
            _ if self.focus == Focus::Editor
                && self.session.proposal_review.is_none()
                && !self.session.empty_workspace =>
            {
                self.session.editor.text.input(key);
            }
            _ if self.focus == Focus::Assistant => {
                self.session.assistant.input.input(key);
            }
            _ => {}
        }
    }

    pub(super) fn handle_alt_key(&mut self, key: KeyEvent) {
        if self.focus == Focus::Assistant && key.code == KeyCode::Enter {
            self.session.assistant.input.insert_newline();
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
                if self.focus == Focus::Editor
                    && self.session.proposal_review.is_none()
                    && !self.session.empty_workspace
                {
                    self.session.editor.text.input(key);
                }
                Ok(())
            }
        };
        if let Err(error) = result {
            self.report_error(error);
        }
    }

    pub(super) fn handle_navigation_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Left if self.focus == Focus::Preview => {
                self.active_preview().pan_horizontal(true)
            }
            KeyCode::Right if self.focus == Focus::Preview => {
                self.active_preview().pan_horizontal(false)
            }
            KeyCode::F(5) => self.refresh_projects(),
            KeyCode::F(6) => {
                self.focus = match self.focus {
                    Focus::Editor => Focus::Assistant,
                    Focus::Assistant => Focus::Preview,
                    Focus::Preview if self.projects_visible => Focus::Projects,
                    Focus::Preview | Focus::Projects => Focus::Editor,
                };
            }
            KeyCode::Esc
                if self.focus == Focus::Assistant && self.session.assistant.is_running() =>
            {
                self.session.assistant.cancel()
            }
            KeyCode::Enter if self.focus == Focus::Assistant => self.send_message(),
            KeyCode::PageUp if self.focus == Focus::Assistant => {
                self.session.assistant.scroll(true)
            }
            KeyCode::PageDown if self.focus == Focus::Assistant => {
                self.session.assistant.scroll(false)
            }
            KeyCode::Esc if self.session.diagnostics.is_some() => self.session.diagnostics = None,
            KeyCode::PageUp if self.session.diagnostics.is_some() => {
                self.session.diagnostic_scroll = self
                    .session
                    .diagnostic_scroll
                    .saturating_sub(DIAGNOSTIC_SCROLL_LINES);
            }
            KeyCode::PageDown if self.session.diagnostics.is_some() => {
                self.session.diagnostic_scroll = self
                    .session
                    .diagnostic_scroll
                    .saturating_add(DIAGNOSTIC_SCROLL_LINES);
            }
            _ if self.focus == Focus::Assistant => {
                self.session.assistant.input.input(key);
            }
            _ if self.focus == Focus::Projects => self.handle_projects_key(key),
            _ if self.focus == Focus::Editor
                && self.session.proposal_review.is_none()
                && !self.session.empty_workspace =>
            {
                self.session.editor.text.input(key);
            }
            _ => {}
        }
    }

    pub(super) fn handle_projects_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Tab => self.projects.files_focused = !self.projects.files_focused,
            KeyCode::Up | KeyCode::Char('k') => {
                self.projects.move_selection(false);
                self.activate_projects();
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.projects.move_selection(true);
                self.activate_projects();
            }
            KeyCode::Enter if self.projects.files_focused => self.focus = Focus::Editor,
            KeyCode::Enter => self.activate_projects(),
            _ => {}
        }
    }
}
