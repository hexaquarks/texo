use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use ignore::WalkBuilder;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, List, ListItem, ListState, Padding, Paragraph},
};
use texo::theme;

const HEADER_HEIGHT: u16 = 3;
const ITEM_HEIGHT: u16 = 3;

pub struct Projects {
    root: PathBuf,
    entries: Vec<PathBuf>,
    state: ListState,
}

impl Projects {
    pub fn new(root: PathBuf, active_file: &Path) -> Result<Self> {
        let mut projects = Self {
            root,
            entries: Vec::new(),
            state: ListState::default(),
        };
        projects.refresh()?;
        projects.select_path(active_file);
        Ok(projects)
    }

    pub fn refresh(&mut self) -> Result<()> {
        let selected = self.selected_path();
        let mut entries = Vec::new();
        for entry in WalkBuilder::new(&self.root)
            .max_depth(Some(1))
            .follow_links(false)
            .build()
            .skip(1)
        {
            let entry = entry.context("Could not read projects")?;
            let directory = entry.file_type().is_some_and(|kind| kind.is_dir());
            let document = entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "tex");
            if directory || document {
                entries.push(entry.into_path());
            }
        }
        entries.sort();
        self.entries = entries;
        if let Some(path) = selected {
            self.select_path(&path);
        }
        if self
            .state
            .selected()
            .is_none_or(|index| index >= self.entries.len())
        {
            self.state.select((!self.entries.is_empty()).then_some(0));
        }
        Ok(())
    }

    pub fn select_path(&mut self, path: &Path) {
        let index = self
            .entries
            .iter()
            .position(|entry| entry == path || (entry.is_dir() && path.starts_with(entry)));
        if index.is_some() {
            self.state.select(index);
        }
    }

    pub fn selected_path(&self) -> Option<PathBuf> {
        self.state
            .selected()
            .and_then(|index| self.entries.get(index))
            .cloned()
    }

    pub fn move_selection(&mut self, down: bool) {
        if self.entries.is_empty() {
            return;
        }
        let current = self.state.selected().unwrap_or(0);
        let next = if down {
            (current + 1).min(self.entries.len() - 1)
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
        if index >= self.entries.len() {
            return false;
        }
        self.state.select(Some(index));
        true
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect, active_file: &Path, focused: bool) {
        frame.render_widget(Block::default().style(theme::text()), area);
        let [header, _] =
            Layout::vertical([Constraint::Length(HEADER_HEIGHT), Constraint::Min(0)]).areas(area);
        frame.render_widget(
            Paragraph::new(vec![
                Line::from("texo").style(Style::default().fg(theme::ACCENT).bold()),
                Line::from("resumes").style(theme::muted()),
            ])
            .block(Block::default().padding(Padding::horizontal(2))),
            header,
        );
        let selected = self.selected_path();
        let items = self
            .entries
            .iter()
            .map(|path| {
                let active = if focused {
                    selected.as_ref() == Some(path)
                } else {
                    path == active_file || (path.is_dir() && active_file.starts_with(path))
                };
                let background = if active {
                    theme::SELECTION
                } else {
                    theme::SIDEBAR
                };
                let style = Style::default().fg(theme::TEXT).bg(background);
                let name = if path.is_dir() {
                    path.file_name()
                } else {
                    path.file_stem()
                }
                .unwrap_or(path.as_os_str())
                .to_string_lossy();
                let detail = if path.is_dir() {
                    "project".to_owned()
                } else {
                    path.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned()
                };
                ListItem::new(vec![
                    Line::from(vec![
                        Span::styled("  · ", Style::default().fg(theme::TEAL)),
                        Span::styled(name.into_owned(), Style::default().bold()),
                    ])
                    .style(style),
                    Line::from(format!("    {detail}")).style(style.fg(if active {
                        theme::LAVENDER
                    } else {
                        theme::MUTED
                    })),
                    Line::default(),
                ])
            })
            .collect::<Vec<_>>();
        frame.render_stateful_widget(List::new(items), list_area(area), &mut self.state);
    }
}

fn list_area(area: Rect) -> Rect {
    let [_, list] =
        Layout::vertical([Constraint::Length(HEADER_HEIGHT), Constraint::Min(0)]).areas(area);
    list
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn lists_projects_and_tex_documents_without_generated_files() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let project = directory.path().join("Backend");
        fs::create_dir(&project)?;
        let document = project.join("main.tex");
        fs::write(&document, "Resume")?;
        fs::write(directory.path().join("main.pdf"), "PDF")?;
        fs::write(directory.path().join("other.tex"), "Other")?;
        let mut projects = Projects::new(directory.path().to_path_buf(), &document)?;
        assert_eq!(projects.entries.len(), 2);
        assert_eq!(projects.selected_path(), Some(project));
        assert!(!projects.select_at(Rect::new(0, 0, 30, 20), 1));
        assert!(projects.select_at(Rect::new(0, 0, 30, 20), HEADER_HEIGHT + ITEM_HEIGHT));
        assert_eq!(
            projects.selected_path(),
            Some(directory.path().join("other.tex"))
        );
        Ok(())
    }
}
