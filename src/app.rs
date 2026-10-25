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
    sessions: HashMap<PathBuf, ResumeSession>,
    settings: Option<SettingsDialog>,
    settings_path: PathBuf,
    prefix: Prefix,
    pane_sizes: PaneSizes,
    layout_state_path: Option<PathBuf>,
    export: Option<ExportDialog>,
    focus: Focus,
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
    app.prefix = Prefix::new(&Settings::load(&app.settings_path)?.keybindings.prefix)?;

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
            sessions: HashMap::new(),
            settings: None,
            settings_path: Settings::path()?,
            prefix: Prefix::default(),
            pane_sizes: PaneSizes::default(),
            layout_state_path: None,
            export: None,
            focus: Focus::Editor,
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

    fn overlay_open(&self) -> bool {
        self.project_dialog.is_some()
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

#[path = "event_loop.rs"]
mod event_loop;
#[path = "input.rs"]
mod input;
#[path = "layout_state.rs"]
mod layout_state;
#[path = "pane_navigation.rs"]
mod pane_navigation;
#[path = "resume_review.rs"]
mod resume_review;
#[path = "workspace_actions.rs"]
mod workspace_actions;
#[path = "workspace_view.rs"]
mod workspace_view;

#[cfg(test)]
#[path = "app_input_tests.rs"]
mod app_input_tests;
#[cfg(test)]
#[path = "modal_preview_tests.rs"]
mod modal_preview_tests;
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
    workspace_view::{WorkspaceLayout, pane_content, preview_header, review_buttons},
};
