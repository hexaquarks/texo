use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use ignore::WalkBuilder;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
};
use texo::theme;

const TREE_INDENT_WIDTH: usize = 2;
const EXPLORER_HEADER_HEIGHT: u16 = 2;
const WORKSPACE_HEADER_HEIGHT: u16 = 1;

struct Entry {
    path: PathBuf,
    directory: bool,
    depth: usize,
}

pub struct Explorer {
    root: PathBuf,
    expanded: HashSet<PathBuf>,
    entries: Vec<Entry>,
    state: ListState,
}

impl Explorer {
    pub fn new(root: PathBuf, active_file: &Path) -> Result<Self> {
        let mut explorer = Self {
            expanded: HashSet::from([root.clone()]),
            root,
            entries: Vec::new(),
            state: ListState::default(),
        };
        explorer.refresh()?;
        explorer.select_path(active_file);
        Ok(explorer)
    }

    pub fn name(&self) -> String {
        self.root
            .file_name()
            .unwrap_or(self.root.as_os_str())
            .to_string_lossy()
            .into_owned()
    }

    pub fn refresh(&mut self) -> Result<()> {
        let selected_path = self.selected_path();
        let mut entries = Vec::new();
        self.collect_children(&self.root, 0, &mut entries)?;
        self.entries = entries;
        if let Some(path) = selected_path {
            self.select_path(&path);
        }
        if self.state.selected().is_none() && !self.entries.is_empty() {
            self.state.select(Some(0));
        }
        Ok(())
    }

    // Scan only expanded directories so large project trees do not block startup.
    fn collect_children(
        &self,
        directory: &Path,
        depth: usize,
        entries: &mut Vec<Entry>,
    ) -> Result<()> {
        let mut children = WalkBuilder::new(directory)
            .max_depth(Some(1))
            .hidden(false)
            .follow_links(false)
            .build()
            .skip(1)
            .map(|entry| {
                let entry = entry.context("Could not read the workspace directory")?;
                Ok(Entry {
                    path: entry.path().to_path_buf(),
                    directory: entry.file_type().is_some_and(|kind| kind.is_dir()),
                    depth,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        children.sort_by(|left, right| {
            right
                .directory
                .cmp(&left.directory)
                .then_with(|| left.path.file_name().cmp(&right.path.file_name()))
        });

        for child in children {
            let expanded = child.directory && self.expanded.contains(&child.path);
            let path = child.path.clone();
            entries.push(child);
            if expanded {
                self.collect_children(&path, depth + 1, entries)?;
            }
        }
        Ok(())
    }

    pub fn select_path(&mut self, path: &Path) {
        self.state
            .select(self.entries.iter().position(|entry| entry.path == path));
    }

    pub fn selected_path(&self) -> Option<PathBuf> {
        self.state
            .selected()
            .and_then(|index| self.entries.get(index))
            .map(|entry| entry.path.clone())
    }

    pub fn move_selection(&mut self, down: bool) {
        if self.entries.is_empty() {
            return;
        }
        let next = match self.state.selected() {
            None => 0,
            Some(selected) if down => (selected + 1).min(self.entries.len() - 1),
            Some(selected) => selected.saturating_sub(1),
        };
        self.state.select(Some(next));
    }

    pub fn activate(&mut self) -> Result<Option<PathBuf>> {
        let Some(index) = self.state.selected() else {
            return Ok(None);
        };
        let Some(entry) = self.entries.get(index) else {
            return Ok(None);
        };
        let path = entry.path.clone();
        if entry.directory {
            if !self.expanded.remove(&path) {
                self.expanded.insert(path);
            }
            self.refresh()?;
            return Ok(None);
        }
        Ok(Some(path))
    }

    pub fn expand_selected(&mut self) -> Result<()> {
        if let Some(path) = self.selected_path()
            && self
                .entries
                .iter()
                .any(|entry| entry.path == path && entry.directory)
        {
            self.expanded.insert(path);
            self.refresh()?;
        }
        Ok(())
    }

    pub fn collapse_selected(&mut self) -> Result<()> {
        let Some(path) = self.selected_path() else {
            return Ok(());
        };
        if self.expanded.remove(&path) {
            self.refresh()?;
        } else if let Some(parent) = path.parent()
            && parent != self.root
        {
            self.select_path(parent);
        }
        Ok(())
    }

    pub fn select_at(&mut self, area: Rect, row: u16) -> bool {
        let header_height = EXPLORER_HEADER_HEIGHT + WORKSPACE_HEADER_HEIGHT;
        if row < area.y + header_height {
            return false;
        }
        let index = usize::from(row - area.y - header_height) + self.state.offset();
        if index < self.entries.len() {
            self.state.select(Some(index));
            return true;
        }
        false
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect, active_file: &Path, focused: bool) {
        let panel = Block::default()
            .borders(Borders::RIGHT)
            .border_style(Style::default().fg(theme::DIVIDER))
            .style(Style::default().bg(theme::SIDEBAR));
        let inner = panel.inner(area);
        frame.render_widget(panel, area);
        let [header, workspace, tree] = Layout::vertical([
            Constraint::Length(EXPLORER_HEADER_HEIGHT),
            Constraint::Length(WORKSPACE_HEADER_HEIGHT),
            Constraint::Min(0),
        ])
        .areas(inner);
        let heading_style = if focused {
            Style::default().fg(theme::TEXT)
        } else {
            theme::muted()
        };
        frame.render_widget(
            Paragraph::new(" EXPLORER  ·  F5 refresh").style(heading_style),
            header,
        );
        frame.render_widget(
            Paragraph::new(format!("⌄ {}", self.name().to_uppercase()))
                .style(theme::muted().bold()),
            workspace,
        );

        let rows: Vec<ListItem> = self
            .entries
            .iter()
            .map(|entry| self.entry_item(entry, active_file))
            .collect();
        let highlight = if focused {
            theme::SELECTION
        } else {
            theme::ELEVATED
        };
        let list = List::new(rows).highlight_style(Style::default().bg(highlight));
        frame.render_stateful_widget(list, tree, &mut self.state);
    }

    fn entry_item(&self, entry: &Entry, active_file: &Path) -> ListItem<'static> {
        let expanded = self.expanded.contains(&entry.path);
        let icon = if !entry.directory {
            "  "
        } else if expanded {
            "⌄ "
        } else {
            "› "
        };
        let filename = entry.path.file_name().unwrap_or_default().to_string_lossy();
        let marker = match entry
            .path
            .extension()
            .and_then(|extension| extension.to_str())
        {
            Some("tex") => "TᴇX ",
            Some("pdf") => "PDF ",
            _ => "    ",
        };
        let mut style = Style::default().fg(theme::TEXT);
        if entry.path == active_file {
            style = style.bg(theme::ELEVATED);
        }
        ListItem::new(Line::from(vec![
            Span::raw(" ".repeat(entry.depth * TREE_INDENT_WIDTH)),
            Span::styled(icon, theme::muted()),
            Span::styled(marker, Style::default().fg(theme::FILE_ICON)),
            Span::raw(filename.into_owned()),
        ]))
        .style(style)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn expands_directories_and_ignores_generated_files() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let root = directory.path().to_path_buf();
        fs::create_dir(root.join(".git"))?;
        fs::write(root.join(".gitignore"), "generated/\n")?;
        fs::create_dir(root.join("generated"))?;
        fs::write(root.join("generated/ignored.tex"), "ignored")?;
        fs::create_dir(root.join("sections"))?;
        let nested = root.join("sections/experience.tex");
        fs::write(&nested, "Experience")?;
        let document = root.join("resume.tex");
        fs::write(&document, "Resume")?;
        let mut explorer = Explorer::new(root.clone(), &document)?;

        assert!(
            !explorer
                .entries
                .iter()
                .any(|entry| entry.path.starts_with(root.join("generated")))
        );
        assert!(!explorer.entries.iter().any(|entry| entry.path == nested));
        explorer.select_path(&root.join("sections"));
        assert!(explorer.activate()?.is_none());
        explorer.select_path(&nested);
        assert_eq!(explorer.activate()?, Some(nested));
        Ok(())
    }

    #[test]
    fn selection_stays_within_the_visible_tree() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let document = directory.path().join("resume.tex");
        fs::write(&document, "Resume")?;
        let mut explorer = Explorer::new(directory.path().to_path_buf(), &document)?;
        explorer.move_selection(false);
        explorer.move_selection(true);
        assert_eq!(explorer.selected_path(), Some(document));
        Ok(())
    }
}
