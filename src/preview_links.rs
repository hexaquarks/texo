use ratatui::layout::{Position, Rect};

use crate::pdf::PdfLink;

/// Maps the visible image (in terminal cells) to normalized coordinates on its PDF page.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct DisplayedPage {
    pub pane: Rect,
    // Rectangles use [left, top, right, bottom].
    pub image: [f64; 4],
    pub source: [f64; 4],
}

impl DisplayedPage {
    pub fn moved_to(mut self, pane: Rect) -> Self {
        let shift_x = f64::from(pane.x) - f64::from(self.pane.x);
        let shift_y = f64::from(pane.y) - f64::from(self.pane.y);
        self.image[0] += shift_x;
        self.image[2] += shift_x;
        self.image[1] += shift_y;
        self.image[3] += shift_y;
        self.pane = pane;
        self
    }

    pub fn link_at<'a>(&self, links: &'a [PdfLink], position: Position) -> Option<&'a str> {
        if !self.pane.contains(position) {
            return None;
        }

        // Mouse reports identify a cell, not a pixel. Intersect its visible image portion
        // so small icon links remain clickable without activating links in the gutters.
        let left = f64::from(position.x).max(self.image[0]);
        let top = f64::from(position.y).max(self.image[1]);
        let right = (f64::from(position.x) + 1.0).min(self.image[2]);
        let bottom = (f64::from(position.y) + 1.0).min(self.image[3]);
        if left >= right || top >= bottom {
            return None;
        }

        let scale_x = (self.source[2] - self.source[0]) / (self.image[2] - self.image[0]);
        let scale_y = (self.source[3] - self.source[1]) / (self.image[3] - self.image[1]);
        let cell = [
            self.source[0] + (left - self.image[0]) * scale_x,
            self.source[1] + (top - self.image[1]) * scale_y,
            self.source[0] + (right - self.image[0]) * scale_x,
            self.source[1] + (bottom - self.image[1]) * scale_y,
        ];

        let mut target = None;
        for link in links {
            let width = cell[2].min(link.bounds[2]) - cell[0].max(link.bounds[0]);
            let height = cell[3].min(link.bounds[3]) - cell[1].max(link.bounds[1]);
            if width <= 0.0 || height <= 0.0 {
                continue;
            }

            // Two different targets in one cell are ambiguous; zooming makes them separable.
            if target.is_some_and(|uri| uri != link.uri) {
                return None;
            }
            target = Some(link.uri.as_str());
        }

        target
    }
}
