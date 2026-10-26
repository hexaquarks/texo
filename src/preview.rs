use std::{
    sync::{
        Arc,
        mpsc::{self, Receiver, TryRecvError},
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{Result, anyhow};
use image::{DynamicImage, imageops::FilterType};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Margin, Position, Rect, Size},
    style::Style,
    text::{Line, Text},
    widgets::{Block, Clear, Paragraph, Wrap},
};
use ratatui_image::{
    Resize, ResizeEncodeRender, StatefulImage,
    picker::{Picker, ProtocolType, cap_parser::QueryStdioOptions},
    protocol::StatefulProtocol,
};

use ratatui_spinner::CircleSpinner;

use crate::theme;
use crate::{
    pdf::PdfPages,
    retained_image::{ImageView, RetainedImage, TerminalImages},
};

const EMPTY_STATE_HEIGHT: u16 = 7;
const LOADING_FRAME_MS: u128 = 100;
const LOADING_LABEL_HEIGHT: u16 = 2;
const SCROLL_LINES: u32 = 3;
const FIT_WIDTH_PERCENT: u16 = 100;
const MAX_ZOOM_PERCENT: u16 = 300;
const ZOOM_STEP_PERCENT: u16 = 20;
const WHEEL_ZOOM_INTERVAL: Duration = Duration::from_millis(150);
const EMPTY_NOTICE: &str =
    "Your PDF will appear here.\n\nBuild your document to update the preview.";

pub struct Preview {
    picker: Picker,
    notice: String,
    loading_started: Option<Instant>,
    pages: Option<PdfPages>,
    page_index: usize,
    image: Option<PreparedImage>,
    image_source: Option<Arc<DynamicImage>>,
    terminal_images: TerminalImages,
    image_presented: bool,
    source_page: Option<Arc<DynamicImage>>,
    scaled_page_size: Option<(u32, u32)>,
    pending: Option<ViewportJob>,
    viewport_pixels: Option<(u32, u32)>,
    scroll_pixels: u32,
    displayed_scroll: Option<u32>,
    zoom_percent: u16,
    last_wheel_zoom: Option<Instant>,
    drag_position: Option<Position>,
    horizontal_pixels: u32,
    displayed_horizontal: Option<u32>,
}

enum PreparedImage {
    Raster(PreparedViewport),
    Retained(RetainedImage),
}

struct PreparedViewport {
    image: StatefulProtocol,
    page_size: (u32, u32),
    scroll: u32,
    horizontal: u32,
}

struct RetainedViewport {
    view: ImageView,
    page_size: (u32, u32),
    scroll: u32,
    horizontal: u32,
}

struct ViewportJob {
    source: Arc<DynamicImage>,
    pixels: (u32, u32),
    scroll: u32,
    result: Receiver<Result<PreparedImage>>,
    zoom_percent: u16,
    horizontal: u32,
}

impl Default for Preview {
    fn default() -> Self {
        Self {
            picker: Picker::halfblocks(),
            notice: EMPTY_NOTICE.into(),
            loading_started: None,
            pages: None,
            page_index: 0,
            image: None,
            image_source: None,
            terminal_images: TerminalImages::default(),
            image_presented: false,
            source_page: None,
            scaled_page_size: None,
            pending: None,
            viewport_pixels: None,
            scroll_pixels: 0,
            displayed_scroll: None,
            zoom_percent: FIT_WIDTH_PERCENT,
            last_wheel_zoom: None,
            drag_position: None,
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
            terminal_images: self.terminal_images.clone(),
            ..Self::default()
        }
    }

    pub fn flush_retired_images(&self, writer: &mut impl std::io::Write) -> std::io::Result<()> {
        self.terminal_images.flush_retired(writer)
    }

    pub fn uses_basic_rendering(&self) -> bool {
        self.picker.protocol_type() == ProtocolType::Halfblocks
    }

    pub fn uses_inline_graphics(&self) -> bool {
        matches!(
            self.picker.protocol_type(),
            ProtocolType::Sixel | ProtocolType::Iterm2
        )
    }

    pub fn has_presented_image(&self) -> bool {
        self.image_presented
    }

    /// Keep one animation clock across compilation and terminal image preparation.
    pub fn begin_loading(&mut self) {
        self.loading_started = Some(Instant::now());
    }

    pub fn draw_loading(&mut self, frame: &mut Frame, area: Rect) {
        let started = self.loading_started.get_or_insert_with(Instant::now);
        Self::draw_loading_at(frame, area, started.elapsed());
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
        if self.source_page.is_some() {
            self.loading_started.get_or_insert_with(Instant::now);
        } else {
            self.loading_started = None;
        }

        self.scaled_page_size = None;
        self.viewport_pixels = None;
        self.displayed_scroll = None;
        self.displayed_horizontal = None;
        self.last_wheel_zoom = None;
        self.end_drag();

        // Retain the previous page for modal redraws until its replacement is ready.
        if self.source_page.is_none() {
            self.image = None;
            self.image_source = None;
            self.image_presented = false;
        }
    }

    pub fn begin_drag(&mut self, area: Rect, position: Position) {
        self.drag_position = None;
        if preview_content(area).contains(position)
            && self.scaled_page_size.is_some()
            && self.viewport_pixels.is_some()
        {
            self.drag_position = Some(position);
        }
    }

    pub fn end_drag(&mut self) {
        self.drag_position = None;
    }

    /// Move the paper with the pointer, including when it crosses the pane boundary.
    pub fn drag_to(&mut self, position: Position) -> bool {
        let Some(previous) = self.drag_position else {
            return false;
        };
        let Some((page_width, page_height)) = self.scaled_page_size else {
            self.end_drag();
            return false;
        };
        let Some((viewport_width, viewport_height)) = self.viewport_pixels else {
            self.end_drag();
            return false;
        };

        let font = self.picker.font_size();
        let horizontal =
            (i32::from(previous.x) - i32::from(position.x)).saturating_mul(i32::from(font.width));
        let vertical =
            (i32::from(previous.y) - i32::from(position.y)).saturating_mul(i32::from(font.height));
        self.horizontal_pixels = self
            .horizontal_pixels
            .saturating_add_signed(horizontal)
            .min(page_width.saturating_sub(viewport_width));
        self.scroll_pixels = self
            .scroll_pixels
            .saturating_add_signed(vertical)
            .min(page_height.saturating_sub(viewport_height));

        // Advance even at an edge, so reversing direction responds immediately.
        self.drag_position = Some(position);
        true
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
        self.source_page.is_some() && (self.pending.is_some() || self.viewport_pixels.is_none())
    }

    /// The page point under the cursor stays in place unless an edge clamps the viewport.
    pub fn zoom_at(&mut self, zoom_in: bool, area: Rect, position: Position) {
        let percent = if zoom_in {
            self.zoom_percent
                .saturating_add(ZOOM_STEP_PERCENT)
                .min(MAX_ZOOM_PERCENT)
        } else {
            self.zoom_percent.saturating_sub(ZOOM_STEP_PERCENT)
        };
        self.set_zoom(percent, area, position);
    }

    pub fn zoom_from_wheel(&mut self, zoom_in: bool, area: Rect, position: Position) {
        self.zoom_from_wheel_at(zoom_in, area, position, Instant::now());
    }

    fn zoom_from_wheel_at(&mut self, zoom_in: bool, area: Rect, position: Position, now: Instant) {
        // Trackpads send many wheel events per swipe. Ignore the excess events
        // instead of queuing zoom steps that continue after the gesture stops.
        if self
            .last_wheel_zoom
            .is_some_and(|previous| now.saturating_duration_since(previous) < WHEEL_ZOOM_INTERVAL)
        {
            return;
        }

        let previous = self.zoom_percent;
        self.zoom_at(zoom_in, area, position);

        if self.zoom_percent != previous {
            self.last_wheel_zoom = Some(now);
        }
    }

    pub fn reset_zoom(&mut self, area: Rect) {
        let Some(source) = &self.source_page else {
            return;
        };
        let content = preview_content(area);
        let font = self.picker.font_size();
        let pixels = (
            u32::from(content.width) * u32::from(font.width),
            u32::from(content.height) * u32::from(font.height),
        );
        let minimum = fit_page_zoom_percent((source.width(), source.height()), pixels);

        let center = Position::new(area.x + area.width / 2, area.y + area.height / 2);
        self.set_zoom(minimum, area, center);
    }

    fn set_zoom(&mut self, percent: u16, area: Rect, position: Position) {
        self.end_drag();
        let Some(source) = &self.source_page else {
            return;
        };
        let content = preview_content(area);
        if content.is_empty() {
            return;
        }

        let font = self.picker.font_size();
        let width = u32::from(content.width) * u32::from(font.width);
        let height = u32::from(content.height) * u32::from(font.height);
        let minimum = fit_page_zoom_percent((source.width(), source.height()), (width, height));
        let percent = percent.clamp(minimum, MAX_ZOOM_PERCENT);
        if percent == self.zoom_percent {
            return;
        }

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

    // Retained pages need only new placement geometry after upload. Other protocols
    // coalesce view changes into one worker and never display obsolete raster results.
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

        let minimum = fit_page_zoom_percent((source.width(), source.height()), pixels);
        if self.zoom_percent < minimum {
            self.zoom_percent = minimum;
            self.viewport_pixels = None;
            self.scaled_page_size = None;
        }

        if !self.receive_prepared_viewport(&source, pixels) {
            return;
        }

        // Beyond the protocol's coordinate table, keep the existing raster backend.
        let retain_page = self.picker.protocol_type() == ProtocolType::Kitty
            && self
                .retained_viewport(&source, area)
                .view
                .supports_placeholders();
        let retained_ready = retain_page
            && matches!(self.image, Some(PreparedImage::Retained(_)))
            && self
                .image_source
                .as_ref()
                .is_some_and(|image_source| Arc::ptr_eq(image_source, &source));
        if retained_ready {
            let viewport = self.retained_viewport(&source, area);
            self.scaled_page_size = Some(viewport.page_size);
            self.viewport_pixels = Some(pixels);
            self.scroll_pixels = viewport.scroll;
            self.horizontal_pixels = viewport.horizontal;
            self.displayed_scroll = Some(viewport.scroll);
            self.displayed_horizontal = Some(viewport.horizontal);
            return;
        }

        if !retain_page
            && self.viewport_pixels == Some(pixels)
            && self.displayed_scroll == Some(self.scroll_pixels)
            && self.displayed_horizontal == Some(self.horizontal_pixels)
        {
            return;
        }

        self.start_viewport(source, area, pixels, retain_page);
    }

    // A result must belong to this source; raster results must also match the view.
    // Retained uploads can adopt a newer zoom without repeating preparation.
    fn receive_prepared_viewport(
        &mut self,
        source: &Arc<DynamicImage>,
        pixels: (u32, u32),
    ) -> bool {
        if let Some(job) = &self.pending {
            let result = match job.result.try_recv() {
                Ok(result) => result,
                Err(TryRecvError::Empty) => return false,
                Err(TryRecvError::Disconnected) => Err(anyhow!("Preview worker stopped")),
            };

            // A retained page upload is independent of zoom or pan. It is still useful
            // when the user changes the view before the first upload has finished.
            let retained = matches!(&result, Ok(PreparedImage::Retained(_)));
            let current = Arc::ptr_eq(&job.source, source)
                && (retained
                    || (job.pixels == pixels
                        && job.scroll == self.scroll_pixels
                        && job.horizontal == self.horizontal_pixels
                        && job.zoom_percent == self.zoom_percent));
            self.pending = None;

            if current {
                match result {
                    Ok(prepared) => {
                        if let PreparedImage::Raster(viewport) = &prepared {
                            self.scaled_page_size = Some(viewport.page_size);
                            self.scroll_pixels = viewport.scroll;
                            self.displayed_scroll = Some(viewport.scroll);
                            self.horizontal_pixels = viewport.horizontal;
                            self.displayed_horizontal = Some(viewport.horizontal);
                            self.viewport_pixels = Some(pixels);
                        }

                        self.image = Some(prepared);
                        self.image_source = Some(source.clone());
                        self.image_presented = false;
                    }
                    Err(error) => {
                        self.notice = format!("Could not display the preview: {error}");
                        self.image = None;
                        self.image_presented = false;
                        if !self.uses_basic_rendering() {
                            self.picker = Picker::halfblocks();
                            self.scaled_page_size = None;
                        } else {
                            // Keep the error visible instead of spawning another failed worker
                            // every frame. A new page or viewport can try again.
                            self.viewport_pixels = Some(pixels);
                            self.displayed_scroll = Some(self.scroll_pixels);
                            self.displayed_horizontal = Some(self.horizontal_pixels);
                            return false;
                        }
                    }
                }
            }
        }

        true
    }

    fn start_viewport(
        &mut self,
        source: Arc<DynamicImage>,
        area: Rect,
        pixels: (u32, u32),
        retain_page: bool,
    ) {
        let picker = self.picker.clone();
        let worker_source = Arc::clone(&source);
        let scroll = self.scroll_pixels;
        let horizontal = self.horizontal_pixels;
        let zoom_percent = self.zoom_percent;
        let terminal_images = self.terminal_images.clone();
        let (sender, result) = mpsc::channel();

        let worker = thread::Builder::new()
            .name("texo-preview".into())
            .spawn(move || {
                let prepared = if retain_page {
                    RetainedImage::new(&picker, worker_source.as_ref().clone(), terminal_images)
                        .map(PreparedImage::Retained)
                } else {
                    prepare_image(
                        picker,
                        worker_source,
                        area,
                        scroll,
                        horizontal,
                        zoom_percent,
                    )
                    .map(PreparedImage::Raster)
                };

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

    fn retained_viewport(&self, source: &DynamicImage, area: Rect) -> RetainedViewport {
        let font = self.picker.font_size();
        let pixels = (
            u32::from(area.width) * u32::from(font.width),
            u32::from(area.height) * u32::from(font.height),
        );
        let page_width = zoomed_width(pixels.0, self.zoom_percent);
        let page_height = fit_width_height(source.width(), source.height(), page_width);
        let (scroll, visible_height) = viewport_crop(page_height, pixels.1, self.scroll_pixels);
        let horizontal = self
            .horizontal_pixels
            .min(page_width.saturating_sub(pixels.0));
        let visible_width = page_width.min(pixels.0);
        let gutter = pixels.0.saturating_sub(visible_width) / 2;

        let view = ImageView {
            page_size: Size::new(
                page_width
                    .div_ceil(u32::from(font.width))
                    .min(u32::from(u16::MAX)) as u16,
                page_height
                    .div_ceil(u32::from(font.height))
                    .min(u32::from(u16::MAX)) as u16,
            ),
            offset: Position::new(
                (horizontal / u32::from(font.width)).min(u32::from(u16::MAX)) as u16,
                (scroll / u32::from(font.height)).min(u32::from(u16::MAX)) as u16,
            ),
            area: Rect::new(
                area.x + (gutter / u32::from(font.width)) as u16,
                area.y,
                visible_width.div_ceil(u32::from(font.width)) as u16,
                visible_height.div_ceil(u32::from(font.height)) as u16,
            ),
        };

        RetainedViewport {
            view,
            page_size: (page_width, page_height),
            scroll,
            horizontal,
        }
    }

    pub fn page_status(&self) -> Option<String> {
        self.pages
            .as_ref()
            .map(|pages| format!("{} / {}", self.page_index + 1, pages.page_count()))
    }

    fn draw_loading_at(frame: &mut Frame, area: Rect, elapsed: Duration) {
        frame.render_widget(Clear, area);
        frame.render_widget(
            Block::default().style(Style::default().bg(theme::PREVIEW)),
            area,
        );

        let tick = (elapsed.as_millis() / LOADING_FRAME_MS) as u64;
        let spinner = CircleSpinner::new(tick)
            .arc_color(theme::LAVENDER)
            .dim_color(theme::DIVIDER);
        let (spinner_width, spinner_height) = spinner.char_size();
        let [_, message, _] = Layout::vertical([
            Constraint::Fill(1),
            Constraint::Length(spinner_height + LOADING_LABEL_HEIGHT),
            Constraint::Fill(1),
        ])
        .areas(area);
        let [spinner_row, label] = Layout::vertical([
            Constraint::Length(spinner_height),
            Constraint::Length(LOADING_LABEL_HEIGHT),
        ])
        .areas(message);
        let [_, spinner_area, _] = Layout::horizontal([
            Constraint::Fill(1),
            Constraint::Length(spinner_width),
            Constraint::Fill(1),
        ])
        .areas(spinner_row);

        frame.render_widget(spinner, spinner_area);
        frame.render_widget(
            Paragraph::new("\nLoading PDF…")
                .centered()
                .style(theme::muted()),
            label,
        );
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect) {
        self.draw_preview(frame, area, false);
    }

    /// Keep the last visible image beneath a modal without consuming a new Kitty upload.
    pub fn draw_cached(&mut self, frame: &mut Frame, area: Rect) {
        self.draw_preview(frame, area, true);
    }

    fn draw_preview(&mut self, frame: &mut Frame, area: Rect, cached_only: bool) {
        frame.render_widget(
            Block::default().style(Style::default().bg(theme::PREVIEW)),
            area,
        );
        let content = preview_content(area);
        if !cached_only {
            self.prepare_viewport(content);

            let waiting_for_page =
                self.is_preparing() && (self.loading_started.is_some() || self.image.is_none());
            if waiting_for_page {
                self.draw_loading(frame, area);
                return;
            }

            self.loading_started = None;
        }

        // A modal or a replacement page can keep an older image visible. Recompute
        // its geometry from that image's source so resizing never writes outside the pane.
        let retained_view = self
            .image_source
            .as_ref()
            .filter(|_| matches!(self.image, Some(PreparedImage::Retained(_))))
            .map(|source| self.retained_viewport(source, content).view);

        match &mut self.image {
            Some(PreparedImage::Retained(image)) if !cached_only || self.image_presented => {
                let view = retained_view
                    .filter(|view| view.supports_placeholders())
                    .or_else(|| image.last_view());
                if let Some(mut view) = view {
                    view.area = view.area.intersection(content);
                    match image.render(view, frame.buffer_mut()) {
                        Ok(()) => self.image_presented = true,
                        Err(error) => {
                            self.notice = format!("Could not place the PDF preview: {error}");
                            self.image = None;
                            self.image_source = None;
                            self.picker = Picker::halfblocks();
                            self.viewport_pixels = None;
                        }
                    }
                }
            }
            Some(PreparedImage::Raster(viewport))
                if (!cached_only || self.image_presented)
                    && viewport
                        .image
                        .needs_resize(&Resize::Crop(None), content.as_size())
                        .is_none() =>
            {
                frame.render_stateful_widget(
                    StatefulImage::default().resize(Resize::Crop(None)),
                    content,
                    &mut viewport.image,
                );
                self.image_presented = true;
            }
            _ => {
                let [_, message, _] = Layout::vertical([
                    Constraint::Fill(1),
                    Constraint::Length(EMPTY_STATE_HEIGHT),
                    Constraint::Fill(1),
                ])
                .areas(content);
                let notice = if self.source_page.is_none() {
                    let mut notice = Text::from(EMPTY_NOTICE);
                    notice.extend([Line::default(), Line::from("Build PDF from Actions.")]);
                    notice
                } else {
                    Text::from(self.notice.as_str())
                };

                frame.render_widget(
                    Paragraph::new(notice)
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

fn fit_page_zoom_percent(source: (u32, u32), viewport: (u32, u32)) -> u16 {
    let height_at_full_width = fit_width_height(source.0, source.1, viewport.0);
    let percent =
        u64::from(viewport.1) * u64::from(FIT_WIDTH_PERCENT) / u64::from(height_at_full_width);

    // Percentages are whole numbers; never request a zero-sized page.
    percent.clamp(1, u64::from(FIT_WIDTH_PERCENT)) as u16
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
