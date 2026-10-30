use std::{collections::HashMap, path::PathBuf, time::Duration};

#[cfg(test)]
use anyhow::Context;

use ratatui::DefaultTerminal;
use texo::preview::Preview;

use crate::{
    actions::{ActionMenu, ExportDialog},
    config::Settings,
    editor::Editor,
    keybindings::Prefix,
    label_dialog::LabelDialog,
    project_dialog::ProjectDialog,
    projects::Projects,
    settings::SettingsDialog,
};

use layout_state::PaneSizes;
use resume_session::ResumeSession;

#[path = "resume_session.rs"]
mod resume_session;

const EVENT_POLL_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Clone, Copy, PartialEq)]
enum Focus {
    Projects,
    Editor,
    Preview,
    Assistant,
}

struct App {
    session: ResumeSession,
    projects: Projects,
    projects_visible: bool,
    actions: Option<ActionMenu>,
    project_dialog: Option<ProjectDialog>,
    label_dialog: Option<LabelDialog>,
    sessions: HashMap<PathBuf, ResumeSession>,
    settings: Option<SettingsDialog>,
    settings_path: PathBuf,
    prefix: Prefix,
    editor_preferences: crate::config::EditorPreferences,
    pane_sizes: PaneSizes,
    layout_state_path: Option<PathBuf>,
    export: Option<ExportDialog>,
    focus: Focus,
    workspace_expansion: Option<Focus>,
    shortcuts_open: bool,
    shortcut_scroll: usize,
    shortcut_query: String,
    shortcut_searching: bool,
}

pub fn run(
    terminal: &mut DefaultTerminal,
    editor: Editor,
    root: PathBuf,
    empty_workspace: bool,
) -> anyhow::Result<()> {
    let mut app = App::with_workspace(editor, Preview::detect(), root)?;
    app.restore_layout(PaneSizes::path()?);
    app.session.empty_workspace = empty_workspace;
    if !empty_workspace && !app.session.editor.is_modified() {
        app.start_render();
    }
    let settings = Settings::load(&app.settings_path)?;
    app.prefix = Prefix::new(&settings.keybindings.prefix)?;
    app.apply_editor_preferences(settings.editor);

    let result = app.run_event_loop(terminal);
    let cleanup = app.release_terminal_images(terminal.backend_mut());
    result.and(cleanup)
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
            session: ResumeSession::new(editor, preview),
            projects,
            projects_visible: true,
            actions: None,
            project_dialog: None,
            label_dialog: None,
            sessions: HashMap::new(),
            settings: None,
            settings_path: Settings::path()?,
            prefix: Prefix::default(),
            editor_preferences: crate::config::EditorPreferences::default(),
            pane_sizes: PaneSizes::default(),
            layout_state_path: None,
            export: None,
            focus: Focus::Editor,
            workspace_expansion: None,
            shortcuts_open: false,
            shortcut_scroll: 0,
            shortcut_query: String::new(),
            shortcut_searching: false,
        })
    }

    #[cfg(test)]
    fn start(editor: Editor, preview: Preview, root: PathBuf) -> anyhow::Result<Self> {
        let mut app = Self::with_workspace(editor, preview, root)?;

        if !app.session.editor.is_modified() {
            app.start_render();
        }

        Ok(app)
    }

    fn apply_editor_preferences(&mut self, preferences: crate::config::EditorPreferences) {
        self.editor_preferences = preferences;
        self.session.editor.apply_preferences(preferences);
        for session in self.sessions.values_mut() {
            session.editor.apply_preferences(preferences);
        }
    }

    fn editor_has_focus(&self) -> bool {
        self.session.history.view.is_none()
            && self.focus == Focus::Editor
            && !self.session.empty_workspace
            && self.session.proposal_review.is_none()
            && !self.overlay_open()
            && !self.session.assistant.login_open()
    }

    fn overlay_open(&self) -> bool {
        self.session.history.dialog.is_some()
            || self.shortcuts_open
            || self.diagnostic_details_open()
            || self.project_dialog.is_some()
            || self.label_dialog.is_some()
            || self.actions.is_some()
            || self.export.is_some()
            || self.settings.is_some()
    }

    fn overlays_require_clear(&mut self, was_open: bool) -> bool {
        // Inline image protocols cannot restore covered pixels from text-cell diffs.
        // Kitty placeholders and basic rendering need only the ordinary modal redraw.
        was_open && !self.overlay_open() && self.active_preview().uses_inline_graphics()
    }
}

#[path = "diagnostic_details.rs"]
mod diagnostic_details;
#[path = "event_loop.rs"]
mod event_loop;
#[path = "history_view.rs"]
mod history_view;
#[path = "input.rs"]
mod input;
#[path = "label_actions.rs"]
mod label_actions;
#[path = "layout_state.rs"]
mod layout_state;
#[path = "pane_navigation.rs"]
mod pane_navigation;
#[path = "problem_actions.rs"]
mod problem_actions;
#[path = "resume_review.rs"]
mod resume_review;
#[path = "shortcut_help.rs"]
mod shortcut_help;
#[path = "workspace_actions.rs"]
mod workspace_actions;
#[path = "workspace_view.rs"]
mod workspace_view;

#[cfg(test)]
#[path = "app_input_tests.rs"]
mod app_input_tests;
#[cfg(test)]
#[path = "build_diagnostics_tests.rs"]
mod build_diagnostics_tests;
#[cfg(test)]
#[path = "label_tests.rs"]
mod label_tests;
#[cfg(test)]
#[path = "modal_preview_tests.rs"]
mod modal_preview_tests;
#[cfg(test)]
#[path = "pane_expansion_tests.rs"]
mod pane_expansion_tests;
#[cfg(test)]
#[path = "project_dialog_tests.rs"]
mod project_dialog_tests;
#[cfg(test)]
#[path = "resume_lifecycle_tests.rs"]
mod resume_lifecycle_tests;
#[cfg(test)]
#[path = "resume_review_tests.rs"]
mod resume_review_tests;
#[cfg(test)]
#[path = "resume_session_tests.rs"]
mod resume_session_tests;
#[cfg(test)]
#[path = "app_test_support.rs"]
mod test_support;
#[cfg(test)]
#[path = "workspace_settings_tests.rs"]
mod workspace_settings_tests;
#[cfg(test)]
#[path = "workspace_view_tests.rs"]
mod workspace_view_tests;

#[cfg(test)]
use {
    crate::{actions::Action, assistant::ProposedEdit, proposal::ProposalReview, resume_project},
    ratatui::{
        crossterm::event::{
            self, Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEventKind,
        },
        layout::Rect,
    },
    texo::theme,
    workspace_view::{WorkspaceLayout, pane_content, preview_header},
};

#[cfg(test)]
#[path = "history_tests.rs"]
mod history_tests;
