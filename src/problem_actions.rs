use super::{App, Focus};
use crate::render_job::RenderJob;

impl App {
    pub(super) fn clear_diagnostics(&mut self) {
        self.session.diagnostics = None;
        self.session.build_diagnostics = None;
        self.session.details_open = false;
        self.session.summary_hidden = false;
        self.session.diagnostic_scroll = 0;
    }

    pub(super) fn diagnostic_details_open(&self) -> bool {
        self.session.diagnostics.is_some()
            && (self.session.details_open
                || (self.session.build_diagnostics.is_none() && !self.session.summary_hidden))
    }

    pub(super) fn toggle_problem_summary(&mut self) {
        self.session.summary_hidden = !self.session.summary_hidden;
        self.session.status = if self.session.summary_hidden {
            format!("Error summary hidden · {} D to show", self.prefix.label())
        } else {
            "Build failed · see the error below the source.".into()
        };
    }

    pub(super) fn diagnostics_current(&self) -> bool {
        self.session
            .build_diagnostics
            .as_ref()
            .is_some_and(|diagnostics| {
                if diagnostics.proposed {
                    self.session
                        .proposal_review
                        .as_ref()
                        .is_some_and(|review| review.edit.source_after == diagnostics.source)
                } else {
                    self.session.proposal_review.is_none()
                        && self.session.editor.source() == diagnostics.source
                }
            })
    }

    pub(super) fn select_problem(&mut self, next: bool) {
        let Some(diagnostics) = &mut self.session.build_diagnostics else {
            return;
        };
        let count = diagnostics.problems.len();
        diagnostics.selected = if next {
            (diagnostics.selected + 1) % count
        } else {
            (diagnostics.selected + count - 1) % count
        };
        self.go_to_problem();
    }

    pub(super) fn go_to_problem(&mut self) {
        if !self.diagnostics_current() || self.session.proposal_review.is_some() {
            return;
        }
        if let Some(line) = self
            .session
            .build_diagnostics
            .as_ref()
            .and_then(|diagnostics| diagnostics.selected().editor_line())
        {
            self.session.editor.go_to_line(line);
            self.focus = Focus::Editor;
        }
    }

    pub(super) fn explain_problem(&mut self) {
        if !self.diagnostics_current() {
            self.session.status = "Source changed. Rebuild before asking about this error.".into();
            return;
        }
        let Some(diagnostics) = &self.session.build_diagnostics else {
            return;
        };
        let problem = diagnostics.selected();
        let location = problem.location.as_deref().unwrap_or("build process");
        let prompt = format!(
            "Help fix this LaTeX build error at {location}: {}\n{}\nPropose a minimal fix and verify it compiles with Tectonic.",
            problem.message, problem.help
        );
        let input = &mut self.session.assistant.input;
        input.cancel_selection();
        if input.lines().iter().any(|line| !line.is_empty()) {
            input.move_cursor(ratatui_textarea::CursorMove::Bottom);
            input.move_cursor(ratatui_textarea::CursorMove::End);
            input.insert_str("\n\n");
        }
        input.insert_str(prompt);
        if let Some(line) = problem
            .editor_line()
            .and_then(|line| diagnostics.source.lines().nth(line - 1))
        {
            input.insert_str(format!("\nSource line: {line}"));
        }

        input.move_cursor(ratatui_textarea::CursorMove::Top);
        input.move_cursor(ratatui_textarea::CursorMove::Head);
        self.focus = Focus::Assistant;
        self.session.status =
            "Error added to chat. Review the message, then press Enter to send.".into();
    }

    pub(super) fn recover_resource(&mut self) {
        if !self.diagnostics_current()
            || self.session.job.is_some()
            || self.session.proposed_job.is_some()
        {
            return;
        }
        let Some(diagnostics) = &self.session.build_diagnostics else {
            return;
        };
        let Some(resource) = diagnostics.selected().resource.clone() else {
            return;
        };
        let proposed = diagnostics.proposed;
        let source = diagnostics.source.clone();
        if !proposed {
            self.session.building_source = source.clone();
        }
        match RenderJob::recover(source, self.session.editor.path.clone(), resource) {
            Ok(job) => {
                if proposed {
                    self.session.proposed_render_failed = false;
                    self.session.proposed_job = Some(job);
                } else {
                    self.session.render_failed = false;
                    self.session.job = Some(job);
                }
                self.session.status =
                    "Downloading resource, then retrying the offline build…".into();
                self.clear_diagnostics();
            }
            Err(error) => self.report_build_error(error, proposed),
        }
    }
}
