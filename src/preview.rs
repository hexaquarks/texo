use std::{
    sync::{
        Arc,
        mpsc::{self, Receiver, TryRecvError},
    },
    thread,
    time::Duration,
};

use anyhow::{Result, anyhow};
use image::{DynamicImage, imageops::FilterType};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Margin, Position, Rect},
    style::Style,
    widgets::{Block, Clear, Paragraph, Wrap},
};
use ratatui_image::{
    Resize, ResizeEncodeRender, StatefulImage,
    picker::{Picker, ProtocolType, cap_parser::QueryStdioOptions},
    protocol::StatefulProtocol,
};

use crate::pdf::PdfPages;
use crate::theme;

const EMPTY_STATE_HEIGHT: u16 = 7;
const LOADING_STATE_HEIGHT: u16 = 3;
const LOADING_FRAME_MS: u128 = 150;
const LOADING_FRAMES: [&str; 6] = ["[=   ]", "[ =  ]", "[  = ]", "[   =]", "[  = ]", "[ =  ]"];
const SCROLL_LINES: u32 = 3;
const FIT_WIDTH_PERCENT: u16 = 100;
const MIN_ZOOM_PERCENT: u16 = 25;
const MAX_ZOOM_PERCENT: u16 = 300;
const ZOOM_STEP_PERCENT: u16 = 25;
const EMPTY_NOTICE: &str =
    "Your PDF will appear here.\n\nBuild your document to update the preview.\n\nCtrl+B  to build";

pub struct Preview {
    picker: Picker,
    notice: String,
    pages: Option<PdfPages>,
    page_index: usize,
    image: Option<StatefulProtocol>,
    source_page: Option<Arc<DynamicImage>>,
    scaled_page_size: Option<(u32, u32)>,
    pending: Option<ViewportJob>,
    viewport_pixels: Option<(u32, u32)>,
    scroll_pixels: u32,
    displayed_scroll: Option<u32>,
    zoom_percent: u16,
    horizontal_pixels: u32,
    displayed_horizontal: Option<u32>,
}

struct PreparedViewport {
    image: StatefulProtocol,
    page_size: (u32, u32),
    scroll: u32,
    horizontal: u32,
}

struct ViewportJob {
    source: Arc<DynamicImage>,
    pixels: (u32, u32),
    scroll: u32,
    result: Receiver<Result<PreparedViewport>>,
    zoom_percent: u16,
    horizontal: u32,
}

impl Default for Preview {
    fn default() -> Self {
        Self {
            picker: Picker::halfblocks(),
            notice: EMPTY_NOTICE.into(),
            pages: None,
            page_index: 0,
            image: None,
            source_page: None,
            scaled_page_size: None,
            pending: None,
            viewport_pixels: None,
            scroll_pixels: 0,
            displayed_scroll: None,
            zoom_percent: FIT_WIDTH_PERCENT,
            horizontal_pixels: 0,
            displayed_horizontal: None,
        }
    }
}

impl Preview {
    /// Query graphics support after entering the alternate screen, before reading input events.
    pub fn detect() -> Self {
        // Resume pages compress well. Raw RGBA uploads can be several megabytes,
        // overwhelming a multiplexer's passthrough before the first page is visible.
        let options = QueryStdioOptions {
            kitty_compression: true,
            ..QueryStdioOptions::default()
        };

        let picker =
            Picker::from_query_stdio_with_options(options).unwrap_or_else(|_| Picker::halfblocks());
        Self::with_picker(picker)
    }

    pub fn with_picker(picker: Picker) -> Self {
        Self {
            picker,
            ..Self::default()
        }
    }

    pub fn empty_like(&self) -> Self {
        Self {
            picker: self.picker.clone(),
            ..Self::default()
        }
    }

    pub fn uses_basic_rendering(&self) -> bool {
        self.picker.protocol_type() == ProtocolType::Halfblocks
    }

    pub fn clear(&mut self) {
        self.pages = None;
        self.set_source(None);
        self.scroll_pixels = 0;
        self.page_index = 0;
        self.zoom_percent = FIT_WIDTH_PERCENT;
        self.horizontal_pixels = 0;
        self.notice = EMPTY_NOTICE.into();
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
            self.horizontal_pixels = 0;
            self.page_index = index;
        }
        Ok(())
    }

    fn set_source(&mut self, source: Option<DynamicImage>) {
        self.source_page = source.map(Arc::new);
        self.scaled_page_size = None;
        self.viewport_pixels = None;
        self.displayed_scroll = None;
        self.displayed_horizontal = None;
        self.image = None;
    }

    pub fn scroll(&mut self, up: bool) {
        let Some((_, page_height)) = self.scaled_page_size else {
            return;
        };
        let Some((_, viewport_height)) = self.viewport_pixels else {
            return;
        };

        let step = SCROLL_LINES * u32::from(self.picker.font_size().height);
        let maximum_scroll = page_height.saturating_sub(viewport_height);
        self.scroll_pixels = if up {
            self.scroll_pixels.saturating_sub(step)
        } else {
            self.scroll_pixels.saturating_add(step).min(maximum_scroll)
        };
    }

    pub fn zoom_percent(&self) -> u16 {
        self.zoom_percent
    }

    pub fn is_preparing(&self) -> bool {
        self.source_page.is_some() && self.pending.is_some()
    }

    /// The page point under the cursor stays in place unless an edge clamps the viewport.
    pub fn zoom_at(&mut self, zoom_in: bool, area: Rect, position: Position) {
        let percent = if zoom_in {
            self.zoom_percent
                .saturating_add(ZOOM_STEP_PERCENT)
                .min(MAX_ZOOM_PERCENT)
        } else {
            self.zoom_percent
                .saturating_sub(ZOOM_STEP_PERCENT)
                .max(MIN_ZOOM_PERCENT)
        };
        self.set_zoom(percent, area, position);
    }

    pub fn reset_zoom(&mut self, area: Rect) {
        let center = Position::new(area.x + area.width / 2, area.y + area.height / 2);
        self.set_zoom(FIT_WIDTH_PERCENT, area, center);
    }

    fn set_zoom(&mut self, percent: u16, area: Rect, position: Position) {
        let Some(source) = &self.source_page else {
            return;
        };
        let content = preview_content(area);
        if content.is_empty() || percent == self.zoom_percent {
            return;
        }

        let font = self.picker.font_size();
        let width = u32::from(content.width) * u32::from(font.width);
        let height = u32::from(content.height) * u32::from(font.height);
        let old_width = zoomed_width(width, self.zoom_percent);
        let new_width = zoomed_width(width, percent);
        let new_height = fit_width_height(source.width(), source.height(), new_width);
        let cursor_x = u32::from(position.x.saturating_sub(content.x)) * u32::from(font.width);
        let cursor_y = u32::from(position.y.saturating_sub(content.y)) * u32::from(font.height);

        // Below fit-width the page is centered; account for that gutter when anchoring.
        let old_gutter = width.saturating_sub(old_width) / 2;
        let new_gutter = width.saturating_sub(new_width) / 2;
        self.horizontal_pixels = anchored_offset(
            self.horizontal_pixels,
            cursor_x.saturating_sub(old_gutter),
            cursor_x.saturating_sub(new_gutter),
            old_width,
            new_width,
        )
        .min(new_width.saturating_sub(width));
        self.scroll_pixels =
            anchored_offset(self.scroll_pixels, cursor_y, cursor_y, old_width, new_width)
                .min(new_height.saturating_sub(height));

        self.zoom_percent = percent;
        self.viewport_pixels = None;
        self.scaled_page_size = None;
    }

    pub fn pan_horizontal(&mut self, left: bool) {
        let Some((page_width, _)) = self.scaled_page_size else {
            return;
        };
        let Some((viewport_width, _)) = self.viewport_pixels else {
            return;
        };

        let step = SCROLL_LINES * u32::from(self.picker.font_size().width);
        let maximum = page_width.saturating_sub(viewport_width);
        self.horizontal_pixels = if left {
            self.horizontal_pixels.saturating_sub(step)
        } else {
            self.horizontal_pixels.saturating_add(step).min(maximum)
        };
    }

    // Only one worker per preview may run at a time. Resizes and scrolls coalesce
    // while it runs; results for an old page or viewport are never displayed.
    fn prepare_viewport(&mut self, area: Rect) {
        let Some(source) = self.source_page.clone() else {
            return;
        };
        let font = self.picker.font_size();
        let pixels = (
            u32::from(area.width) * u32::from(font.width),
            u32::from(area.height) * u32::from(font.height),
        );
        if pixels.0 == 0 || pixels.1 == 0 {
            return;
        }

        if let Some(job) = &self.pending {
            let result = match job.result.try_recv() {
                Ok(result) => result,
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => Err(anyhow!("Preview worker stopped")),
            };
            let current = Arc::ptr_eq(&job.source, &source)
                && job.pixels == pixels
                && job.scroll == self.scroll_pixels
                && job.horizontal == self.horizontal_pixels
                && job.zoom_percent == self.zoom_percent;
            self.pending = None;

            if current {
                match result {
                    Ok(prepared) => {
                        self.image = Some(prepared.image);
                        self.scaled_page_size = Some(prepared.page_size);
                        self.scroll_pixels = prepared.scroll;
                        self.displayed_scroll = Some(prepared.scroll);
                        self.horizontal_pixels = prepared.horizontal;
                        self.displayed_horizontal = Some(prepared.horizontal);
                        self.viewport_pixels = Some(pixels);
                    }
                    Err(error) => {
                        self.notice = format!("Could not display the preview: {error}");
                        self.image = None;
                        if !self.uses_basic_rendering() {
                            self.picker = Picker::halfblocks();
                            self.scaled_page_size = None;
                        } else {
                            // Keep the error visible instead of spawning another failed worker
                            // every frame. A new page or viewport can try again.
                            self.viewport_pixels = Some(pixels);
                            self.displayed_scroll = Some(self.scroll_pixels);
                            self.displayed_horizontal = Some(self.horizontal_pixels);
                            return;
                        }
                    }
                }
            }
        }

        if self.viewport_pixels == Some(pixels)
            && self.displayed_scroll == Some(self.scroll_pixels)
            && self.displayed_horizontal == Some(self.horizontal_pixels)
        {
            return;
        }

        let picker = self.picker.clone();
        let worker_source = Arc::clone(&source);
        let scroll = self.scroll_pixels;
        let horizontal = self.horizontal_pixels;
        let zoom_percent = self.zoom_percent;
        let (sender, result) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("texo-preview".into())
            .spawn(move || {
                let prepared = prepare_image(
                    picker,
                    worker_source,
                    area,
                    scroll,
                    horizontal,
                    zoom_percent,
                );
                let _ = sender.send(prepared);
            });

        match worker {
            Ok(_) => {
                self.pending = Some(ViewportJob {
                    source,
                    pixels,
                    scroll,
                    result,
                    zoom_percent,
                    horizontal,
                });
            }
            Err(error) => {
                self.notice = format!("Could not prepare the preview: {error}");
                self.viewport_pixels = Some(pixels);
                self.displayed_scroll = Some(self.scroll_pixels);
                self.displayed_horizontal = Some(self.horizontal_pixels);
            }
        }
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
        let content = preview_content(area);
        self.prepare_viewport(content);

        match &mut self.image {
            Some(image)
                if image
                    .needs_resize(&Resize::Crop(None), content.as_size())
                    .is_none() =>
            {
                frame.render_stateful_widget(
                    StatefulImage::default().resize(Resize::Crop(None)),
                    content,
                    image,
                );
            }
            _ => {
                let [_, message, _] = Layout::vertical([
                    Constraint::Fill(1),
                    Constraint::Length(EMPTY_STATE_HEIGHT),
                    Constraint::Fill(1),
                ])
                .areas(content);
                frame.render_widget(
                    Paragraph::new(if self.source_page.is_some() && self.pending.is_some() {
                        "Preparing PDF preview…"
                    } else {
                        self.notice.as_str()
                    })
                    .centered()
                    .style(theme::muted())
                    .wrap(Wrap { trim: false }),
                    message,
                );
            }
        }
    }
}

// Resampling and protocol encoding can be expensive, especially for large PNGs
// under tmux. Finish both before giving the image to the terminal's draw loop.
fn prepare_image(
    picker: Picker,
    source: Arc<DynamicImage>,
    area: Rect,
    scroll: u32,
    horizontal: u32,
    zoom_percent: u16,
) -> Result<PreparedViewport> {
    let font = picker.font_size();
    let width = u32::from(area.width) * u32::from(font.width);
    let height = u32::from(area.height) * u32::from(font.height);
    let page_width = zoomed_width(width, zoom_percent);
    let page_height = fit_width_height(source.width(), source.height(), page_width);
    let (scroll, visible_height) = viewport_crop(page_height, height, scroll);
    let horizontal = horizontal.min(page_width.saturating_sub(width));
    let visible_width = width.min(page_width);

    // Crop the original page first. Enlarging the entire page makes zoom cost grow
    // with off-screen pixels, leaving the previous image visible for seconds.
    let source_region = source_viewport(
        (source.width(), source.height()),
        (page_width, page_height),
        image::math::Rect {
            x: horizontal,
            y: scroll,
            width: visible_width,
            height: visible_height,
        },
    );
    let crop = source.crop_imm(
        source_region.x,
        source_region.y,
        source_region.width,
        source_region.height,
    );
    let crop = crop.resize_exact(visible_width, visible_height, FilterType::Lanczos3);
    let visible = if crop.width() == width {
        crop
    } else {
        let mut canvas = DynamicImage::new_rgb8(width, visible_height);
        let gutter = (width - crop.width()) / 2;
        image::imageops::overlay(&mut canvas, &crop, i64::from(gutter), 0);
        canvas
    };
    let mut image = picker.new_resize_protocol(visible);
    let resize = Resize::Crop(None);

    // A short page occupies fewer rows than its pane. Match the widget's target
    // size so drawing recognizes the prepared image instead of requesting another resize.
    if let Some(size) = image.needs_resize(&resize, area.as_size()) {
        image.resize_encode(&resize, size);
    }

    if let Some(result) = image.last_encoding_result() {
        result?;
    }

    Ok(PreparedViewport {
        image,
        page_size: (page_width, page_height),
        scroll,
        horizontal,
    })
}

// Round outward so fractional source pixels at the viewport edge are retained.
fn source_viewport(
    source: (u32, u32),
    scaled: (u32, u32),
    viewport: image::math::Rect,
) -> image::math::Rect {
    let left = u64::from(viewport.x) * u64::from(source.0) / u64::from(scaled.0);
    let top = u64::from(viewport.y) * u64::from(source.1) / u64::from(scaled.1);
    let right = (u64::from(viewport.x + viewport.width) * u64::from(source.0))
        .div_ceil(u64::from(scaled.0));
    let bottom = (u64::from(viewport.y + viewport.height) * u64::from(source.1))
        .div_ceil(u64::from(scaled.1));

    image::math::Rect {
        x: left as u32,
        y: top as u32,
        width: (right - left) as u32,
        height: (bottom - top) as u32,
    }
}

fn preview_content(area: Rect) -> Rect {
    area.inner(Margin::new(theme::PANEL_PADDING, 1))
}

fn zoomed_width(viewport_width: u32, percent: u16) -> u32 {
    (u64::from(viewport_width) * u64::from(percent) / u64::from(FIT_WIDTH_PERCENT))
        .clamp(1, u64::from(u32::MAX)) as u32
}

fn anchored_offset(
    offset: u32,
    old_anchor: u32,
    new_anchor: u32,
    old_width: u32,
    new_width: u32,
) -> u32 {
    let scaled = (u64::from(offset) + u64::from(old_anchor)) * u64::from(new_width)
        / u64::from(old_width.max(1));
    scaled
        .saturating_sub(u64::from(new_anchor))
        .min(u64::from(u32::MAX)) as u32
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
#[path = "preview_tests.rs"]
mod tests;
