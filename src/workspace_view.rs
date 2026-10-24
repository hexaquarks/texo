use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::Style,
    widgets::{Block, Borders, Paragraph, Wrap},
};
use texo::{preview::Preview, theme};

use super::{App, Focus};

const SETTINGS_HINT: &str = "? Settings";

const MULTIPANE_WIDTH: u16 = 100;
const MULTIPANE_HEIGHT: u16 = 20;
const SIDEBAR_WIDTH: u16 = 24;
const PREVIEW_PERCENT: u16 = 38;
const EDITOR_PERCENT: u16 = 55;
// Borders, padding, header, and composer must fit even with the status row visible.
const MIN_CHAT_HEIGHT: u16 = 11;
const DIAGNOSTICS_HEIGHT: u16 = 8;
const REVIEW_CONTROLS_HEIGHT: u16 = 2;
const REVIEW_BUTTON_WIDTH: u16 = 13;
const PREVIEW_TAB_WIDTH: u16 = 13;

impl App {
    pub(super) fn workspace_layout(&self, area: Rect) -> WorkspaceLayout {
        let mut layout = WorkspaceLayout::new(
            area,
            self.session.diagnostics.is_some(),
            self.projects_visible,
            self.focus,
        );

        // Keep the PDF and sidebar in place; chat only takes over the editor's space.
        if self.session.chat_maximized && self.focus != Focus::Editor && !layout.editor.is_empty() {
            layout.assistant = layout.assistant.union(layout.editor);
            layout.editor = Rect::default();
        }

        layout
    }

    pub(super) fn preview_area(&self, area: Rect) -> Rect {
        self.workspace_layout(area).preview
    }

    pub(super) fn draw(&mut self, frame: &mut Frame) {
        // Opening another document can focus the editor without a navigation key.
        if self.focus == Focus::Editor {
            self.session.chat_maximized = false;
        }

        let area = frame.area();
        frame.render_widget(Block::default().style(theme::text()), area);

        let layout = self.workspace_layout(area);
        if self.projects_visible {
            self.projects.draw(
                frame,
                layout.projects,
                layout.files,
                &self.session.editor.path,
                self.focus == Focus::Projects,
            );
        }
        let editor = draw_pane_border(frame, layout.editor, self.focus == Focus::Editor);
        let assistant = draw_pane_border(frame, layout.assistant, self.focus == Focus::Assistant);
        let preview = draw_pane_border(frame, layout.preview, self.focus == Focus::Preview);

        let source_state = self.session.editor.disk_state();
        self.draw_editor(frame, editor, &source_state);
        self.draw_assistant(frame, assistant);
        self.draw_preview(frame, preview, &source_state);
        self.draw_diagnostics(frame, &layout);

        let settings_hint = if self.prefix.armed {
            if self.focus == Focus::Assistant {
                "Prefix · u Maximize/restore · n New · i Import · r Resumes · f Files · s Settings"
                    .to_owned()
            } else {
                "Prefix · n New · i Import · r Resumes · f Files · s Settings · Esc Cancel"
                    .to_owned()
            }
        } else {
            format!("{} Prefix · {SETTINGS_HINT} ", self.prefix.label())
        };
        frame.render_widget(
            Paragraph::new(settings_hint)
                .alignment(ratatui::layout::Alignment::Right)
                .style(theme::muted().bg(theme::ELEVATED)),
            layout.status,
        );

        if let Some(menu) = &mut self.actions {
            menu.draw(frame);
        }
        if let Some(export) = &mut self.export {
            export.draw(frame);
        }
        if let Some(settings) = &self.settings {
            settings.draw(frame);
        }
        if let Some(dialog) = &mut self.project_dialog {
            dialog.draw(frame);
        }
    }

    pub(super) fn draw_editor(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        source_state: &anyhow::Result<crate::editor::DiskState>,
    ) {
        if self.session.empty_workspace {
            let prompt = "Create a resume with +, or Import from Actions (Ctrl+P).\nExisting loose TeX files stay untouched.";
            frame.render_widget(
                Paragraph::new(prompt)
                    .style(theme::muted())
                    .wrap(Wrap { trim: false }),
                area.inner(ratatui::layout::Margin::new(theme::PANEL_PADDING, 1)),
            );
            return;
        }

        if let Some(review) = &self.session.proposal_review {
            review.draw(frame, area);
        } else {
            self.session
                .editor
                .draw(frame, area, self.focus == Focus::Editor);
        }

        if !area.is_empty() {
            let modified = if self.session.editor.is_modified() {
                " · unsaved"
            } else {
                ""
            };
            let label = if !self.session.editor.path.is_file() {
                "Source missing · Actions → Restore source".into()
            } else if matches!(source_state, Ok(crate::editor::DiskState::Changed)) {
                "Source changed externally · Actions → Reload source".into()
            } else if source_state.is_err() {
                "Source unreadable · Your editor text is preserved".into()
            } else if self.session.proposal_review.is_some() {
                format!(
                    "Unapproved changes · {}",
                    crate::workspace::display_path(&self.session.editor.path)
                )
            } else if self.session.status.is_empty() {
                format!(
                    "{}{}",
                    crate::workspace::display_path(&self.session.editor.path),
                    modified
                )
            } else {
                self.session.status.clone()
            };
            let title = Rect::new(
                area.x + theme::PANEL_PADDING,
                area.y,
                area.width.saturating_sub(theme::PANEL_PADDING * 2),
                1,
            );
            frame.render_widget(
                Paragraph::new(label).style(theme::pane_title(self.focus == Focus::Editor)),
                title,
            );
        }
    }

    pub(super) fn assistant_body(&self, area: Rect) -> Rect {
        if self.session.proposal_review.is_some() && !self.session.assistant.login_open() {
            below_header(area, REVIEW_CONTROLS_HEIGHT)
        } else {
            area
        }
    }

    pub(super) fn draw_assistant(&mut self, frame: &mut Frame, area: Rect) {
        let body = self.assistant_body(area);
        self.session.assistant.draw(
            frame,
            body,
            self.focus == Focus::Assistant,
            self.session.chat_maximized,
            &self.prefix.label(),
        );

        if self.session.proposal_review.is_some() && !self.session.assistant.login_open() {
            for (button, label) in
                review_buttons(area)
                    .into_iter()
                    .zip(["[F7 Accept]", "[F8 Modify]", "[F9 Discard]"])
            {
                frame.render_widget(Paragraph::new(label).style(theme::pane_title(true)), button);
            }
        }
    }

    pub(super) fn draw_preview(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        source_state: &anyhow::Result<crate::editor::DiskState>,
    ) {
        if area.is_empty() {
            return;
        }

        let body = below_header(area, 1);
        let proposed = self.session.show_proposed && self.session.proposal_review.is_some();
        let loading = if proposed {
            self.session.proposed_job.as_ref()
        } else {
            self.session.job.as_ref()
        };

        let is_loading = loading.is_some();
        if let Some(job) = loading {
            Preview::draw_loading(frame, body, job.elapsed());
        } else if !self.overlay_open() {
            // Kitty uploads only on its first draw. A modal must not overwrite that
            // upload in Ratatui's buffer before it reaches the terminal.
            self.active_preview().draw(frame, body);
        }

        let state = if self.session.empty_workspace {
            "PDF missing"
        } else if is_loading {
            "PDF building"
        } else if (proposed && self.session.proposed_render_failed)
            || (!proposed && self.session.render_failed)
        {
            "PDF failed"
        } else if self.session.rendered_pdf.is_none() && !proposed {
            "PDF missing"
        } else if !proposed
            && (self.session.rendered_source.as_deref()
                != Some(self.session.editor.source().as_str())
                || !matches!(source_state, Ok(crate::editor::DiskState::Unchanged)))
        {
            "PDF outdated"
        } else {
            "PDF current"
        };
        let page = self.active_preview().page_status().unwrap_or_default();
        let [label, current_tab, proposed_tab] = preview_header(area);
        let title_area = if self.session.proposal_review.is_some() {
            label
        } else {
            Rect { height: 1, ..area }
        };
        let zoom = self.active_preview().zoom_percent();
        let mut title = format!(" {state}  {page} · {zoom}%");
        if self.active_preview().is_preparing() {
            title.push_str(" · Updating…");
        }

        let zoom_hint = " · Ctrl+wheel / +/- zoom · 0 fit";
        if self.focus == Focus::Preview
            && title.chars().count() + zoom_hint.chars().count() <= usize::from(title_area.width)
        {
            title.push_str(zoom_hint);
        }
        frame.render_widget(
            Paragraph::new(title).style(theme::pane_title(self.focus == Focus::Preview)),
            title_area,
        );

        if self.session.proposal_review.is_some() {
            for (tab, text, selected) in [
                (current_tab, "[ Current ]", !proposed),
                (proposed_tab, "[ Proposed ]", proposed),
            ] {
                let style = if selected {
                    Style::default().fg(theme::BACKGROUND).bg(theme::LAVENDER)
                } else {
                    theme::muted()
                };
                frame.render_widget(Paragraph::new(text).style(style), tab);
            }
        }
    }

    pub(super) fn draw_diagnostics(&self, frame: &mut Frame, layout: &WorkspaceLayout) {
        if let Some(diagnostics) = &self.session.diagnostics {
            let log = Paragraph::new(diagnostics.as_str())
                .block(Block::default().title(" Build diagnostics · PgUp/PgDn scroll · Esc close "))
                .style(Style::default().fg(theme::ERROR).bg(theme::ELEVATED))
                .wrap(Wrap { trim: false })
                .scroll((self.session.diagnostic_scroll, 0));
            frame.render_widget(log, layout.diagnostics);
        }
    }
}

pub(super) fn below_header(area: Rect, height: u16) -> Rect {
    let height = height.min(area.height);
    Rect {
        y: area.y + height,
        height: area.height - height,
        ..area
    }
}

pub(super) fn review_buttons(area: Rect) -> [Rect; 3] {
    let row = Rect {
        x: area.x + theme::PANEL_PADDING.min(area.width),
        y: area.y + area.height.min(1),
        width: area.width.saturating_sub(theme::PANEL_PADDING * 2),
        height: area.height.saturating_sub(1).min(1),
    };

    Layout::horizontal([
        Constraint::Length(REVIEW_BUTTON_WIDTH),
        Constraint::Length(REVIEW_BUTTON_WIDTH),
        Constraint::Length(REVIEW_BUTTON_WIDTH),
    ])
    .areas(row)
}

pub(super) fn preview_header(area: Rect) -> [Rect; 3] {
    Layout::horizontal([
        Constraint::Min(0),
        Constraint::Length(PREVIEW_TAB_WIDTH),
        Constraint::Length(PREVIEW_TAB_WIDTH),
    ])
    .areas(Rect {
        height: area.height.min(1),
        ..area
    })
}

pub(super) fn pane_content(area: Rect) -> Rect {
    Block::bordered().inner(area)
}

fn draw_pane_border(frame: &mut Frame, area: Rect, focused: bool) -> Rect {
    let color = if focused {
        theme::ACCENT
    } else {
        theme::DIVIDER
    };
    frame.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(color)),
        area,
    );
    pane_content(area)
}

#[derive(Default)]
pub(super) struct WorkspaceLayout {
    pub(super) projects: Rect,
    pub(super) files: Rect,
    pub(super) editor: Rect,
    pub(super) assistant: Rect,
    pub(super) preview: Rect,
    pub(super) diagnostics: Rect,
    pub(super) status: Rect,
}

impl WorkspaceLayout {
    pub(super) fn new(
        area: Rect,
        show_diagnostics: bool,
        show_projects: bool,
        focus: Focus,
    ) -> Self {
        let [area, status] =
            Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(area);

        let diagnostics_height = if show_diagnostics {
            DIAGNOSTICS_HEIGHT.min(area.height / 2)
        } else {
            0
        };
        let [body, diagnostics] =
            Layout::vertical([Constraint::Min(0), Constraint::Length(diagnostics_height)])
                .areas(area);

        // A narrow terminal can still fit a vertical workspace. Keep chat visible and
        // use the upper pane for the editor, preview, or projects as focus changes.
        if area.width < MULTIPANE_WIDTH && body.height >= MULTIPANE_HEIGHT {
            let [upper, assistant] = Layout::vertical([
                Constraint::Percentage(EDITOR_PERCENT),
                Constraint::Min(MIN_CHAT_HEIGHT),
            ])
            .areas(body);
            let mut layout = Self {
                assistant,
                diagnostics,
                status,
                ..Self::default()
            };
            match focus {
                Focus::Preview => layout.preview = upper,
                Focus::Projects => {
                    let [resumes, files] =
                        Layout::vertical([Constraint::Percentage(50), Constraint::Min(1)])
                            .areas(upper);
                    layout.projects = resumes;
                    layout.files = files;
                }
                Focus::Editor | Focus::Assistant => layout.editor = upper,
            }
            return layout;
        }

        if area.width < MULTIPANE_WIDTH || body.height < MULTIPANE_HEIGHT {
            let mut layout = Self {
                diagnostics,
                status,
                ..Self::default()
            };
            match focus {
                Focus::Editor => layout.editor = body,
                Focus::Assistant => layout.assistant = body,
                Focus::Projects => layout.projects = body,
                Focus::Preview => layout.preview = body,
            }
            return layout;
        }

        let sidebar_width = if show_projects { SIDEBAR_WIDTH } else { 0 };
        let [sidebar, center, preview] = Layout::horizontal([
            Constraint::Length(sidebar_width),
            Constraint::Min(1),
            Constraint::Percentage(PREVIEW_PERCENT),
        ])
        .areas(body);
        let [editor, assistant] = Layout::vertical([
            Constraint::Percentage(EDITOR_PERCENT),
            Constraint::Min(MIN_CHAT_HEIGHT),
        ])
        .areas(center);
        let [projects, files] = Layout::vertical([
            Constraint::Percentage(EDITOR_PERCENT),
            Constraint::Min(MIN_CHAT_HEIGHT),
        ])
        .areas(sidebar);
        Self {
            projects,
            files,
            editor,
            assistant,
            preview,
            diagnostics,
            status,
        }
    }
}
