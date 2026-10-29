use ratatui::layout::Rect;

use super::{
    App, Focus,
    workspace_view::{MIN_CHAT_HEIGHT, MIN_EDITOR_HEIGHT, MIN_PANE_WIDTH, WorkspaceLayout},
};

pub(super) const PANE_RESIZE_CELLS: u16 = 10;

impl App {
    pub(super) fn select_directional_pane(&mut self, direction: char, area: Rect) {
        self.workspace_expansion = None;

        // Navigation toward a hidden sibling first restores the column.
        let hidden_sibling = match (self.session.column_expansion, self.focus, direction) {
            (Some(Focus::Assistant), Focus::Assistant, 'k') => Some(Focus::Editor),
            (Some(Focus::Editor), Focus::Editor, 'j') => Some(Focus::Assistant),
            _ => None,
        };
        if let Some(focus) = hidden_sibling {
            self.session.column_expansion = None;
            self.focus = focus;
            return;
        }

        let layout = self.workspace_layout(area);
        if let Some((focus, _)) = neighboring_pane(&layout, self.focus, direction) {
            self.focus = focus;
            self.restore_hidden_column_pane();
        }
    }

    pub(super) fn toggle_workspace_expansion(&mut self) {
        if self.focus == Focus::Projects {
            return;
        }

        self.workspace_expansion = if self.workspace_expansion == Some(self.focus) {
            None
        } else {
            Some(self.focus)
        };
    }

    pub(super) fn restore_workspace_panes(&mut self) {
        self.workspace_expansion = None;
        self.session.column_expansion = None;
    }

    pub(super) fn toggle_column_expansion(&mut self) {
        self.workspace_expansion = None;
        if !matches!(self.focus, Focus::Editor | Focus::Assistant) {
            return;
        }

        self.session.column_expansion = if self.session.column_expansion == Some(self.focus) {
            None
        } else {
            Some(self.focus)
        };
    }

    pub(super) fn restore_hidden_column_pane(&mut self) {
        if matches!(
            (self.session.column_expansion, self.focus),
            (Some(Focus::Editor), Focus::Assistant) | (Some(Focus::Assistant), Focus::Editor)
        ) {
            self.session.column_expansion = None;
        }
    }

    pub(super) fn resize_directional_pane(&mut self, direction: char, area: Rect) {
        self.workspace_expansion = None;
        let previous_sizes = self.pane_sizes;
        let layout = self.workspace_layout(area);

        match direction {
            'H' | 'L' => self.move_horizontal_divider(&layout, direction == 'L'),
            'J' | 'K' => self.move_vertical_divider(&layout, direction == 'J'),
            _ => {}
        }

        if self.pane_sizes != previous_sizes {
            self.save_layout();
        }
    }

    fn move_horizontal_divider(&mut self, layout: &WorkspaceLayout, right: bool) {
        // Like tmux, use the pane's right divider, falling back to its left divider
        // at the outer edge. The direction moves the divider, rather than growing the pane.
        let center = if !layout.editor.is_empty() {
            layout.editor
        } else {
            layout.assistant
        };
        if center.is_empty() {
            return;
        }

        if self.focus == Focus::Projects && layout.projects.right() == center.x {
            self.pane_sizes.sidebar = Some(moved_divider_size(
                layout.projects.width,
                center.width,
                MIN_PANE_WIDTH,
                MIN_PANE_WIDTH,
                right,
            ));
        } else if !layout.preview.is_empty() && center.right() == layout.preview.x {
            let center_width = moved_divider_size(
                center.width,
                layout.preview.width,
                MIN_PANE_WIDTH,
                MIN_PANE_WIDTH,
                right,
            );
            self.pane_sizes.preview = Some(center.width + layout.preview.width - center_width);
        } else if !layout.projects.is_empty() && layout.projects.right() == center.x {
            self.pane_sizes.sidebar = Some(moved_divider_size(
                layout.projects.width,
                center.width,
                MIN_PANE_WIDTH,
                MIN_PANE_WIDTH,
                right,
            ));
        }
    }

    fn move_vertical_divider(&mut self, layout: &WorkspaceLayout, down: bool) {
        if !matches!(self.focus, Focus::Editor | Focus::Assistant)
            || layout.editor.is_empty()
            || layout.assistant.is_empty()
        {
            return;
        }

        self.pane_sizes.editor = Some(moved_divider_size(
            layout.editor.height,
            layout.assistant.height,
            MIN_EDITOR_HEIGHT,
            MIN_CHAT_HEIGHT,
            down,
        ));
    }
}

// Return the new size before the divider, preserving space for both panes.
fn moved_divider_size(
    before: u16,
    after: u16,
    minimum_before: u16,
    minimum_after: u16,
    forward: bool,
) -> u16 {
    if forward {
        before + PANE_RESIZE_CELLS.min(after.saturating_sub(minimum_after))
    } else {
        before - PANE_RESIZE_CELLS.min(before.saturating_sub(minimum_before))
    }
}

fn pane_areas(layout: &WorkspaceLayout) -> [(Focus, Rect); 4] {
    let projects = if layout.files.is_empty() {
        layout.projects
    } else {
        layout.projects.union(layout.files)
    };

    [
        (Focus::Projects, projects),
        (Focus::Editor, layout.editor),
        (Focus::Assistant, layout.assistant),
        (Focus::Preview, layout.preview),
    ]
}

fn neighboring_pane(
    layout: &WorkspaceLayout,
    focus: Focus,
    direction: char,
) -> Option<(Focus, Rect)> {
    let panes = pane_areas(layout);
    let (_, current) = panes.iter().find(|(candidate, _)| *candidate == focus)?;
    if current.is_empty() {
        return None;
    }

    panes
        .into_iter()
        .filter(|(candidate, pane)| {
            if *candidate == focus || pane.is_empty() {
                return false;
            }

            let overlaps_rows = pane.y < current.bottom() && current.y < pane.bottom();
            let overlaps_columns = pane.x < current.right() && current.x < pane.right();
            match direction {
                'h' => pane.right() == current.x && overlaps_rows,
                'l' => pane.x == current.right() && overlaps_rows,
                'k' => pane.bottom() == current.y && overlaps_columns,
                'j' => pane.y == current.bottom() && overlaps_columns,
                _ => false,
            }
        })
        .min_by_key(|(_, pane)| {
            // A full-height pane can touch both editor and chat; choose the nearer center.
            if matches!(direction, 'h' | 'l') {
                pane.y
                    .saturating_add(pane.height / 2)
                    .abs_diff(current.y + current.height / 2)
            } else {
                pane.x
                    .saturating_add(pane.width / 2)
                    .abs_diff(current.x + current.width / 2)
            }
        })
}
