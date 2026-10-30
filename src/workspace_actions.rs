use std::path::PathBuf;

use anyhow::Context;

use ratatui::{
    crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind},
    layout::{Position, Rect},
};

use super::{App, resume_session::ResumeSession};
use crate::{
    actions::{Action, ActionMenu, ExportDialog},
    config::Settings,
    editor::Editor,
    keybindings::Prefix,
    project_dialog::ProjectDialog,
    projects::Projects,
    resume_project::{self, ImportPlan},
    settings::{SettingsAction, SettingsDialog},
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
        dialog.original_preferences = self.editor_preferences;
        self.settings = Some(dialog);
    }

    pub(super) fn handle_settings_event(&mut self, event: Event, screen: Rect) {
        let action = self
            .settings
            .as_mut()
            .and_then(|dialog| dialog.handle_event(event, screen));
        let result = match action {
            Some(SettingsAction::Cancel) => {
                self.settings = None;
                Ok(())
            }
            Some(SettingsAction::Apply) => self.apply_settings(),
            Some(SettingsAction::OpenConfig) => self.open_config_with(crate::config::open_editor),
            Some(SettingsAction::ReloadConfig) => self.reload_settings(),
            Some(SettingsAction::ChangeFolder) => self.begin_folder_change(),
            Some(SettingsAction::PrepareFolder) => self.prepare_folder_change(),
            Some(SettingsAction::SwitchFolder) => self.confirm_folder_change(),
            None => return,
        };
        if let Some(dialog) = &mut self.settings {
            match result {
                Ok(()) => {
                    dialog.error.clear();
                    if matches!(action, Some(SettingsAction::OpenConfig)) {
                        dialog.notice = "Opened config in your text editor.".into();
                    }
                }
                Err(error) => {
                    dialog.notice.clear();
                    dialog.error = error.to_string();
                }
            }
        }
    }

    fn require_resolved_preferences(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self
                .settings
                .as_ref()
                .is_some_and(SettingsDialog::has_staged_preferences),
            "Apply or cancel preference changes before changing workspaces or reloading config."
        );
        Ok(())
    }

    pub(super) fn begin_folder_change(&mut self) -> anyhow::Result<()> {
        self.require_resolved_preferences()?;
        if let Some(dialog) = &mut self.settings {
            dialog.reload_folder = false;
            dialog.begin_folder_change();
        }
        Ok(())
    }

    fn workspace_losses(&self) -> Vec<&'static str> {
        let sessions: Vec<_> = std::iter::once(&self.session)
            .chain(self.sessions.values())
            .collect();
        let mut losses = Vec::new();
        if sessions
            .iter()
            .any(|session| session.editor.is_modified() && !session.empty_workspace)
        {
            losses.push("unsaved text");
        }
        if sessions
            .iter()
            .any(|session| session.assistant.is_running() || session.proposal_review.is_some())
        {
            losses.push("pending AI work or proposals");
        }
        if sessions.iter().any(|session| session.assistant.has_chat()) {
            losses.push("chat");
        }
        losses
    }

    pub(super) fn prepare_folder_change(&mut self) -> anyhow::Result<()> {
        self.require_resolved_preferences()?;
        let dialog = self.settings.as_ref().context("Settings is closed")?;
        let root = crate::config::resume_folder(&dialog.value())?;
        let mut settings = Settings::load(&self.settings_path)?;
        settings.resume_folder = Some(root.clone());
        if !dialog.reload_folder {
            settings.editor = self.editor_preferences;
        }
        let save = !dialog.reload_folder;
        if self.projects.root().canonicalize()? == root {
            return self.replace_workspace(root, settings, save);
        }
        // Validate the complete replacement while all current sessions and workers remain live.
        let _prepared = self.prepare_workspace(root.clone(), &settings)?;
        let losses = self.workspace_losses();
        self.settings.as_mut().unwrap().confirm_folder(root, losses);
        Ok(())
    }

    pub(super) fn confirm_folder_change(&mut self) -> anyhow::Result<()> {
        self.require_resolved_preferences()?;
        let dialog = self.settings.as_ref().context("Settings is closed")?;
        let confirmation = dialog
            .confirmation
            .as_ref()
            .context("Validate the destination first")?;
        let root = crate::config::resume_folder(&dialog.value())?;
        let losses = self.workspace_losses();
        if confirmation.input != dialog.value()
            || confirmation.destination != root
            || confirmation.losses != losses
        {
            return self.prepare_folder_change();
        }
        let save = !dialog.reload_folder;
        let mut settings = Settings::load(&self.settings_path)?;
        if save {
            settings.resume_folder = Some(root.clone());
            settings.editor = self.editor_preferences;
        } else {
            let configured = settings
                .resume_folder
                .as_deref()
                .unwrap_or(std::path::Path::new("~/resumes"));
            anyhow::ensure!(
                crate::config::resume_folder(&configured.to_string_lossy())? == root,
                "Config destination changed. Reload config again."
            );
        }
        self.replace_workspace(root, settings, save)
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
        self.require_resolved_preferences()?;
        let settings = Settings::load(&self.settings_path)?;
        let folder = settings
            .resume_folder
            .as_deref()
            .unwrap_or(std::path::Path::new("~/resumes"));
        let root = crate::config::resume_folder(&folder.to_string_lossy())?;
        let keep_open = self.settings.is_some();
        if self.projects.root().canonicalize()? != root {
            let _prepared = self.prepare_workspace(root.clone(), &settings)?;
            self.open_settings();
            let losses = self.workspace_losses();
            let dialog = self.settings.as_mut().unwrap();
            dialog.input =
                ratatui_textarea::TextArea::new(vec![crate::workspace::display_path(&root)]);
            dialog.begin_folder_change();
            dialog.reload_folder = true;
            dialog.confirm_folder(root, losses);
            return Ok(());
        }
        self.replace_workspace(root, settings, false)?;
        if keep_open {
            self.open_settings();
            self.settings.as_mut().unwrap().notice = "Configuration reloaded.".into();
        }
        Ok(())
    }

    pub(super) fn apply_settings(&mut self) -> anyhow::Result<()> {
        let Some(dialog) = &self.settings else {
            return Ok(());
        };
        let preferences = dialog.editor_preferences;
        let mut settings = Settings::load(&self.settings_path)?;
        settings.editor = preferences;
        settings.save(&self.settings_path)?;
        self.apply_editor_preferences(preferences);
        self.settings = None;
        Ok(())
    }

    fn prepare_workspace(&self, root: PathBuf, settings: &Settings) -> anyhow::Result<Self> {
        let prefix = Prefix::new(&settings.keybindings.prefix)?;
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
        Ok(replacement)
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

        // Prepare again at confirmation so unreadable or changed destinations cannot replace live work.
        let replacement = self.prepare_workspace(root, &settings)?;
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
        if let Err(error) = self.ensure_export_ready() {
            self.session.status = error.to_string();
            return;
        }

        self.export = Some(ExportDialog::new(&self.session.editor.path));
    }

    fn ensure_export_ready(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.session.proposal_review.is_none(),
            "Accept or discard the proposal before exporting."
        );
        anyhow::ensure!(
            self.session.job.is_none(),
            "Wait for the build to finish before exporting."
        );

        // Included files can break a rebuild without changing the editor's source text.
        anyhow::ensure!(
            !self.session.render_failed,
            "The latest build failed. Rebuild successfully before exporting."
        );
        anyhow::ensure!(
            self.session.rendered_source.as_deref() == Some(self.session.editor.source().as_str())
                && self.session.rendered_pdf.is_some(),
            "Build the current document with {} b before exporting.",
            self.prefix.label()
        );

        Ok(())
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
        match action {
            Action::LabelResume => self.open_label_dialog(),
            _ => self.open_project_dialog(action),
        }
        false
    }

    pub(super) fn finish_export(&mut self) {
        // Background completions can change review/build state while the dialog is open.
        if let Err(error) = self.ensure_export_ready() {
            self.export = None;
            self.session.status = format!("PDF changed while exporting. {error}");
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

    pub(super) fn handle_project_dialog(&mut self, event: Event, area: Rect) {
        let Some(mut dialog) = self.project_dialog.take() else {
            return;
        };

        match event {
            Event::Mouse(mouse) if mouse.kind == MouseEventKind::Down(MouseButton::Left) => {
                dialog.click(area, Position::new(mouse.column, mouse.row));
            }
            Event::Key(key) if key.code == KeyCode::Esc => return,
            Event::Key(key) if key.code == KeyCode::Enter => {
                match self.finish_project_dialog(&mut dialog) {
                    Ok(true) => return,
                    Ok(false) => {}
                    Err(error) => {
                        // Let the user correct the fields and inspect the source again on retry.
                        if matches!(dialog.action, Action::ImportResume) {
                            dialog.plan = None;
                            dialog.selection.select(Some(0));
                        }

                        dialog.error = error.to_string();
                    }
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

    fn project_working_copies(&self, root: &std::path::Path) -> Vec<(PathBuf, String)> {
        self.sessions
            .values()
            .chain(std::iter::once(&self.session))
            .filter_map(|session| {
                let relative = session.editor.path.strip_prefix(root).ok()?;
                Some((relative.to_path_buf(), session.editor.source()))
            })
            .collect()
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
                let working_copies = self.project_working_copies(&project.root);
                Some(resume_project::duplicate(
                    &root,
                    project,
                    &name,
                    &working_copies,
                )?)
            }
            Action::ReloadSource => {
                self.session.editor.reload()?;
                self.return_to_working_copy();
                self.refresh_history();
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
                let warning = self.session.editor.recover_missing()?;
                self.refresh_history();
                self.start_render();
                if let Some(warning) = warning {
                    self.session.status = warning;
                }
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
            self.return_to_working_copy();
            self.refresh_history();
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

        let history_focused = self.projects.history_focused;
        self.projects = projects;
        self.projects.history_focused = history_focused;
        self.prefix.cancel();

        self.restore_workspace_panes();
        target.column_expansion = None;

        // Late results stay with the editor, undo stack, chat, and preview that requested them.
        let previous = std::mem::replace(&mut self.session, target);
        self.sessions.insert(previous.editor.path.clone(), previous);
        self.return_to_working_copy();
        self.refresh_history();

        if self.session.job.is_none()
            && self.session.rendered_source.is_none()
            && !self.session.editor.is_modified()
        {
            self.start_render();
        }
    }
}
