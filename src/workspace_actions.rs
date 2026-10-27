use std::path::PathBuf;

use ratatui::{
    crossterm::event::{
        Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
    },
    layout::{Position, Rect},
};

use super::{App, Focus, resume_session::ResumeSession};
use crate::{
    actions::{Action, ActionMenu, ExportDialog},
    config::Settings,
    editor::Editor,
    keybindings::Prefix,
    project_dialog::ProjectDialog,
    projects::Projects,
    resume_project::{self, ImportPlan},
    settings::SettingsDialog,
};

impl App {
    pub(super) fn open_settings(&mut self) {
        if self.settings.is_some() {
            return;
        }

        self.actions = None;
        self.export = None;
        self.prefix.cancel();
        let mut dialog = SettingsDialog::new(
            self.projects.root(),
            &self.settings_path,
            self.prefix.label(),
        );
        dialog.editor_preferences = self.editor_preferences;
        self.settings = Some(dialog);
    }

    pub(super) fn handle_settings_event(&mut self, event: Event, screen: Rect) {
        if let Event::Key(key) = event
            && key.code == KeyCode::Char('v')
            && key.modifiers == KeyModifiers::CONTROL
        {
            if let Some(dialog) = &mut self.settings {
                dialog.toggle_mode();
            }
            return;
        }

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
            Event::Mouse(mouse) if mouse.kind == MouseEventKind::Down(MouseButton::Left) => {
                if let Some(dialog) = &mut self.settings {
                    dialog.select_mode_at(screen, Position::new(mouse.column, mouse.row));
                }
            }
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

    pub(super) fn open_config_with(
        &self,
        open_file: impl FnOnce(&std::path::Path) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        Settings::ensure_file(&self.settings_path)?;
        open_file(&self.settings_path).map_err(|_| {
            anyhow::anyhow!("Could not open config. Open the path above in a text editor.")
        })
    }

    pub(super) fn reload_settings(&mut self) -> anyhow::Result<()> {
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

    pub(super) fn apply_settings(&mut self) -> anyhow::Result<()> {
        let Some(dialog) = &self.settings else {
            return Ok(());
        };
        let root = crate::config::resume_folder(&dialog.value())?;
        let mut settings = Settings::load(&self.settings_path)?;
        settings.resume_folder = Some(root.clone());
        settings.editor = dialog.editor_preferences;
        self.replace_workspace(root, settings, true)
    }

    pub(super) fn replace_workspace(
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
            self.apply_editor_preferences(settings.editor);
            self.settings = None;
            return Ok(());
        }

        // Prepare the replacement before discarding anything. Failed reads or saves
        // leave the current editor, proposals, chat, and workers untouched.
        let workspace = crate::workspace::Workspace::resolve(Some(&root))?;
        let editor = if workspace.empty {
            Editor::placeholder(workspace.document)?
        } else {
            Editor::open(workspace.document)?
        };
        let mut replacement =
            Self::with_workspace(editor, self.session.preview.empty_like(), root)?;
        replacement.session.empty_workspace = workspace.empty;
        replacement.settings_path = self.settings_path.clone();
        replacement.pane_sizes = self.pane_sizes;
        replacement.layout_state_path = self.layout_state_path.clone();
        replacement.prefix = prefix;
        replacement.apply_editor_preferences(settings.editor);
        if save {
            settings.save(&self.settings_path)?;
        }

        // Join cancelled workers before exposing the replacement so old results
        // cannot reach the new workspace or publish over its preview.
        self.session.assistant.cancel();
        self.session.job = None;
        self.session.proposed_job = None;
        *self = replacement;

        if !self.session.editor.is_modified() {
            self.start_render();
        }
        Ok(())
    }

    pub(super) fn open_export(&mut self) {
        if self.session.proposal_review.is_some() {
            self.session.status = "Accept or discard the proposal before exporting.".into();
            return;
        }

        if self.session.job.is_some() {
            self.session.status = "Wait for the build to finish before exporting.".into();
            return;
        }
        if self.session.rendered_source.as_deref() != Some(self.session.editor.source().as_str())
            || self.session.rendered_pdf.is_none()
        {
            self.session.status = format!(
                "Build the current document with {} b before exporting.",
                self.prefix.label()
            );
            return;
        }
        self.export = Some(ExportDialog::new(&self.session.editor.path));
    }

    pub(super) fn handle_overlay(&mut self, event: Event, area: Rect) -> bool {
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
                        KeyCode::Up | KeyCode::Char('k') => menu.move_selection(false),
                        KeyCode::Down | KeyCode::Char('j') => menu.move_selection(true),
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

    pub(super) fn perform_action(&mut self, action: Action) -> bool {
        let key = match action {
            Action::NewResume
            | Action::ImportResume
            | Action::RenameResume
            | Action::DuplicateResume
            | Action::ReloadSource
            | Action::RecoverSource => {
                self.open_project_dialog(action);
                return false;
            }
            Action::FocusResumes | Action::FocusFiles => {
                self.projects_visible = true;
                self.focus = Focus::Projects;
                self.projects.files_focused = matches!(action, Action::FocusFiles);
                return false;
            }
            Action::WorkspaceWarnings => {
                self.session.diagnostics = Some(self.projects.catalog.warnings.join("\n"));
                return false;
            }
            Action::LabelResume => {
                self.open_label_dialog();
                return false;
            }
            Action::Settings => {
                self.open_settings();
                return false;
            }
            Action::Build => 'b',
            Action::Export => 'e',
            Action::Save => 's',
            Action::ToggleProjects => 't',
            Action::Quit => return true,
            Action::FocusAssistant => 'j',
            Action::ClearChat => {
                self.discard_proposal();
                self.session.assistant.reset();
                self.session.status = "Conversation cleared.".into();
                return false;
            }
            Action::ReviewEdit => {
                if self.session.proposal_review.is_some() {
                    self.focus = Focus::Editor;
                } else {
                    self.session.status = "No proposed edit to review.".into();
                }
                return false;
            }
            Action::ScrollUp | Action::ScrollDown => {
                self.active_preview()
                    .scroll(matches!(action, Action::ScrollUp));
                return false;
            }
            Action::PreviousPage | Action::NextPage => {
                let result = if matches!(action, Action::PreviousPage) {
                    self.active_preview().previous_page()
                } else {
                    self.active_preview().next_page()
                };
                if let Err(error) = result {
                    self.report_error(error);
                }
                return false;
            }
        };
        self.handle_control_key(KeyEvent::new(KeyCode::Char(key), KeyModifiers::CONTROL));
        false
    }

    pub(super) fn finish_export(&mut self) {
        if self.session.rendered_source.as_deref() != Some(self.session.editor.source().as_str()) {
            self.export = None;
            self.session.status =
                "PDF changed while exporting. Build the current document and try again.".into();
            return;
        }
        let Some(dialog) = &mut self.export else {
            return;
        };
        let Some(source) = &self.session.rendered_pdf else {
            return;
        };
        match dialog.export(source, &self.session.editor.path) {
            Ok(Some(path)) => {
                self.session.status = format!("Exported {}", path.display());
                self.export = None;
                self.refresh_projects();
            }
            Ok(None) => {}
            Err(error) => dialog.message = format!("{error:#}"),
        }
    }

    pub(super) fn open_resume_menu(&mut self) {
        self.activate_projects();
        let Some(project) = self.projects.project_for(&self.session.editor.path) else {
            return;
        };
        self.actions = Some(ActionMenu::for_resume(&project.manifest.name));
        self.prefix.cancel();
    }

    pub(super) fn open_project_dialog(&mut self, action: Action) {
        let active = self.projects.project_for(&self.session.editor.path);
        if active.is_none() && !matches!(action, Action::NewResume | Action::ImportResume) {
            self.session.status = "Select a recognized resume first.".into();
            return;
        }

        self.actions = None;
        self.prefix.cancel();
        self.project_dialog = Some(ProjectDialog::new(
            action,
            active,
            self.projects
                .catalog
                .loose_files
                .first()
                .map(PathBuf::as_path),
        ));
    }

    pub(super) fn handle_project_dialog(&mut self, event: Event) {
        let Some(mut dialog) = self.project_dialog.take() else {
            return;
        };
        match event {
            Event::Key(key) if key.code == KeyCode::Esc => return,
            Event::Key(key) if key.code == KeyCode::Enter => {
                match self.finish_project_dialog(&mut dialog) {
                    Ok(true) => return,
                    Ok(false) => {}
                    Err(error) => dialog.error = error.to_string(),
                }
            }
            Event::Key(key)
                if dialog.plan.is_some()
                    && matches!(
                        key.code,
                        KeyCode::Up | KeyCode::Down | KeyCode::Char('j' | 'k')
                    ) =>
            {
                let current = dialog.selection.selected().unwrap_or(0);
                let count = dialog.plan.as_ref().unwrap().candidates.len();
                let next = if matches!(key.code, KeyCode::Up | KeyCode::Char('k')) {
                    current.saturating_sub(1)
                } else {
                    (current + 1).min(count - 1)
                };
                dialog.selection.select(Some(next));
            }
            Event::Key(key)
                if key.code == KeyCode::Tab && matches!(dialog.action, Action::ImportResume) =>
            {
                dialog.source_focused = !dialog.source_focused;
            }
            Event::Key(key) if dialog.plan.is_none() => {
                dialog.input().input(key);
            }
            Event::Paste(text) if dialog.plan.is_none() => {
                dialog.input().insert_str(text.replace(['\n', '\r'], ""));
            }
            _ => {}
        }
        self.project_dialog = Some(dialog);
    }

    pub(super) fn finish_project_dialog(
        &mut self,
        dialog: &mut ProjectDialog,
    ) -> anyhow::Result<bool> {
        let name = dialog.name.lines().join("");
        let root = self.projects.root().to_path_buf();
        let project = match dialog.action {
            Action::NewResume => Some(resume_project::create(&root, &name)?),
            Action::ImportResume => {
                if dialog.plan.is_none() {
                    let source = dialog.source.lines().join("");
                    let path = if let Some(relative) = source.strip_prefix("~/") {
                        directories::BaseDirs::new()
                            .ok_or_else(|| anyhow::anyhow!("Could not locate home"))?
                            .home_dir()
                            .join(relative)
                    } else {
                        PathBuf::from(source.trim())
                    };
                    let plan = ImportPlan::inspect(&path)?;
                    let ambiguous = plan.candidates.len() > 1;
                    dialog.plan = Some(plan);
                    if ambiguous {
                        return Ok(false);
                    }
                }
                let plan = dialog.plan.as_ref().unwrap();
                let entrypoint = &plan.candidates[dialog.selection.selected().unwrap_or(0)];
                Some(plan.import(&root, &name, entrypoint)?)
            }
            Action::RenameResume => {
                let mut project = self
                    .projects
                    .project_for(&self.session.editor.path)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("Select a resume"))?;
                project.rename(&name)?;
                self.refresh_projects();
                None
            }
            Action::DuplicateResume => {
                let project = self
                    .projects
                    .project_for(&self.session.editor.path)
                    .ok_or_else(|| anyhow::anyhow!("Select a resume"))?;
                Some(resume_project::duplicate(
                    &root,
                    project,
                    &name,
                    &self.session.editor.source(),
                )?)
            }
            Action::ReloadSource => {
                self.session.editor.reload()?;
                self.session.assistant.cancel();
                self.session.assistant.take_proposal();
                self.clear_proposal();
                self.session.job = None;
                self.session.preview.clear();
                self.session.rendered_source = None;
                self.session.rendered_pdf = None;
                self.start_render();
                None
            }
            Action::RecoverSource => {
                self.session.editor.recover_missing()?;
                self.start_render();
                None
            }
            _ => None,
        };

        if let Some(project) = project {
            self.refresh_projects();
            self.open_document(project.document());
            self.projects.select_path(&self.session.editor.path);
        }
        Ok(true)
    }

    pub(super) fn activate_projects(&mut self) {
        if let Some(path) = self.projects.selected_path() {
            self.open_document(path);
        }
    }

    pub(super) fn refresh_projects(&mut self) {
        if let Err(error) = self.projects.refresh() {
            self.report_error(error);
        }
    }

    pub(super) fn open_document(&mut self, path: PathBuf) {
        if path == self.session.editor.path {
            return;
        }

        let root = self.projects.root().to_path_buf();
        let projects = match Projects::new(root.clone(), &path) {
            Ok(projects) => projects,
            Err(error) => {
                self.report_error(error);
                return;
            }
        };

        let target = if let Some(session) = self.sessions.remove(&path) {
            Ok(session)
        } else {
            Editor::open(path.clone())
                .map(|editor| ResumeSession::new(editor, self.session.preview.empty_like()))
        };
        let mut target = match target {
            Ok(target) => target,
            Err(error) => {
                self.report_error(error);
                return;
            }
        };

        self.session.editor.cancel_pending_commands();
        self.session.editor.end_drag();
        target.editor.cancel_pending_commands();
        target.editor.end_drag();
        target.editor.apply_preferences(self.editor_preferences);

        let files_focused = self.projects.files_focused;
        self.projects = projects;
        self.projects.files_focused = files_focused;
        self.prefix.cancel();

        // Late results stay with the editor, undo stack, chat, and preview that requested them.
        let previous = std::mem::replace(&mut self.session, target);
        self.sessions.insert(previous.editor.path.clone(), previous);

        if self.session.job.is_none()
            && self.session.rendered_source.is_none()
            && !self.session.editor.is_modified()
        {
            self.start_render();
        }
    }
}
