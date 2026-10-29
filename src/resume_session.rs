use std::path::PathBuf;

use texo::preview::Preview;

use crate::{
    assistant::Assistant, editor::Editor, proposal::ProposalReview, render_job::RenderJob,
};

// A session owns only one resume's state. Application settings and navigation stay in App.
pub(super) struct ResumeSession {
    pub(super) editor: Editor,
    pub(super) empty_workspace: bool,

    pub(super) preview: Preview,
    pub(super) job: Option<RenderJob>,
    pub(super) rendered_source: Option<String>,
    pub(super) rendered_pdf: Option<PathBuf>,
    pub(super) render_failed: bool,
    pub(super) proposed_render_failed: bool,

    pub(super) assistant: Assistant,
    pub(super) column_expansion: Option<super::Focus>,

    pub(super) proposal_review: Option<ProposalReview>,
    pub(super) proposed_preview: Preview,
    pub(super) proposed_job: Option<RenderJob>,
    pub(super) proposed_build: Option<texo::sandbox::CompiledDraft>,
    pub(super) show_proposed: bool,

    pub(super) status: String,
    pub(super) diagnostics: Option<String>,
    pub(super) diagnostic_scroll: u16,
    pub(super) details_open: bool,
    pub(super) summary_hidden: bool,
    pub(super) build_diagnostics: Option<crate::build_diagnostics::BuildDiagnostics>,
    pub(super) building_source: String,
}

impl ResumeSession {
    pub(super) fn new(editor: Editor, preview: Preview) -> Self {
        let proposed_preview = preview.empty_like();

        Self {
            editor,
            empty_workspace: false,

            preview,
            job: None,
            rendered_source: None,
            rendered_pdf: None,
            render_failed: false,
            proposed_render_failed: false,

            assistant: Assistant::new(),
            column_expansion: None,

            proposal_review: None,
            proposed_preview,
            proposed_job: None,
            proposed_build: None,
            show_proposed: false,

            status: String::new(),
            diagnostics: None,
            diagnostic_scroll: 0,
            details_open: false,
            summary_hidden: false,
            build_diagnostics: None,
            building_source: String::new(),
        }
    }
}
