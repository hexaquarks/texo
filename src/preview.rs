use std::time::Duration;

use anyhow::Result;
use image::{DynamicImage, imageops::FilterType};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Margin, Rect},
    style::Style,
    widgets::{Block, Clear, Paragraph, Wrap},
};
use ratatui_image::{
    Resize, StatefulImage,
    picker::{Picker, ProtocolType},
    protocol::StatefulProtocol,
};

use crate::pdf::PdfPages;
use crate::theme;

const EMPTY_STATE_HEIGHT: u16 = 7;
const LOADING_STATE_HEIGHT: u16 = 3;
const LOADING_FRAME_MS: u128 = 150;
const LOADING_FRAMES: [&str; 6] = ["[=   ]", "[ =  ]", "[  = ]", "[   =]", "[  = ]", "[ =  ]"];
const SCROLL_LINES: u32 = 3;
const EMPTY_NOTICE: &str =
    "Your PDF will appear here.\n\nBuild your document to update the preview.\n\nCtrl+B  to build";
const UNSUPPORTED_NOTICE: &str = "Preview needs a graphics-capable terminal.\n\nLaunch texo in its dedicated window to see your PDF here.";

pub struct Preview {
    picker: Option<Picker>,
    notice: String,
    pages: Option<PdfPages>,
    page_index: usize,
    image: Option<StatefulProtocol>,
    source_page: Option<DynamicImage>,
    fitted_page: Option<DynamicImage>,
    viewport_pixels: Option<(u32, u32)>,
    scroll_pixels: u32,
    displayed_scroll: Option<u32>,
}

impl Default for Preview {
    fn default() -> Self {
        Self {
            picker: None,
            notice: UNSUPPORTED_NOTICE.into(),
            pages: None,
            page_index: 0,
            image: None,
            source_page: None,
            fitted_page: None,
            viewport_pixels: None,
            scroll_pixels: 0,
            displayed_scroll: None,
        }
    }
}

impl Preview {
    /// Query graphics support after entering the alternate screen, before reading input events.
    pub fn detect() -> Self {
        match Picker::from_query_stdio() {
            Ok(picker) if picker.protocol_type() != ProtocolType::Halfblocks => Self {
                picker: Some(picker),
                notice: EMPTY_NOTICE.into(),
                ..Self::default()
            },
            _ => Self::default(),
        }
    }

    pub fn clear(&mut self) {
        self.pages = None;
        self.set_source(None);
        self.scroll_pixels = 0;
        self.page_index = 0;
        self.notice = if self.picker.is_some() {
            EMPTY_NOTICE.into()
        } else {
            UNSUPPORTED_NOTICE.into()
        };
    }

    pub fn set_pages(&mut self, pages: PdfPages) -> Result<()> {
        let index = self.page_index.min(pages.page_count() - 1);
        let image = pages.load_page(index)?;
        self.set_source(Some(image));
        self.pages = Some(pages);
        self.page_index = index;
        Ok(())
    }

    pub fn next_page(&mut self) -> Result<()> {
        if let Some(pages) = &self.pages
            && self.page_index + 1 < pages.page_count()
        {
            self.show_page(self.page_index + 1)?;
        }
        Ok(())
    }

    pub fn previous_page(&mut self) -> Result<()> {
        if self.page_index > 0 {
            self.show_page(self.page_index - 1)?;
        }
        Ok(())
    }

    fn show_page(&mut self, index: usize) -> Result<()> {
        if let Some(pages) = &self.pages {
            let image = pages.load_page(index)?;
            self.set_source(Some(image));
            self.scroll_pixels = 0;
            self.page_index = index;
        }
        Ok(())
    }

    fn set_source(&mut self, source: Option<DynamicImage>) {
        self.source_page = source;
        self.fitted_page = None;
        self.viewport_pixels = None;
        self.displayed_scroll = None;
        self.image = None;
    }

    pub fn scroll(&mut self, up: bool) {
        let Some(picker) = &self.picker else {
            return;
        };
        let Some(page) = &self.fitted_page else {
            return;
        };
        let Some((_, viewport_height)) = self.viewport_pixels else {
            return;
        };

        let step = SCROLL_LINES * u32::from(picker.font_size().height);
        let maximum_scroll = page.height().saturating_sub(viewport_height);
        self.scroll_pixels = if up {
            self.scroll_pixels.saturating_sub(step)
        } else {
            self.scroll_pixels.saturating_add(step).min(maximum_scroll)
        };
    }

    // Cache the resized page separately from its visible crop so scrolling does
    // not repeatedly resample document text or reload the PDF from disk.
    fn prepare_viewport(&mut self, area: Rect) {
        let (Some(picker), Some(source)) = (&self.picker, &self.source_page) else {
            return;
        };
        let font = picker.font_size();
        let width = u32::from(area.width) * u32::from(font.width);
        let height = u32::from(area.height) * u32::from(font.height);
        if width == 0 || height == 0 {
            return;
        }

        if self
            .fitted_page
            .as_ref()
            .is_none_or(|page| page.width() != width)
        {
            let fitted_height = fit_width_height(source.width(), source.height(), width);
            self.fitted_page =
                Some(source.resize_exact(width, fitted_height, FilterType::Lanczos3));
            self.displayed_scroll = None;
        }
        let Some(page) = self.fitted_page.as_ref() else {
            return;
        };
        let (offset, visible_height) = viewport_crop(page.height(), height, self.scroll_pixels);
        self.scroll_pixels = offset;

        if self.viewport_pixels == Some((width, height)) && self.displayed_scroll == Some(offset) {
            return;
        }
        let visible = page.crop_imm(0, offset, width, visible_height);
        self.image = Some(picker.new_resize_protocol(visible));
        self.viewport_pixels = Some((width, height));
        self.displayed_scroll = Some(offset);
    }

    pub fn page_status(&self) -> Option<String> {
        self.pages
            .as_ref()
            .map(|pages| format!("{} / {}", self.page_index + 1, pages.page_count()))
    }

    pub fn draw_loading(frame: &mut Frame, area: Rect, elapsed: Duration) {
        frame.render_widget(Clear, area);
        frame.render_widget(
            Block::default().style(Style::default().bg(theme::PREVIEW)),
            area,
        );
        let phase = (elapsed.as_millis() / LOADING_FRAME_MS) % LOADING_FRAMES.len() as u128;
        let label = format!(
            "{}  Building PDF\n\nThe preview will appear when ready.",
            LOADING_FRAMES[phase as usize]
        );
        let [_, message, _] = Layout::vertical([
            Constraint::Fill(1),
            Constraint::Length(LOADING_STATE_HEIGHT),
            Constraint::Fill(1),
        ])
        .areas(area);
        frame.render_widget(
            Paragraph::new(label).centered().style(theme::muted()),
            message,
        );
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect) {
        frame.render_widget(
            Block::default().style(Style::default().bg(theme::PREVIEW)),
            area,
        );
        let content = area.inner(Margin::new(1, 0));
        self.prepare_viewport(content);

        match &mut self.image {
            Some(image) => {
                frame.render_stateful_widget(
                    StatefulImage::default().resize(Resize::Crop(None)),
                    content,
                    image,
                );
                if let Some(Err(error)) = image.last_encoding_result() {
                    self.notice = format!("Could not display the preview: {error}");
                    self.image = None;
                }
            }
            None => {
                let [_, message, _] = Layout::vertical([
                    Constraint::Fill(1),
                    Constraint::Length(EMPTY_STATE_HEIGHT),
                    Constraint::Fill(1),
                ])
                .areas(content);
                frame.render_widget(
                    Paragraph::new(self.notice.as_str())
                        .centered()
                        .style(theme::muted())
                        .wrap(Wrap { trim: false }),
                    message,
                );
            }
        }
    }
}

// Compute the proportional paper height without losing precision for narrow panes.
fn fit_width_height(source_width: u32, source_height: u32, target_width: u32) -> u32 {
    let height =
        u64::from(source_height) * u64::from(target_width) / u64::from(source_width.max(1));
    height.clamp(1, u64::from(u32::MAX)) as u32
}

fn viewport_crop(page_height: u32, viewport_height: u32, scroll: u32) -> (u32, u32) {
    let visible_height = page_height.min(viewport_height);
    let offset = scroll.min(page_height.saturating_sub(viewport_height));
    (offset, visible_height)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loading_state_replaces_previous_content_and_animates() -> anyhow::Result<()> {
        use ratatui::{Terminal, backend::TestBackend};
        let mut terminal = Terminal::new(TestBackend::new(60, 20))?;
        let mut captures = Vec::new();
        for elapsed in [Duration::ZERO, Duration::from_millis(150)] {
            terminal.draw(|frame| {
                frame.render_widget(Paragraph::new("Previous PDF"), frame.area());
                Preview::draw_loading(frame, frame.area(), elapsed);
            })?;
            let text: String = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect();
            assert!(text.contains("Building PDF"));
            assert!(!text.contains("Previous PDF"));
            captures.push(text);
        }
        assert_ne!(captures[0], captures[1]);
        Ok(())
    }

    #[test]
    fn fitting_width_preserves_portrait_and_landscape_proportions() {
        assert_eq!(fit_width_height(600, 900, 800), 1200);
        assert_eq!(fit_width_height(900, 600, 450), 300);
        assert_eq!(fit_width_height(1000, 1, 1), 1);
    }

    #[test]
    fn preview_scrolls_in_lines_and_clamps_after_viewport_resize() {
        let picker = Picker::halfblocks();
        let mut preview = Preview {
            picker: Some(picker),
            ..Preview::default()
        };
        preview.set_source(Some(DynamicImage::new_rgb8(120, 200)));
        preview.prepare_viewport(Rect::new(0, 0, 10, 5));
        assert_eq!(preview.fitted_page.as_ref().unwrap().width(), 100);

        preview.scroll(false);
        assert_eq!(preview.scroll_pixels, 60);
        preview.scroll(false);
        assert_eq!(preview.scroll_pixels, 66);
        preview.prepare_viewport(Rect::new(0, 0, 10, 5));
        assert_eq!(preview.displayed_scroll, Some(66));

        preview.prepare_viewport(Rect::new(0, 0, 10, 20));
        assert_eq!(preview.scroll_pixels, 0);
        preview.scroll(true);
        assert_eq!(preview.scroll_pixels, 0);
        preview.clear();
        assert!(preview.source_page.is_none());
        assert!(preview.image.is_none());
    }

    #[test]
    fn viewport_scroll_reaches_bottom_without_reading_past_paper() {
        assert_eq!(viewport_crop(1200, 800, 150), (150, 800));
        assert_eq!(viewport_crop(1200, 800, 1000), (400, 800));
        assert_eq!(viewport_crop(1200, 1500, 400), (0, 1200));
    }
}
