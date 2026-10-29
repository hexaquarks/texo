use texo::preview::Preview;

use super::{App, Focus};
use crate::{assistant::ProposedEdit, proposal::ProposalReview, render_job::RenderJob};

impl App {
    pub(super) fn send_message(&mut self) {
        if self.session.assistant.input.lines().join("\n").trim() == "/settings" {
            self.session.assistant.clear_input();
            self.open_settings();
            return;
        }

        let command = self.session.assistant.input.lines().join("\n");
        if self.session.empty_workspace && !command.trim_start().starts_with('/') {
            self.session
                .assistant
                .report_edit_result("Create or import a resume first.");
            return;
        }

        let source = self.assistant_source();

        match self
            .session
            .assistant
            .submit(source, &self.session.editor.path)
        {
            Ok(()) => self.session.status.clear(),
            Err(error) => self.session.status = error.to_string(),
        }
    }

    pub(super) fn assistant_source(&self) -> String {
        self.session
            .proposal_review
            .as_ref()
            .map(|review| review.edit.source_after.clone())
            .unwrap_or_else(|| self.session.editor.source())
    }

    pub(super) fn receive_assistant(&mut self) {
        if let Some(result) = self.session.assistant.poll() {
            match result {
                Ok(()) if self.session.assistant.proposal().is_some() => {
                    let mut edit = self.session.assistant.proposal().cloned().unwrap();

                    // Refinements use the pending draft as context, but review against the original.
                    if let Some(review) = &self.session.proposal_review {
                        edit.source_before = review.edit.source_before.clone();
                    }

                    self.stage_proposal(edit);
                }
                Ok(()) => {
                    self.session.status = "Assistant replied without changing the document.".into()
                }
                Err(_) => {
                    self.session.status = "Assistant failed. See its message for details.".into()
                }
            }
        }
    }

    pub(super) fn stage_proposal(&mut self, edit: ProposedEdit) {
        if edit.source_before != self.session.editor.source() {
            self.session.status = "Proposal is stale: your newer text is preserved.".into();
            self.session.assistant.take_proposal();
            return;
        }

        if edit.source_before == edit.source_after {
            self.discard_proposal();
            self.session.status = "No changes proposed.".into();
            return;
        }

        if self.session.proposal_review.is_none()
            && self.session.rendered_source.as_deref() != Some(edit.source_before.as_str())
        {
            self.session.job = None;
            self.start_render();
        }

        self.session.editor.cancel_pending_commands();
        self.session.proposal_review = Some(ProposalReview::new(&edit));
        let build = self.session.assistant.take_compiled_draft();
        self.session.assistant.take_proposal();
        self.render_proposal(build);
        self.session.status = format!(
            "Review changes · {} a accept · m modify · d discard",
            self.prefix.label()
        );
    }

    pub(super) fn start_proposed_render(&mut self) {
        self.render_proposal(None);
    }

    pub(super) fn render_proposal(&mut self, build: Option<texo::sandbox::CompiledDraft>) {
        self.session.proposed_job = None;
        self.session.proposed_build = None;
        self.session.proposed_preview.clear();
        self.session.proposed_render_failed = false;
        self.clear_diagnostics();
        self.session.show_proposed = true;

        let Some(review) = &self.session.proposal_review else {
            return;
        };

        let source = review.edit.source_after.clone();
        let job = match build {
            Some(build) => RenderJob::compiled(source, build),
            None => RenderJob::draft(source, self.session.editor.path.clone()),
        };

        match job {
            Ok(job) => {
                self.session.proposed_preview.begin_loading();
                self.session.proposed_job = Some(job);
            }
            Err(error) => self.report_build_error(error, true),
        }
    }

    pub(super) fn receive_proposed_render(&mut self) {
        let Some(result) = self.session.proposed_job.as_ref().and_then(RenderJob::poll) else {
            return;
        };

        self.session.proposed_job = None;

        match result {
            Ok(document) => {
                if let Err(error) = self.session.proposed_preview.set_pages(document.pages) {
                    self.report_build_error(error, true);
                    return;
                }

                self.session.proposed_build = document.build;
                self.session.show_proposed = true;
            }
            Err(error) => self.report_build_error(error, true),
        }
    }

    pub(super) fn apply_proposal(&mut self) {
        let Some(review) = &self.session.proposal_review else {
            return;
        };

        if self.session.assistant.is_running() || self.session.proposed_job.is_some() {
            self.session.status = "Wait for the proposed version to finish.".into();
            return;
        }

        if review.edit.source_before != self.session.editor.source() {
            self.session.status =
                "Proposal is stale: discard it to preserve your newer text.".into();
            return;
        }

        let Some(build) = &self.session.proposed_build else {
            self.session.status = format!(
                "The proposal must build successfully before accepting. {} b retries.",
                self.prefix.label()
            );
            return;
        };
        let edit = review.edit.clone();

        // Stop an older current build before publishing the accepted PDF.
        self.session.job = None;
        let pdf_path = match texo::compiler::publish_pdf(&build.pdf_path, &self.session.editor.path)
        {
            Ok(path) => path,
            Err(error) => {
                self.report_error(error);
                return;
            }
        };

        self.apply_edit(&edit);
        std::mem::swap(
            &mut self.session.preview,
            &mut self.session.proposed_preview,
        );
        self.session.rendered_source = Some(edit.source_after);
        self.session.rendered_pdf = Some(pdf_path);
        self.session.render_failed = false;
        self.clear_proposal();
        self.session.status = format!(
            "Accepted · PDF updated · {} w to save TeX",
            self.prefix.label()
        );
        self.session.assistant.report_edit_result(&format!(
            "Changes accepted. PDF updated; {} w to save TeX.",
            self.prefix.label()
        ));
    }

    pub(super) fn modify_proposal(&mut self) {
        if self.session.proposal_review.is_some() {
            self.focus = Focus::Assistant;
            self.session.status =
                "Describe your revisions in chat; the pending draft will be used.".into();
            self.session.assistant.report_edit_result(
                "Ready to revise. Tell me what to change in the proposed draft.",
            );
        }
    }

    pub(super) fn discard_proposal(&mut self) {
        self.session.assistant.cancel();
        self.session.assistant.take_proposal();
        self.clear_proposal();
        self.session.status = "Proposal discarded. Original text and preview preserved.".into();
        self.session
            .assistant
            .report_edit_result("Proposal discarded; no changes accepted.");
    }

    pub(super) fn clear_proposal(&mut self) {
        self.session.proposed_job = None;
        self.session.proposed_build = None;
        self.session.proposed_preview.clear();
        self.session.proposal_review = None;
        self.clear_diagnostics();
        self.session.show_proposed = false;
    }

    pub(super) fn active_preview(&mut self) -> &mut Preview {
        if self.session.show_proposed && self.session.proposal_review.is_some() {
            &mut self.session.proposed_preview
        } else {
            &mut self.session.preview
        }
    }

    pub(super) fn apply_edit(&mut self, proposal: &ProposedEdit) -> bool {
        if proposal.source_before != self.session.editor.source() {
            self.session.status =
                "Edit not applied: the document changed during the request. Ask Texo again.".into();
            self.session.assistant.report_edit_result(
                "Not applied: you changed the editor while I was responding. Your newer text is preserved.",
            );
            return false;
        }

        if self.session.editor.replace_source(&proposal.source_after) {
            self.session.status = format!(
                "Edit applied. Undo twice to revert; {} w to save; {} b to build.",
                self.prefix.label(),
                self.prefix.label()
            );
        } else {
            self.session.status = "The returned resume is unchanged.".into();
            self.session
                .assistant
                .report_edit_result("No changes applied: the returned text matches the editor.");
        }
        true
    }

    pub(super) fn start_render(&mut self) {
        if self.session.empty_workspace {
            return;
        }
        if self
            .projects
            .project_for(&self.session.editor.path)
            .is_some()
            && !self.session.editor.path.is_file()
        {
            self.session.status = "Source missing · Actions → Restore source".into();
            return;
        }
        if self.session.proposal_review.is_some() {
            self.start_proposed_render();
            return;
        }

        if self.session.job.is_some() {
            return;
        }

        self.session.building_source = self.session.editor.source();
        match RenderJob::draft(
            self.session.editor.source(),
            self.session.editor.path.clone(),
        ) {
            Ok(job) => {
                self.session.preview.begin_loading();
                self.session.job = Some(job);
                self.session.render_failed = false;
                self.clear_diagnostics();
                self.session.status = "Building…".into();
            }
            Err(error) => self.report_build_error(error, false),
        }
    }

    pub(super) fn receive_render(&mut self) {
        let Some(result) = self.session.job.as_ref().and_then(RenderJob::poll) else {
            return;
        };
        self.session.job = None;

        match result {
            Ok(document) if document.source != self.session.editor.source() => {
                self.session.status = "Build is outdated; rebuild the current text.".into();
            }
            Ok(mut document) => {
                match texo::compiler::publish_pdf(&document.pdf_path, &self.session.editor.path) {
                    Ok(path) => document.pdf_path = path,
                    Err(error) => {
                        self.report_build_error(error, false);
                        return;
                    }
                }
                match self.session.preview.set_pages(document.pages) {
                    Ok(()) => {
                        self.session.rendered_source = Some(document.source);
                        self.session.rendered_pdf = Some(document.pdf_path);
                        if self.session.proposal_review.is_none() {
                            self.session.status.clear();
                            self.clear_diagnostics();
                        }

                        self.refresh_projects();
                    }
                    Err(error) => self.report_build_error(error, false),
                }
            }
            Err(error) => self.report_build_error(error, false),
        }
    }

    pub(super) fn report_build_error(&mut self, error: anyhow::Error, proposed: bool) {
        if proposed {
            self.session.proposed_render_failed = true;
        } else {
            self.session.render_failed = true;
        }

        let source = if proposed {
            self.assistant_source()
        } else {
            self.session.building_source.clone()
        };
        let log = format!("{error:#}");
        self.report_error(error);
        self.session.build_diagnostics = Some(crate::build_diagnostics::BuildDiagnostics::parse(
            &log,
            source,
            &self.session.editor.path,
            proposed,
        ));
        self.session.status = "Build failed · see the error below the source.".into();
    }

    pub(super) fn report_error(&mut self, error: anyhow::Error) {
        self.clear_diagnostics();
        self.session.status = format!("Operation failed. {} E opens details.", self.prefix.label());
        self.session.diagnostics = Some(format!("{error:#}"));
        self.session.diagnostic_scroll = 0;
    }
}
