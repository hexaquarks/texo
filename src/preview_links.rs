use image::{DynamicImage, Rgba, RgbaImage};
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

// A light blue wash at roughly 12.5% opacity keeps text and icon strokes legible.
const HOVER_TINT: Rgba<u8> = Rgba([100, 149, 237, 32]);

pub(super) fn highlight_link(image: &mut DynamicImage, bounds: [f64; 4]) {
    let left = (bounds[0] * f64::from(image.width())).floor() as u32;
    let top = (bounds[1] * f64::from(image.height())).floor() as u32;
    let right = (bounds[2] * f64::from(image.width())).ceil() as u32;
    let bottom = (bounds[3] * f64::from(image.height())).ceil() as u32;

    let tint = RgbaImage::from_pixel(right - left, bottom - top, HOVER_TINT);
    image::imageops::overlay(image, &tint, i64::from(left), i64::from(top));
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
        self.link_index_at(links, position)
            .map(|index| links[index].uri.as_str())
    }

    pub fn link_index_at(&self, links: &[PdfLink], position: Position) -> Option<usize> {
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

        let mut target: Option<usize> = None;
        for (index, link) in links.iter().enumerate() {
            let width = cell[2].min(link.bounds[2]) - cell[0].max(link.bounds[0]);
            let height = cell[3].min(link.bounds[3]) - cell[1].max(link.bounds[1]);
            if width <= 0.0 || height <= 0.0 {
                continue;
            }

            // Two different targets in one cell are ambiguous; zooming makes them separable.
            if target.is_some_and(|previous| links[previous].uri != link.uri) {
                return None;
            }
            target.get_or_insert(index);
        }

        target
    }
}
