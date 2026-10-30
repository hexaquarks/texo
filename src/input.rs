use ratatui::{
    crossterm::event::{
        Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent,
        MouseEventKind,
    },
    layout::{Position, Rect},
};

use super::{
    App, Focus,
    workspace_view::{WorkspaceLayout, below_header, pane_content, preview_header},
};
use crate::{actions::Action, assistant::Assistant, keybindings::PrefixInput, projects::Projects};

impl App {
    pub(super) fn handle_event(&mut self, event: Event, area: Rect) -> bool {
        let previous_focus = self.focus;
        let had_overlay = self.overlay_open() || self.session.assistant.login_open();

        // Escape must reach the editor so leaving insert mode can finish a repeatable edit.
        if matches!(&event, Event::FocusLost) {
            self.session.editor.cancel_pending_commands();
        }

        let quit = self.route_event(event, area);
        if self.focus != previous_focus {
            self.workspace_expansion = None;
        }
        self.restore_hidden_column_pane();
        let has_overlay = self.overlay_open() || self.session.assistant.login_open();
        if self.focus != previous_focus || (has_overlay && !had_overlay) {
            self.session.editor.cancel_pending_commands();
        }

        quit
    }

    fn route_event(&mut self, event: Event, area: Rect) -> bool {
        let continues_pointer_gesture = matches!(&event, Event::Mouse(mouse) if matches!(
            mouse.kind,
            MouseEventKind::Drag(MouseButton::Left) | MouseEventKind::Up(MouseButton::Left)
        ));
        if !continues_pointer_gesture {
            self.session.preview.end_drag();
            self.session.proposed_preview.end_drag();
            self.session.editor.end_drag();
        }

        if let Event::Key(key) = &event {
            if key.kind == KeyEventKind::Release {
                return false;
            }
            if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('q') {
                return true;
            }
        }

        if self.shortcuts_open {
            self.handle_shortcut_help(event, area);
            return false;
        }

        if self.diagnostic_details_open() {
            self.handle_diagnostic_details(event, area);
            return false;
        }

        if self.label_dialog.is_some() {
            self.handle_label_event(event, area);
            return false;
        }

        if self.project_dialog.is_some() {
            self.handle_project_dialog(event, area);
            return false;
        }

        if self.settings.is_some() {
            self.handle_settings_event(event, area);
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
                PrefixInput::Pending => {
                    self.session.editor.cancel_pending_commands();
                    return false;
                }
                PrefixInput::Action(code) => {
                    self.session.editor.cancel_pending_commands();
                    self.handle_prefix_action(code, area);
                    return false;
                }
                PrefixInput::Pass => {}
            }

            if !matches!(self.focus, Focus::Editor | Focus::Assistant)
                && key.code == KeyCode::Char('?')
                && !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
            {
                self.open_settings();
                return false;
            }
        } else if !matches!(event, Event::Resize(_, _)) {
            self.prefix.cancel();
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
                self.session.editor.paste_text(&text);
            }
            Event::Mouse(mouse) => self.handle_mouse(mouse, area),
            _ => {}
        }
        false
    }

    fn open_preview_link(&mut self, uri: &str) {
        #[cfg(not(test))]
        if open::that_detached(uri).is_err() {
            self.session.status = "Could not open PDF link.".into();
        }

        #[cfg(test)]
        {
            self.session.status = format!("Opened PDF link: {uri}");
        }
    }

    fn handle_mouse(&mut self, mouse: MouseEvent, area: Rect) {
        let layout = self.workspace_layout(area);
        let position = Position::new(mouse.column, mouse.row);
        if self.focus == Focus::Preview && mouse.kind == MouseEventKind::Drag(MouseButton::Left) {
            self.active_preview().drag_to(position);
            return;
        }

        if mouse.kind == MouseEventKind::Up(MouseButton::Left) {
            let body = below_header(pane_content(layout.preview), 1);
            let uri = self.active_preview().release_link(body, position);
            if let Some(uri) = uri {
                self.open_preview_link(&uri);
            }

            self.session.preview.end_drag();
            self.session.proposed_preview.end_drag();
        }

        if self.focus == Focus::Editor
            && self.session.editor.is_dragging()
            && self.session.proposal_review.is_none()
            && matches!(
                mouse.kind,
                MouseEventKind::Drag(MouseButton::Left) | MouseEventKind::Up(MouseButton::Left)
            )
        {
            self.session
                .editor
                .handle_mouse(self.editor_text_area(&layout), mouse);
            return;
        }

        if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
            if layout.diagnostics.contains(position) && self.session.build_diagnostics.is_some() {
                if let Some((_, key, _)) = self
                    .problem_action_areas(layout.diagnostics)
                    .into_iter()
                    .find(|(rect, _, _)| rect.contains(position))
                {
                    self.handle_prefix_action(KeyCode::Char(key), area);
                } else {
                    self.go_to_problem();
                }
                return;
            }

            if self
                .editor_maximize_button(self.editor_text_area(&layout))
                .contains(position)
            {
                self.focus = Focus::Editor;
                self.toggle_column_expansion();
                return;
            }

            let chat_body = pane_content(layout.assistant);
            if Assistant::maximize_button(
                chat_body,
                self.pane_is_expanded(Focus::Assistant),
                &self.prefix.label(),
            )
            .contains(position)
            {
                self.focus = Focus::Assistant;
                self.toggle_column_expansion();
                return;
            }

            if self.handle_review_click(position, &layout) {
                return;
            }

            if self
                .projects
                .manage_button(layout.projects)
                .contains(position)
            {
                self.open_resume_menu();
            } else if Projects::new_button(layout.projects).contains(position) {
                self.open_project_dialog(Action::NewResume);
            } else if layout.files.contains(position) {
                self.focus = Focus::Projects;
                self.projects.files_focused = true;
                if self.projects.select_file_at(layout.files, mouse.row) {
                    self.activate_projects();
                }
            } else if layout.projects.contains(position) {
                self.focus = Focus::Projects;
                if self.projects.select_at(layout.projects, mouse.row) {
                    self.activate_projects();
                }
            } else if layout.assistant.contains(position) {
                self.focus = Focus::Assistant;
                self.session.assistant.click(
                    pane_content(layout.assistant),
                    mouse.column,
                    mouse.row,
                );
            } else if layout.preview.contains(position) {
                self.focus = Focus::Preview;
                let body = below_header(pane_content(layout.preview), 1);
                self.active_preview().begin_drag(body, position);
            } else if layout.editor.contains(position) {
                self.focus = Focus::Editor;
                if self.session.proposal_review.is_none() && !self.session.empty_workspace {
                    self.session
                        .editor
                        .handle_mouse(self.editor_text_area(&layout), mouse);
                }
            }
        } else if layout.editor.contains(position) {
            if let Some(review) = &mut self.session.proposal_review {
                match mouse.kind {
                    MouseEventKind::ScrollUp => review.scroll(true),
                    MouseEventKind::ScrollDown => review.scroll(false),
                    _ => {}
                }
            } else if self.focus == Focus::Editor && !self.session.empty_workspace {
                self.session
                    .editor
                    .handle_mouse(self.editor_text_area(&layout), mouse);
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

    pub(super) fn handle_prefix_action(&mut self, code: KeyCode, area: Rect) {
        match code {
            KeyCode::Char(character @ ('h' | 'j' | 'k' | 'l')) => {
                self.select_directional_pane(character, area);
            }
            KeyCode::Char(character @ ('H' | 'J' | 'K' | 'L')) => {
                self.resize_directional_pane(character, area);
            }
            KeyCode::Char('z') => self.toggle_workspace_expansion(),
            KeyCode::Char('u') => {
                self.toggle_column_expansion();
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
            KeyCode::Char('b') => self.start_render(),
            KeyCode::Char('e') => self.open_export(),
            KeyCode::Char('v') => self.refresh_projects(),
            KeyCode::Char('w') => {
                self.handle_control_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL))
            }
            KeyCode::Char('t') => {
                self.handle_control_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL))
            }
            KeyCode::Char('D') if self.session.build_diagnostics.is_some() => {
                self.toggle_problem_summary()
            }
            KeyCode::Char('G') if self.session.diagnostics.is_some() => self.go_to_problem(),
            KeyCode::Char('F') if self.session.diagnostics.is_some() => self.explain_problem(),
            KeyCode::Char('R') if self.session.diagnostics.is_some() => self.recover_resource(),
            KeyCode::Char('E') if self.session.diagnostics.is_some() => {
                self.session.details_open = true;
                self.session.diagnostic_scroll = 0;
            }
            KeyCode::Char(character @ ('[' | ']')) if self.session.build_diagnostics.is_some() => {
                self.select_problem(character == ']');
            }
            KeyCode::Char('a') => self.apply_proposal(),
            KeyCode::Char('m') => self.modify_proposal(),
            KeyCode::Char('d') if self.session.proposal_review.is_some() => self.discard_proposal(),
            KeyCode::Char('c') => {
                self.focus = match self.focus {
                    Focus::Editor => Focus::Assistant,
                    Focus::Assistant => Focus::Preview,
                    Focus::Preview if self.projects_visible => Focus::Projects,
                    Focus::Preview | Focus::Projects => Focus::Editor,
                };
            }
            KeyCode::Char('s') => self.open_settings(),
            KeyCode::Char('?') => {
                self.shortcuts_open = true;
                self.shortcut_query.clear();
                self.shortcut_searching = false;
                self.shortcut_scroll = 0;
            }
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

        let buttons = if self.session.assistant.login_open() {
            [Rect::default(); 3]
        } else {
            self.review_buttons(pane_content(layout.assistant))
        };
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

        if let Some(review) = &mut self.session.proposal_review {
            match key.code {
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

        if self.focus == Focus::Editor
            && !(key.modifiers == KeyModifiers::CONTROL && key.code == KeyCode::Char('s'))
        {
            if self.session.proposal_review.is_none() && !self.session.empty_workspace {
                self.session.editor.handle_key(key);
            }

            self.restore_hidden_column_pane();
            return false;
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
        self.restore_hidden_column_pane();

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
                Ok(warning) => {
                    self.session.status = warning.unwrap_or_else(|| "Saved TeX document.".into());
                    self.refresh_projects();
                    self.projects.select_path(&self.session.editor.path);
                }
                Err(error) => self.report_error(error),
            },
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
        if self.focus != Focus::Preview {
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
            _ => Ok(()),
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
            _ if self.focus == Focus::Assistant => {
                self.session.assistant.input.input(key);
            }
            _ if self.focus == Focus::Projects => self.handle_projects_key(key),
            _ => {}
        }
    }

    pub(super) fn handle_projects_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('m') if !self.projects.files_focused => self.open_resume_menu(),
            KeyCode::Char('l') if !self.projects.files_focused => self.open_label_dialog(),
            KeyCode::Tab => self.projects.files_focused = !self.projects.files_focused,
            KeyCode::Up | KeyCode::Char('k') => {
                self.projects.move_selection(false);
                self.activate_projects();
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.projects.move_selection(true);
                self.activate_projects();
            }
            KeyCode::Enter if self.projects.files_focused => {
                self.activate_projects();
                self.focus = Focus::Editor;
            }
            KeyCode::Enter => self.activate_projects(),
            _ => {}
        }
    }
}
