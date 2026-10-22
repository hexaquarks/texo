use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use ignore::WalkBuilder;
use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState},
};
use texo::theme;

const TREE_INDENT_WIDTH: usize = 2;
const TREE_ROW_HEIGHT: u16 = 2;

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
        for ancestor in active_file.ancestors().skip(1) {
            if ancestor.starts_with(&explorer.root) {
                explorer.expanded.insert(ancestor.to_path_buf());
            }
        }
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
        let mut entries = vec![Entry {
            path: self.root.clone(),
            directory: true,
            depth: 0,
        }];
        if self.expanded.contains(&self.root) {
            self.collect_children(&self.root, 1, &mut entries)?;
        }
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
                self.expanded.insert(path.clone());
            }
            self.refresh()?;
            // The workspace root is navigation only; other folders can open a project.
            return Ok((path != self.root).then_some(path));
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
            && parent.starts_with(&self.root)
        {
            self.select_path(parent);
        }
        Ok(())
    }

    pub fn select_at(&mut self, area: Rect, row: u16) -> bool {
        if row < area.y || row >= area.bottom() {
            return false;
        }
        let index = usize::from((row - area.y) / TREE_ROW_HEIGHT) + self.state.offset();
        if index < self.entries.len() {
            self.state.select(Some(index));
            return true;
        }
        false
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect, active_file: &Path, _focused: bool) {
        let panel = Block::default()
            .borders(Borders::RIGHT)
            .border_style(Style::default().fg(theme::DIVIDER))
            .style(Style::default().bg(theme::SIDEBAR));
        let inner = panel.inner(area);
        frame.render_widget(panel, area);
        let rows: Vec<ListItem> = self
            .entries
            .iter()
            .map(|entry| self.entry_item(entry, active_file))
            .collect();
        let list = List::new(rows).highlight_style(Style::default().bg(theme::SELECTION));
        frame.render_stateful_widget(list, inner, &mut self.state);
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
        let filename = if entry.path == self.root && self.name() == "resumes" {
            "My resumes".to_owned()
        } else {
            entry
                .path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        };
        let marker = if entry.directory { "▱ " } else { "▤ " };
        let mut style = Style::default().fg(theme::TEXT);
        if entry.path == active_file {
            style = style.bg(theme::SELECTION);
        }
        ListItem::new(vec![
            Line::from(vec![
                Span::raw(" ".repeat(entry.depth * TREE_INDENT_WIDTH)),
                Span::styled(icon, theme::muted()),
                Span::styled(marker, Style::default().fg(theme::FILE_ICON)),
                Span::raw(filename),
            ]),
            Line::raw(""),
        ])
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
        assert_eq!(explorer.activate()?, Some(root.join("sections")));
        explorer.select_path(&nested);
        assert_eq!(explorer.activate()?, Some(nested));
        Ok(())
    }

    #[test]
    fn reveals_active_document_and_collapses_workspace_root() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let root = directory.path().to_path_buf();
        fs::create_dir_all(root.join("AI engineering/sections"))?;
        let document = root.join("AI engineering/sections/main.tex");
        fs::write(&document, "Resume")?;
        let mut explorer = Explorer::new(root.clone(), &document)?;
        assert_eq!(explorer.selected_path(), Some(document.clone()));

        explorer.select_path(&root);
        explorer.activate()?;
        assert_eq!(explorer.entries.len(), 1);
        assert_eq!(explorer.selected_path(), Some(root));
        explorer.activate()?;
        assert!(explorer.entries.iter().any(|entry| entry.path == document));
        Ok(())
    }

    #[test]
    fn mouse_selection_accounts_for_row_spacing_and_scrolling() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let document = directory.path().join("resume.tex");
        fs::write(&document, "Resume")?;
        let mut explorer = Explorer::new(directory.path().to_path_buf(), &document)?;
        let area = Rect::new(0, 3, 30, 10);

        assert!(!explorer.select_at(area, 2));
        assert!(explorer.select_at(area, 6));
        assert_eq!(explorer.selected_path(), Some(document.clone()));
        *explorer.state.offset_mut() = 1;
        assert!(explorer.select_at(area, 3));
        assert_eq!(explorer.selected_path(), Some(document));
        assert!(!explorer.select_at(area, 13));
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
