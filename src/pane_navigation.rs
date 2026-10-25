use ratatui::layout::Rect;

use super::{
    App, Focus,
    workspace_view::{MIN_CHAT_HEIGHT, MIN_EDITOR_HEIGHT, MIN_PANE_WIDTH, WorkspaceLayout},
};

pub(super) const PANE_RESIZE_PIXELS: u16 = 15;

impl App {
    pub(super) fn select_directional_pane(&mut self, direction: char, area: Rect) {
        // Restore the upper pane before navigating out of maximized chat.
        if self.session.chat_maximized && self.focus == Focus::Assistant && direction == 'k' {
            self.session.chat_maximized = false;
            self.focus = Focus::Editor;
            return;
        }

        let layout = self.workspace_layout(area);
        if let Some((focus, _)) = neighboring_pane(&layout, self.focus, direction) {
            self.focus = focus;
            if focus == Focus::Editor {
                self.session.chat_maximized = false;
            }
        }
    }

    pub(super) fn resize_directional_pane(&mut self, direction: char, area: Rect) {
        let layout = self.workspace_layout(area);
        let direction = direction.to_ascii_lowercase();
        let Some((neighbor, neighbor_area)) = neighboring_pane(&layout, self.focus, direction)
        else {
            return;
        };

        let (cell_width, cell_height) = self.session.preview.cell_size_pixels();
        let horizontal = matches!(direction, 'h' | 'l');
        let cell_pixels = if horizontal { cell_width } else { cell_height };
        let requested_cells = PANE_RESIZE_PIXELS.div_ceil(cell_pixels.max(1));

        if horizontal {
            let step = requested_cells.min(neighbor_area.width.saturating_sub(MIN_PANE_WIDTH));
            match (self.focus, neighbor) {
                (Focus::Projects, _) => {
                    self.pane_sizes.sidebar = Some(layout.projects.width + step);
                }
                (_, Focus::Projects) => {
                    self.pane_sizes.sidebar = Some(layout.projects.width - step);
                }
                (Focus::Preview, _) => {
                    self.pane_sizes.preview = Some(layout.preview.width + step);
                }
                (_, Focus::Preview) => {
                    self.pane_sizes.preview = Some(layout.preview.width - step);
                }
                _ => {}
            }
        } else {
            let minimum = if neighbor == Focus::Assistant {
                MIN_CHAT_HEIGHT
            } else {
                MIN_EDITOR_HEIGHT
            };
            let step = requested_cells.min(neighbor_area.height.saturating_sub(minimum));
            match (self.focus, neighbor) {
                (Focus::Editor, Focus::Assistant) => {
                    self.pane_sizes.editor = Some(layout.editor.height + step);
                }
                (Focus::Assistant, Focus::Editor) => {
                    self.pane_sizes.editor = Some(layout.editor.height - step);
                }
                _ => {}
            }
        }
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
