use ratatui::{crossterm::event::Event, layout::Rect};

use super::App;
use crate::{
    label_dialog::{LabelDialog, LabelResult},
    resume_project::Project,
};

impl App {
    pub(super) fn open_label_dialog(&mut self) {
        let Some(project) = self.projects.project_for(&self.session.editor.path) else {
            self.session.status = "Create or import a resume before adding a label.".into();
            return;
        };

        self.label_dialog = Some(LabelDialog::new(project));
        self.actions = None;
        self.prefix.cancel();
    }

    pub(super) fn handle_label_event(&mut self, event: Event, area: Rect) {
        let Some(mut dialog) = self.label_dialog.take() else {
            return;
        };

        match dialog.handle_event(event, area) {
            LabelResult::Cancel => return,
            LabelResult::Continue => {}
            LabelResult::Apply(label) => {
                // Reload metadata so a label edit cannot overwrite a concurrent rename.
                let result = Project::load(&dialog.project_root)
                    .and_then(|mut project| project.set_label(label));

                match result {
                    Ok(()) => {
                        self.refresh_projects();
                        return;
                    }
                    Err(error) => dialog.error = error.to_string(),
                }
            }
        }

        self.label_dialog = Some(dialog);
    }
}
