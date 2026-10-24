use std::path::{Path, PathBuf};

use anyhow::Result;
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Margin, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, List, ListItem, ListState, Paragraph},
};
use texo::theme;

use crate::resume_project::{self, Catalog, Project};

const HEADER_HEIGHT: u16 = 4;
const SECTION_HEADER_GAP: u16 = 1;
const FILES_LABEL_WIDTH: u16 = 6;
const CONTENT_INSET: u16 = 1;
const ITEM_HEIGHT: u16 = 1;

pub struct Projects {
    root: PathBuf,
    pub catalog: Catalog,
    state: ListState,
    pub files_focused: bool,
}

impl Projects {
    pub fn new(root: PathBuf, active_file: &Path) -> Result<Self> {
        let mut projects = Self {
            root,
            catalog: Catalog::default(),
            state: ListState::default(),
            files_focused: false,
        };
        projects.refresh()?;
        projects.select_path(active_file);
        Ok(projects)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn refresh(&mut self) -> Result<()> {
        let selected = self.selected_path();
        self.catalog = resume_project::discover(&self.root)?;
        if let Some(path) = selected {
            self.select_path(&path);
        }
        if self
            .state
            .selected()
            .is_none_or(|index| index >= self.catalog.projects.len())
        {
            self.state
                .select((!self.catalog.projects.is_empty()).then_some(0));
        }
        Ok(())
    }

    pub fn project_for(&self, path: &Path) -> Option<&Project> {
        self.catalog
            .projects
            .iter()
            .find(|project| project.document() == path)
    }

    pub fn select_path(&mut self, path: &Path) {
        if let Some(index) = self
            .catalog
            .projects
            .iter()
            .position(|project| project.document() == path)
        {
            self.state.select(Some(index));
        }
    }

    pub fn selected_path(&self) -> Option<PathBuf> {
        self.state
            .selected()
            .and_then(|index| self.catalog.projects.get(index))
            .map(Project::document)
    }

    pub fn move_selection(&mut self, down: bool) {
        if self.catalog.projects.is_empty() || self.files_focused {
            return;
        }
        let current = self.state.selected().unwrap_or(0);
        let next = if down {
            (current + 1).min(self.catalog.projects.len() - 1)
        } else {
            current.saturating_sub(1)
        };
        self.state.select(Some(next));
    }

    pub fn select_at(&mut self, area: Rect, row: u16) -> bool {
        let content = list_area(area);
        if row < content.y || row >= content.bottom() {
            return false;
        }
        let index = usize::from((row - content.y) / ITEM_HEIGHT) + self.state.offset();
        if index >= self.catalog.projects.len() {
            return false;
        }
        self.state.select(Some(index));
        self.files_focused = false;
        true
    }

    pub fn new_button(area: Rect) -> Rect {
        Rect::new(
            area.right().saturating_sub(3),
            area.y.saturating_add(HEADER_HEIGHT),
            area.width.min(1),
            area.height.saturating_sub(HEADER_HEIGHT).min(1),
        )
    }

    pub fn draw(
        &mut self,
        frame: &mut Frame,
        resumes: Rect,
        files: Rect,
        active_file: &Path,
        focused: bool,
    ) {
        let sidebar = if files.is_empty() {
            resumes
        } else {
            resumes.union(files)
        };
        frame.render_widget(Block::default().style(theme::text()), sidebar);

        let inner = resumes.inner(Margin::new(CONTENT_INSET, 1));
        let header = Rect {
            height: HEADER_HEIGHT.min(inner.height),
            ..inner
        };
        frame.render_widget(
            Paragraph::new(vec![
                Line::styled(" texo", theme::pane_title(true)),
                Line::styled(
                    format!(" {}", crate::workspace::display_path(&self.root)),
                    theme::muted(),
                ),
                Line::default(),
                Line::styled("RESUMES", theme::pane_title(focused && !self.files_focused)),
            ]),
            header,
        );
        frame.render_widget(
            Paragraph::new("+").style(theme::pane_title(true)),
            Self::new_button(resumes),
        );

        let items: Vec<_> = self
            .catalog
            .projects
            .iter()
            .map(|project| {
                ListItem::new(Line::from(vec![
                    Span::styled(" • ", Style::default().fg(theme::ACCENT)),
                    Span::raw(project.manifest.name.clone()),
                ]))
            })
            .collect();
        frame.render_stateful_widget(
            List::new(items)
                .style(theme::text())
                .highlight_style(Style::default().bg(theme::SELECTION)),
            list_area(resumes),
            &mut self.state,
        );

        let note = if !self.catalog.warnings.is_empty() {
            format!("{} invalid projects · Actions", self.catalog.warnings.len())
        } else if self.catalog.projects.is_empty() {
            "No resumes yet".into()
        } else {
            String::new()
        };
        if inner.height > HEADER_HEIGHT {
            frame.render_widget(
                Paragraph::new(note).style(theme::muted()),
                Rect::new(inner.x, inner.bottom() - 1, inner.width, 1),
            );
        }

        let files_inner = files.inner(Margin::new(CONTENT_INSET, 1));
        let project = self.project_for(active_file);
        let name = project
            .map(|project| project.manifest.name.as_str())
            .unwrap_or("");
        let [label, resume_name] =
            Layout::horizontal([Constraint::Length(FILES_LABEL_WIDTH), Constraint::Min(0)]).areas(
                Rect {
                    height: files_inner.height.min(1),
                    ..files_inner
                },
            );
        frame.render_widget(
            Paragraph::new("FILES").style(theme::pane_title(focused && self.files_focused)),
            label,
        );
        frame.render_widget(
            Paragraph::new(name)
                .alignment(Alignment::Right)
                .style(theme::muted()),
            resume_name,
        );

        let entry_offset = 1 + SECTION_HEADER_GAP;
        if let Some(project) = project
            && files_inner.height > entry_offset
        {
            let entry = Line::from(vec![
                Span::styled(" • ", Style::default().fg(theme::ACCENT)),
                Span::raw(project.manifest.entrypoint.display().to_string()),
            ]);
            frame.render_widget(
                Paragraph::new(entry).style(theme::text().bg(theme::SELECTION)),
                Rect::new(
                    files_inner.x,
                    files_inner.y + entry_offset,
                    files_inner.width,
                    ITEM_HEIGHT.min(files_inner.height - entry_offset),
                ),
            );
        }
    }
}

fn list_area(area: Rect) -> Rect {
    let inner = area.inner(Margin::new(CONTENT_INSET, 1));
    let header = (HEADER_HEIGHT + SECTION_HEADER_GAP).min(inner.height);
    Rect {
        y: inner.y + header,
        height: inner.height.saturating_sub(header + 1),
        ..inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn split_sidebar_keeps_selection_visible_without_keyboard_focus() -> Result<()> {
        let workspace = tempfile::tempdir()?;
        let active = resume_project::create(workspace.path(), "AI engineer")?;
        for name in ["General", "Backend", "C++ / Systems"] {
            resume_project::create(workspace.path(), name)?;
        }
        std::fs::write(active.root.join("notes.tex"), "supporting file")?;
        std::fs::write(active.root.join("main.pdf"), "generated PDF")?;
        std::fs::write(workspace.path().join("legacy.tex"), "legacy")?;
        let mut projects = Projects::new(workspace.path().to_path_buf(), &active.document())?;
        let resumes = Rect::new(0, 0, 24, 18);
        let files = Rect::new(0, 18, 24, 14);
        let mut terminal = Terminal::new(TestBackend::new(24, 32))?;

        terminal.draw(|frame| projects.draw(frame, resumes, files, &active.document(), false))?;
        let buffer = terminal.backend().buffer();
        let rows: Vec<String> = (0..32)
            .map(|y| (0..24).map(|x| buffer[(x, y)].symbol()).collect())
            .collect();
        let text = rows.join("\n");
        assert!(
            text.contains("RESUMES")
                && text.contains("FILES")
                && rows[usize::from(files.y + 1)].ends_with("AI engineer ")
        );
        assert!(text.contains("main.tex"));
        assert!(!text.contains("loose TeX") && !text.contains("Import"));
        assert!(
            rows[usize::from(list_area(resumes).y - 1)]
                .trim()
                .is_empty()
        );
        assert!(rows[usize::from(files.y + 2)].trim().is_empty());
        assert_eq!(
            buffer[(CONTENT_INSET + 1, list_area(resumes).y)].symbol(),
            "•"
        );
        assert_eq!(
            buffer[(CONTENT_INSET + 1, list_area(resumes).y)].fg,
            theme::ACCENT
        );
        assert!(!text.contains("notes.tex") && !text.contains("main.pdf"));
        assert_eq!(
            buffer[(CONTENT_INSET, list_area(resumes).y)].bg,
            theme::SELECTION
        );
        assert_eq!(buffer[(0, 0)].symbol(), " ");
        assert_eq!(buffer[(0, files.y)].symbol(), " ");
        assert_eq!(buffer[(23, files.y)].symbol(), " ");
        assert_eq!(buffer[(0, files.y - 1)].symbol(), " ");
        assert_eq!(buffer[(CONTENT_INSET, list_area(resumes).y)].symbol(), " ");
        assert_eq!(
            buffer[(CONTENT_INSET + 3, list_area(resumes).y)].symbol(),
            "A"
        );
        assert_ne!(
            buffer[(CONTENT_INSET, list_area(resumes).y + ITEM_HEIGHT)].bg,
            theme::SELECTION
        );

        terminal.draw(|frame| projects.draw(frame, resumes, files, &active.document(), true))?;
        let buffer = terminal.backend().buffer();
        assert_eq!(
            buffer[(CONTENT_INSET, list_area(resumes).y)].bg,
            theme::SELECTION
        );
        assert_eq!(buffer[(0, 0)].symbol(), " ");
        assert!(projects.select_at(resumes, list_area(resumes).y + ITEM_HEIGHT));
        assert_ne!(projects.selected_path(), Some(active.document()));
        Ok(())
    }
}
