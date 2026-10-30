use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};
use texo::theme;

use super::{App, Focus, PaneSizes};

const MULTIPANE_WIDTH: u16 = 100;
pub(super) const MIN_PANE_WIDTH: u16 = 8;
pub(super) const MIN_EDITOR_HEIGHT: u16 = 3;
const MULTIPANE_HEIGHT: u16 = 20;
const SIDEBAR_WIDTH: u16 = 24;
const PREVIEW_PERCENT: u16 = 38;
const EDITOR_PERCENT: u16 = 55;
// Borders, padding, header, and composer must fit even with the status row visible.
pub(super) const MIN_CHAT_HEIGHT: u16 = 11;
const INLINE_PROBLEM_HEIGHT: u16 = 5;
const MIN_EDITOR_TITLE_WIDTH: u16 = 20;
const PREVIEW_TAB_WIDTH: u16 = 13;

impl App {
    pub(super) fn workspace_layout(&self, area: Rect) -> WorkspaceLayout {
        let mut layout =
            WorkspaceLayout::with_sizes(area, self.projects_visible, self.focus, &self.pane_sizes);

        // Column expansion keeps the PDF and sidebar in place.
        if !layout.editor.is_empty() && !layout.assistant.is_empty() {
            let column = layout.editor.union(layout.assistant);
            match self.session.column_expansion {
                Some(Focus::Editor) if self.focus != Focus::Assistant => {
                    layout.editor = column;
                    layout.assistant = Rect::default();
                }
                Some(Focus::Assistant) if self.focus != Focus::Editor => {
                    layout.assistant = column;
                    layout.editor = Rect::default();
                }
                _ => {}
            }
        }

        if let Some(focus) = self
            .workspace_expansion
            .filter(|focus| *focus == self.focus)
        {
            let content = [layout.editor, layout.assistant, layout.preview]
                .into_iter()
                .filter(|pane| !pane.is_empty())
                .reduce(|area, pane| area.union(pane))
                .unwrap_or_default();
            layout.editor = Rect::default();
            layout.assistant = Rect::default();
            layout.preview = Rect::default();

            match focus {
                Focus::Editor => layout.editor = content,
                Focus::Assistant => layout.assistant = content,
                Focus::Preview => layout.preview = content,
                Focus::Projects => {}
            }
        }

        if self.session.history.view.is_none()
            && self.session.diagnostics.is_some()
            && self.session.build_diagnostics.as_ref().is_some()
            && !self.session.summary_hidden
            && !layout.editor.is_empty()
        {
            let content = pane_content(layout.editor);
            let height = INLINE_PROBLEM_HEIGHT.min(content.height / 2);
            layout.diagnostics =
                Rect::new(content.x, content.bottom() - height, content.width, height);
        }

        layout
    }

    pub(super) fn editor_text_area(&self, layout: &WorkspaceLayout) -> Rect {
        let mut content = pane_content(layout.editor);
        if content.intersects(layout.diagnostics) {
            content.height = layout.diagnostics.y.saturating_sub(content.y);
        }
        content
    }

    pub(super) fn preview_area(&self, area: Rect) -> Rect {
        self.workspace_layout(area).preview
    }

    pub(super) fn preview_body_area(&self, area: Rect) -> Rect {
        let header_rows = if self.session.history.view.is_some() {
            2
        } else {
            1
        };
        below_header(pane_content(area), header_rows)
    }

    pub(super) fn draw(&mut self, frame: &mut Frame) {
        self.restore_hidden_column_pane();

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
        if self.projects_visible {
            self.draw_history_sidebar(frame, layout.files);
        }
        draw_pane_border(frame, layout.editor, self.focus == Focus::Editor);
        let editor = self.editor_text_area(&layout);
        let assistant = draw_pane_border(frame, layout.assistant, self.focus == Focus::Assistant);
        let preview = draw_pane_border(frame, layout.preview, self.focus == Focus::Preview);

        let source_state = self.session.editor.disk_state();
        self.draw_editor(frame, editor, &source_state);
        self.draw_assistant(frame, assistant);
        self.draw_preview(frame, preview, &source_state);
        self.draw_diagnostics(frame, &layout);

        self.draw_status_bar(frame, layout.status);

        if let Some(menu) = &mut self.actions {
            menu.draw(frame);
        }
        if let Some(export) = &mut self.export {
            export.draw(frame);
        }
        if let Some(settings) = &self.settings {
            settings.draw(frame);
        }
        if let Some(dialog) = &mut self.label_dialog {
            dialog.draw(frame);
        }
        if let Some(dialog) = &mut self.project_dialog {
            dialog.draw(frame);
        }
        if self.diagnostic_details_open() {
            self.draw_diagnostic_details(frame);
        }
        self.draw_history_dialog(frame);
        if self.shortcuts_open {
            self.draw_shortcut_help(frame);
        }
    }

    fn draw_status_bar(&self, frame: &mut Frame, area: Rect) {
        let prefix = self.prefix.label();
        let help = format!("{prefix} ?");
        let settings = format!("{prefix} s");
        let mut hint = theme::shortcuts(&[
            (&prefix, "Prefix"),
            (&help, "Shortcuts"),
            (&settings, "Settings"),
        ]);
        if hint.width() > usize::from(area.width) {
            hint = theme::shortcuts(&[(&help, "Help"), (&settings, "Settings")]);
        }
        if hint.width() > usize::from(area.width) {
            hint = theme::shortcuts(&[(&help, "Help")]);
        }

        frame.render_widget(
            Block::default().style(theme::muted().bg(theme::ELEVATED)),
            area,
        );
        let [status, controls] = Layout::horizontal([
            Constraint::Min(0),
            Constraint::Length(hint.width().min(usize::from(area.width)) as u16),
        ])
        .areas(area);

        // Review replaces the editor title, so retain operation results beside the controls.
        if self.session.proposal_review.is_some() || self.session.history.view.is_some() {
            frame.render_widget(
                Paragraph::new(self.session.status.as_str()).style(theme::muted()),
                status,
            );
        }
        frame.render_widget(Paragraph::new(hint).style(theme::muted()), controls);
    }

    pub(super) fn draw_editor(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        source_state: &anyhow::Result<crate::editor::DiskState>,
    ) {
        if self.session.history.view.is_some() {
            self.draw_historical_source(frame, area);
            return;
        }
        if self.session.empty_workspace {
            let prompt = vec![
                Line::from("Create or import a resume."),
                theme::shortcuts(&[
                    (&format!("{} n", self.prefix.label()), "New"),
                    (&format!("{} i", self.prefix.label()), "Import"),
                ]),
                Line::from("Existing loose TeX files stay untouched."),
            ];
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
            let focused = self.editor_has_focus();
            self.session.editor.draw(frame, area, focused);
            if self.diagnostics_current() {
                let lines: Vec<_> = self
                    .session
                    .build_diagnostics
                    .as_ref()
                    .into_iter()
                    .flat_map(|diagnostics| &diagnostics.problems)
                    .filter_map(|problem| problem.editor_line())
                    .collect();
                self.session.editor.draw_problems(frame, area, &lines);
            }
        }

        if !area.is_empty() {
            let modified = if self.session.editor.is_modified() {
                " · unsaved"
            } else {
                ""
            };
            let label = if !self.session.editor.path.is_file() {
                "Source missing · Save to restore source".into()
            } else if matches!(source_state, Ok(crate::editor::DiskState::Changed)) {
                "Source changed externally · Manage → Reload source".into()
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
            let button = self.editor_maximize_button(area);
            let label_area = Rect {
                width: button.x.saturating_sub(title.x),
                ..title
            };
            frame.render_widget(
                Paragraph::new(label).style(theme::pane_title(self.focus == Focus::Editor)),
                label_area,
            );
            frame.render_widget(
                Paragraph::new(self.editor_maximize_hint(area)).style(theme::muted()),
                button,
            );
        }
    }

    fn editor_maximize_hint(&self, area: Rect) -> Line<'static> {
        let title = area.inner(ratatui::layout::Margin::new(theme::PANEL_PADDING, 0));
        let action = if self.pane_is_expanded(Focus::Editor) {
            "Restore"
        } else {
            "Maximize"
        };
        let shortcut = format!("{} u", self.prefix.label());
        let full = theme::shortcuts(&[(&shortcut, action)]);

        // Keep short operation results readable beside the control in narrow panes.
        let compact = Line::from(theme::keycap(&shortcut));
        let available = title.width.saturating_sub(MIN_EDITOR_TITLE_WIDTH) as usize;
        if available >= full.width() {
            full
        } else if available >= compact.width() {
            compact
        } else {
            Line::default()
        }
    }

    pub(super) fn editor_maximize_button(&self, area: Rect) -> Rect {
        let title = area.inner(ratatui::layout::Margin::new(theme::PANEL_PADDING, 0));
        let width = (self.editor_maximize_hint(area).width() as u16).min(title.width);
        Rect::new(title.right() - width, title.y, width, title.height.min(1))
    }

    pub(super) fn review_buttons(&self, area: Rect) -> [Rect; 3] {
        Layout::horizontal([Constraint::Ratio(1, 3); 3])
            .areas(self.session.assistant.review_area(area))
    }

    pub(super) fn draw_assistant(&mut self, frame: &mut Frame, area: Rect) {
        let reviewing =
            self.session.proposal_review.is_some() && !self.session.assistant.login_open();
        self.session.assistant.draw(
            frame,
            area,
            self.focus == Focus::Assistant,
            self.pane_is_expanded(Focus::Assistant),
            &self.prefix.label(),
            reviewing,
        );

        if reviewing {
            let actions = [
                ('a', "Accept", theme::TEAL),
                ('m', "Modify", theme::ACCENT),
                ('d', "Discard", theme::ERROR),
            ];
            for (button, (key, label, color)) in self.review_buttons(area).into_iter().zip(actions)
            {
                let shortcut = format!("{} {key}", self.prefix.label());
                let mut text = Line::from(vec![
                    Span::raw(format!(" {label} ")),
                    theme::keycap(&shortcut),
                ]);
                if text.width() > usize::from(button.width) {
                    text = Line::from(format!(" {label} "));
                }
                frame.render_widget(
                    Paragraph::new(text)
                        .style(Style::default().fg(color).bg(theme::SELECTION).bold()),
                    button,
                );
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

        if self.session.history.view.is_some() {
            self.draw_history_pdf(frame, area);
            return;
        }
        let body = below_header(area, 1);
        let proposed = self.session.show_proposed && self.session.proposal_review.is_some();
        let building = if proposed {
            self.session.proposed_job.is_some()
        } else {
            self.session.job.is_some()
        };

        if building {
            self.active_preview().draw_loading(frame, body);
        } else if self.overlay_open() {
            // A finished proposal may switch versions while a dialog covers the upload cell.
            // Keep this resume's current image visible until the new version can be presented.
            let preview = if proposed && !self.session.proposed_preview.has_presented_image() {
                &mut self.session.preview
            } else {
                self.active_preview()
            };
            preview.draw_cached(frame, body);
        } else {
            self.active_preview().draw(frame, body);
        }

        let is_loading = building || self.active_preview().is_preparing();
        let state = if self.session.empty_workspace {
            "PDF missing"
        } else if is_loading {
            "PDF loading"
        } else if (proposed && self.session.proposed_render_failed)
            || (!proposed && self.session.render_failed)
        {
            if !proposed && self.session.rendered_pdf.is_some() {
                "PDF failed · showing previous build"
            } else {
                "PDF failed"
            }
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
        let title = format!(" {state}  {page} · {zoom}%");

        let compact_title = if state == "PDF failed" {
            " PDF failed".into()
        } else if state.starts_with("PDF failed") {
            " PDF failed · stale".into()
        } else {
            format!(" PDF {} {zoom}%", page.replace(' ', ""))
        };
        draw_preview_title(
            frame,
            title_area,
            title,
            compact_title,
            self.focus == Focus::Preview,
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
        if !self.session.summary_hidden
            && let Some(problems) = &self.session.build_diagnostics
        {
            self.draw_problems(frame, layout.diagnostics, problems);
        }
    }

    pub(super) fn problem_action_areas(&self, area: Rect) -> Vec<(Rect, char, String)> {
        let Some(diagnostics) = &self.session.build_diagnostics else {
            return Vec::new();
        };
        let mut actions = Vec::new();
        let current = self.diagnostics_current();
        if current {
            if diagnostics.selected().editor_line().is_some()
                && self.session.proposal_review.is_none()
            {
                actions.push(('G', "Go to line"));
            }
            if diagnostics.selected().resource.is_some() {
                actions.push(('R', "Download & retry"));
            }
            actions.push(('F', "Ask AI"));
        }
        actions.push(('E', "Details"));
        if diagnostics.problems.len() > 1 {
            actions.push(('[', "Previous"));
            actions.push((']', "Next"));
        }
        actions.push(('D', "Hide"));

        let mut result = Vec::new();
        let mut x = 0;
        let mut row = 0;
        for (key, label) in actions {
            let text = if result.is_empty() {
                format!(" {} {key} {label}  ", self.prefix.label())
            } else {
                format!(" {key} {label}  ")
            };
            let width = (Line::from(text.as_str()).width() as u16).min(area.width);
            if x > 0 && x + width > area.width {
                row += 1;
                x = 0;
            }
            result.push((Rect::new(area.x + x, row, width, 1), key, text));
            x += width;
        }

        let top = area.bottom().saturating_sub(row + 1).max(area.y);
        for (rect, _, _) in &mut result {
            rect.y += top;
            *rect = rect.intersection(area);
        }
        result
    }

    fn draw_problems(
        &self,
        frame: &mut Frame,
        area: Rect,
        diagnostics: &crate::build_diagnostics::BuildDiagnostics,
    ) {
        let current = self.diagnostics_current();
        let problem = diagnostics.selected();
        let version = if diagnostics.proposed {
            "Proposed"
        } else {
            "Current"
        };
        let state = if current {
            ""
        } else {
            " · outdated — rebuild to update"
        };
        let title = format!(
            " Problems · {version} · {}/{}{state}",
            diagnostics.selected + 1,
            diagnostics.problems.len()
        );
        let location = problem.location.as_deref().unwrap_or("Build process");
        let origin = problem
            .import_line
            .map(|line| format!("Imported at line {line} → "))
            .unwrap_or_default();
        let body = format!("{origin}{location}: {}", problem.message);
        let actions = self.problem_action_areas(area);
        let action_top = actions.first().map_or(area.bottom(), |(rect, _, _)| rect.y);
        let body_area = Rect {
            height: action_top.saturating_sub(area.y),
            ..area
        };
        let block =
            Block::default().title(Line::from(title).style(Style::default().fg(theme::ERROR)));
        frame.render_widget(
            Paragraph::new(body)
                .block(block)
                .style(theme::text().bg(theme::ELEVATED))
                .wrap(Wrap { trim: false }),
            body_area,
        );
        for (rect, _, label) in actions {
            frame.render_widget(
                Paragraph::new(label).style(theme::muted().bg(theme::ELEVATED)),
                rect,
            );
        }
    }
}

fn draw_preview_title(
    frame: &mut Frame,
    area: Rect,
    title: String,
    compact_title: String,
    focused: bool,
) {
    let mut title_area = area;
    if focused {
        let full_hint = theme::shortcuts(&[("Ctrl+wheel / +/-", "zoom"), ("0", "fit")]);
        let compact_hint = theme::shortcuts(&[("+/-", "zoom"), ("0", "fit")]);
        let hint = if Line::from(title.as_str()).width() + full_hint.width() + 2
            <= usize::from(area.width)
        {
            full_hint
        } else if compact_hint.width() + 2 <= usize::from(area.width) {
            compact_hint
        } else {
            theme::shortcuts(&[("0", "fit")])
        };

        // Reserve room for the controls; a split layout can give the PDF a narrow pane
        // even when the overall terminal is wide.
        let [label, controls] =
            Layout::horizontal([Constraint::Min(0), Constraint::Length(hint.width() as u16)])
                .spacing(2)
                .areas(area);
        title_area = label;
        frame.render_widget(Paragraph::new(hint).style(theme::muted()), controls);
    }

    let title = if Line::from(title.as_str()).width() <= usize::from(title_area.width) {
        title
    } else {
        compact_title
    };
    frame.render_widget(
        Paragraph::new(title).style(theme::pane_title(focused)),
        title_area,
    );
}

pub(super) fn below_header(area: Rect, height: u16) -> Rect {
    let height = height.min(area.height);
    Rect {
        y: area.y + height,
        height: area.height - height,
        ..area
    }
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
    #[cfg(test)]
    pub(super) fn new(area: Rect, show_projects: bool, focus: Focus) -> Self {
        Self::with_sizes(area, show_projects, focus, &PaneSizes::default())
    }

    pub(super) fn with_sizes(
        area: Rect,
        show_projects: bool,
        focus: Focus,
        sizes: &PaneSizes,
    ) -> Self {
        let [area, status] =
            Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(area);

        let body = area;
        let diagnostics = Rect::default();

        // A narrow terminal can still fit a vertical workspace. Keep chat visible and
        // use the upper pane for the editor, preview, or projects as focus changes.
        if area.width < MULTIPANE_WIDTH && body.height >= MULTIPANE_HEIGHT {
            let [upper, assistant] = Layout::vertical([
                editor_constraint(sizes, body.height),
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

        let sidebar_width = if show_projects {
            sizes
                .sidebar
                .unwrap_or(SIDEBAR_WIDTH)
                .clamp(MIN_PANE_WIDTH, body.width - 2 * MIN_PANE_WIDTH)
        } else {
            0
        };
        let preview_constraint = match sizes.preview {
            Some(width) => Constraint::Length(
                width.clamp(MIN_PANE_WIDTH, body.width - sidebar_width - MIN_PANE_WIDTH),
            ),
            None => Constraint::Percentage(PREVIEW_PERCENT),
        };
        let [sidebar, center, preview] = Layout::horizontal([
            Constraint::Length(sidebar_width),
            Constraint::Min(MIN_PANE_WIDTH),
            preview_constraint,
        ])
        .areas(body);
        let [editor, assistant] = Layout::vertical([
            editor_constraint(sizes, center.height),
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

// Retain the default ratio until resized, then clamp saved dimensions on terminal resize.
fn editor_constraint(sizes: &PaneSizes, height: u16) -> Constraint {
    match sizes.editor {
        Some(rows) => Constraint::Length(rows.clamp(MIN_EDITOR_HEIGHT, height - MIN_CHAT_HEIGHT)),
        None => Constraint::Percentage(EDITOR_PERCENT),
    }
}
