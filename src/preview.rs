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
    layout::{Constraint, Layout, Margin, Rect},
    style::Style,
    widgets::{Block, Clear, Paragraph, Wrap},
};
use ratatui_image::{
    Resize, ResizeEncodeRender, StatefulImage,
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

pub struct Preview {
    picker: Picker,
    notice: String,
    pages: Option<PdfPages>,
    page_index: usize,
    image: Option<StatefulProtocol>,
    source_page: Option<Arc<DynamicImage>>,
    fitted_page: Option<Arc<DynamicImage>>,
    pending: Option<ViewportJob>,
    viewport_pixels: Option<(u32, u32)>,
    scroll_pixels: u32,
    displayed_scroll: Option<u32>,
}

struct PreparedViewport {
    image: StatefulProtocol,
    fitted: Arc<DynamicImage>,
    scroll: u32,
}

struct ViewportJob {
    source: Arc<DynamicImage>,
    pixels: (u32, u32),
    scroll: u32,
    result: Receiver<Result<PreparedViewport>>,
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
            fitted_page: None,
            pending: None,
            viewport_pixels: None,
            scroll_pixels: 0,
            displayed_scroll: None,
        }
    }
}

impl Preview {
    /// Query graphics support after entering the alternate screen, before reading input events.
    pub fn detect() -> Self {
        Self {
            picker: Picker::from_query_stdio().unwrap_or_else(|_| Picker::halfblocks()),
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
            self.page_index = index;
        }
        Ok(())
    }

    fn set_source(&mut self, source: Option<DynamicImage>) {
        self.source_page = source.map(Arc::new);
        self.fitted_page = None;
        self.viewport_pixels = None;
        self.displayed_scroll = None;
        self.image = None;
    }

    pub fn scroll(&mut self, up: bool) {
        let Some(page) = &self.fitted_page else {
            return;
        };
        let Some((_, viewport_height)) = self.viewport_pixels else {
            return;
        };

        let step = SCROLL_LINES * u32::from(self.picker.font_size().height);
        let maximum_scroll = page.height().saturating_sub(viewport_height);
        self.scroll_pixels = if up {
            self.scroll_pixels.saturating_sub(step)
        } else {
            self.scroll_pixels.saturating_add(step).min(maximum_scroll)
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
                && job.scroll == self.scroll_pixels;
            self.pending = None;

            if current {
                match result {
                    Ok(prepared) => {
                        self.image = Some(prepared.image);
                        self.fitted_page = Some(prepared.fitted);
                        self.scroll_pixels = prepared.scroll;
                        self.displayed_scroll = Some(prepared.scroll);
                        self.viewport_pixels = Some(pixels);
                    }
                    Err(error) => {
                        self.notice = format!("Could not display the preview: {error}");
                        self.image = None;
                        if !self.uses_basic_rendering() {
                            self.picker = Picker::halfblocks();
                            self.fitted_page = None;
                        } else {
                            return;
                        }
                    }
                }
            }
        }

        if self.viewport_pixels == Some(pixels) && self.displayed_scroll == Some(self.scroll_pixels)
        {
            return;
        }

        let picker = self.picker.clone();
        let worker_source = Arc::clone(&source);
        let fitted = self.fitted_page.clone();
        let scroll = self.scroll_pixels;
        let (sender, result) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("texo-preview".into())
            .spawn(move || {
                let prepared = prepare_image(picker, worker_source, fitted, area, scroll);
                let _ = sender.send(prepared);
            });

        match worker {
            Ok(_) => {
                self.pending = Some(ViewportJob {
                    source,
                    pixels,
                    scroll,
                    result,
                });
            }
            Err(error) => self.notice = format!("Could not prepare the preview: {error}"),
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
        let content = area.inner(Margin::new(theme::PANEL_PADDING, 1));
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
    fitted: Option<Arc<DynamicImage>>,
    area: Rect,
    scroll: u32,
) -> Result<PreparedViewport> {
    let font = picker.font_size();
    let width = u32::from(area.width) * u32::from(font.width);
    let height = u32::from(area.height) * u32::from(font.height);
    let fitted = fitted
        .filter(|page| page.width() == width)
        .unwrap_or_else(|| {
            let fitted_height = fit_width_height(source.width(), source.height(), width);
            Arc::new(source.resize_exact(width, fitted_height, FilterType::Lanczos3))
        });

    let (scroll, visible_height) = viewport_crop(fitted.height(), height, scroll);
    let visible = fitted.crop_imm(0, scroll, width, visible_height);
    let mut image = picker.new_resize_protocol(visible);
    image.resize_encode(&Resize::Crop(None), area.as_size());
    if let Some(result) = image.last_encoding_result() {
        result?;
    }

    Ok(PreparedViewport {
        image,
        fitted,
        scroll,
    })
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

    const WORKER_TEST_TIMEOUT: Duration = Duration::from_secs(5);
    const WORKER_TEST_POLL_INTERVAL: Duration = Duration::from_millis(5);

    fn finish_viewport(preview: &mut Preview, area: Rect) {
        let deadline = std::time::Instant::now() + WORKER_TEST_TIMEOUT;
        loop {
            preview.prepare_viewport(area);
            if preview.pending.is_none() {
                assert!(preview.image.is_some());
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "Preview worker timed out"
            );
            thread::sleep(WORKER_TEST_POLL_INTERVAL);
        }
    }

    #[test]
    fn pending_image_work_does_not_block_or_replace_a_new_page() {
        let mut preview = Preview::default();
        preview.set_source(Some(DynamicImage::new_rgb8(120, 200)));
        let area = Rect::new(0, 0, 10, 5);
        let source = preview.source_page.clone().unwrap();
        let (sender, result) = mpsc::channel();
        preview.pending = Some(ViewportJob {
            source: source.clone(),
            pixels: (100, 100),
            scroll: 0,
            result,
        });

        // The worker has not sent anything: drawing must still return immediately.
        preview.prepare_viewport(area);
        assert!(preview.pending.is_some());
        assert!(preview.image.is_none());

        preview.clear();
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(60, 20)).unwrap();
        terminal
            .draw(|frame| preview.draw(frame, frame.area()))
            .unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains("Your PDF will appear here."));
        assert!(!text.contains("Preparing PDF"));

        preview.set_source(Some(DynamicImage::new_rgb8(60, 60)));
        sender
            .send(prepare_image(Picker::halfblocks(), source, None, area, 0))
            .unwrap();
        preview.prepare_viewport(area);
        assert!(
            preview.image.is_none(),
            "The previous page must not reappear"
        );

        finish_viewport(&mut preview, area);
        assert_eq!(preview.fitted_page.as_ref().unwrap().height(), 100);
        preview.prepare_viewport(area);
        assert!(
            preview.pending.is_none(),
            "An unchanged viewport must use its cache"
        );
    }

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
        let mut preview = Preview::default();
        preview.set_source(Some(DynamicImage::new_rgb8(120, 200)));
        finish_viewport(&mut preview, Rect::new(0, 0, 10, 5));
        assert_eq!(preview.fitted_page.as_ref().unwrap().width(), 100);

        preview.scroll(false);
        assert_eq!(preview.scroll_pixels, 60);
        preview.scroll(false);
        assert_eq!(preview.scroll_pixels, 66);
        finish_viewport(&mut preview, Rect::new(0, 0, 10, 5));
        assert_eq!(preview.displayed_scroll, Some(66));

        finish_viewport(&mut preview, Rect::new(0, 0, 10, 20));
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
