use super::*;

const WORKER_TEST_TIMEOUT: Duration = Duration::from_secs(5);
const WORKER_TEST_POLL_INTERVAL: Duration = Duration::from_millis(5);

fn patterned_page(width: u32, height: u32) -> DynamicImage {
    DynamicImage::ImageRgb8(image::RgbImage::from_fn(width, height, |x, y| {
        let is_mark = x % 30 < 10 && y % 40 < 10;
        let color = if is_mark { 0 } else { 255 };

        image::Rgb([color; 3])
    }))
}

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
fn failed_preview_does_not_retry_until_the_viewport_changes() {
    let mut preview = Preview::default();
    preview.set_source(Some(DynamicImage::new_rgb8(120, 200)));
    let area = Rect::new(0, 0, 10, 5);
    let font = preview.picker.font_size();
    let (sender, result) = mpsc::channel();

    preview.pending = Some(ViewportJob {
        hovered_link: None,
        source: preview.source_page.clone().unwrap(),
        pixels: (
            u32::from(area.width) * u32::from(font.width),
            u32::from(area.height) * u32::from(font.height),
        ),
        scroll: 0,
        result,
        zoom_percent: FIT_WIDTH_PERCENT,
        horizontal: 0,
    });
    sender
        .send(Err(anyhow!("Synthetic encoding failure")))
        .unwrap();

    preview.prepare_viewport(area);
    preview.prepare_viewport(area);
    assert!(preview.pending.is_none());
    assert!(preview.image.is_none());
    assert!(preview.notice.contains("Synthetic encoding failure"));

    finish_viewport(&mut preview, Rect::new(0, 0, 12, 5));
    assert!(preview.image.is_some());
}

#[test]
fn pending_image_work_does_not_block_or_replace_a_new_page() {
    let mut preview = Preview::default();
    preview.set_source(Some(DynamicImage::new_rgb8(120, 200)));
    let area = Rect::new(0, 0, 10, 5);
    let source = preview.source_page.clone().unwrap();
    let (sender, result) = mpsc::channel();
    preview.pending = Some(ViewportJob {
        hovered_link: None,
        source: source.clone(),
        pixels: (100, 100),
        scroll: 0,
        result,
        zoom_percent: FIT_WIDTH_PERCENT,
        horizontal: 0,
    });

    // The worker has not sent anything: drawing must still return immediately.
    preview.prepare_viewport(area);
    assert!(preview.pending.is_some());
    assert!(preview.image.is_none());

    preview.clear();
    assert!(!preview.is_preparing());
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(60, 20)).unwrap();
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
        .send(
            prepare_image(Picker::halfblocks(), source, area, 0, 0, FIT_WIDTH_PERCENT)
                .map(PreparedImage::Raster),
        )
        .unwrap();
    preview.prepare_viewport(area);
    assert!(
        preview.image.is_none(),
        "The previous page must not reappear"
    );

    finish_viewport(&mut preview, area);
    assert_eq!(preview.scaled_page_size.unwrap().1, 100);
    preview.prepare_viewport(area);
    assert!(
        preview.pending.is_none(),
        "An unchanged viewport must use its cache"
    );
}

#[test]
fn kitty_preview_transmits_image_on_first_draw_after_background_preparation() -> anyhow::Result<()>
{
    let mut preview = Preview::default();
    preview.picker.set_protocol_type(ProtocolType::Kitty);
    preview.set_source(Some(DynamicImage::new_rgb8(120, 60)));
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(20, 12))?;
    let content = Rect::new(0, 0, 20, 12).inner(Margin::new(theme::PANEL_PADDING, 1));
    finish_viewport(&mut preview, content);

    terminal.draw(|frame| preview.draw(frame, frame.area()))?;
    let first: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(
        first.contains("a=T"),
        "The first frame must transmit the image, not only its placeholders"
    );

    terminal.draw(|frame| preview.draw(frame, frame.area()))?;
    let next: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(
        !next.contains("a=T"),
        "Unchanged frames must reuse the transmitted image"
    );
    assert!(preview.pending.is_none());
    Ok(())
}

#[test]
fn loading_state_replaces_previous_content_and_animates() -> anyhow::Result<()> {
    use ratatui::{Terminal, backend::TestBackend};
    let mut terminal = Terminal::new(TestBackend::new(60, 20))?;
    let mut captures = Vec::new();
    for elapsed in [Duration::ZERO, Duration::from_millis(150)] {
        terminal.draw(|frame| {
            frame.render_widget(Paragraph::new("Previous PDF"), frame.area());
            Preview::draw_loading_at(frame, frame.area(), elapsed);
        })?;
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains("Loading PDF…"));
        assert!(!text.contains("Previous PDF"));
        captures.push(text);
    }
    assert_ne!(captures[0], captures[1]);
    Ok(())
}

#[test]
fn loading_continues_until_the_replacement_image_is_ready() -> anyhow::Result<()> {
    use ratatui::{Terminal, backend::TestBackend};

    let mut preview = Preview::default();
    let mut terminal = Terminal::new(TestBackend::new(60, 20))?;
    let area = terminal.get_frame().area();
    let content = preview_content(area);
    preview.set_source(Some(patterned_page(120, 200)));
    finish_viewport(&mut preview, content);
    terminal.draw(|frame| preview.draw(frame, area))?;
    assert!(preview.has_presented_image());

    preview.begin_loading();
    let started = preview.loading_started;
    terminal.draw(|frame| preview.draw_loading(frame, area))?;

    // Hold encoding pending so the compilation-to-image handoff is deterministic.
    preview.set_source(Some(patterned_page(200, 120)));
    let source = preview.source_page.clone().unwrap();
    let font = preview.picker.font_size();
    let (sender, result) = mpsc::channel();
    preview.pending = Some(ViewportJob {
        hovered_link: None,
        source: source.clone(),
        pixels: (
            u32::from(content.width) * u32::from(font.width),
            u32::from(content.height) * u32::from(font.height),
        ),
        scroll: 0,
        result,
        zoom_percent: FIT_WIDTH_PERCENT,
        horizontal: 0,
    });
    terminal.draw(|frame| preview.draw(frame, area))?;
    let loading: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(loading.contains("Loading PDF…"));
    assert!(!loading.contains("Preparing PDF"));
    assert_eq!(preview.loading_started, started);

    sender.send(
        prepare_image(
            Picker::halfblocks(),
            source,
            content,
            0,
            0,
            FIT_WIDTH_PERCENT,
        )
        .map(PreparedImage::Raster),
    )?;
    terminal.draw(|frame| preview.draw(frame, area))?;
    let ready: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(!ready.contains("Loading PDF…"));
    assert!(!preview.is_preparing());
    assert!(preview.has_presented_image());
    assert!(preview.loading_started.is_none());
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
    assert_eq!(preview.scaled_page_size.unwrap().0, 100);

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
fn kitty_zoom_reuses_the_upload_through_the_terminal_backend() -> anyhow::Result<()> {
    use ratatui::{Terminal, TerminalOptions, Viewport, backend::CrosstermBackend};
    use std::io::{Read, Seek, SeekFrom};

    let mut preview = Preview::default();
    preview.picker.set_protocol_type(ProtocolType::Kitty);
    preview.set_source(Some(patterned_page(120, 200)));
    let area = Rect::new(0, 0, 24, 12);
    let mut wire = tempfile::tempfile()?;
    let mut terminal = Terminal::with_options(
        CrosstermBackend::new(wire.try_clone()?),
        TerminalOptions {
            viewport: Viewport::Fixed(area),
        },
    )?;

    for percent in [100, 125] {
        preview.set_zoom(percent, area, Position::new(4, 2));

        let start = wire.metadata()?.len();
        let deadline = std::time::Instant::now() + WORKER_TEST_TIMEOUT;
        loop {
            terminal.draw(|frame| preview.draw(frame, frame.area()))?;
            if preview.pending.is_none() {
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            thread::sleep(WORKER_TEST_POLL_INTERVAL);
        }

        wire.seek(SeekFrom::Start(start))?;
        let mut output = String::new();
        wire.read_to_string(&mut output)?;
        assert!(
            output.contains("a=p"),
            "zoom={percent}%: placement was lost"
        );
        assert_eq!(output.contains("a=T"), percent == 100);
        assert!(
            preview.pending.is_none(),
            "Zoom must not start another encoder"
        );
    }
    Ok(())
}

#[test]
#[ignore = "measures interactive latency with a high-resolution viewport"]
fn high_resolution_zoom_prepares_within_interactive_budget() -> anyhow::Result<()> {
    const INTERACTIVE_ZOOM_BUDGET: Duration = Duration::from_secs(1);
    let source = Arc::new(patterned_page(1200, 1600));

    for percent in [100, 125, 200, 300] {
        let started = std::time::Instant::now();
        prepare_image(
            Picker::halfblocks(),
            source.clone(),
            Rect::new(0, 0, 160, 60),
            0,
            0,
            percent,
        )?;
        let elapsed = started.elapsed();

        eprintln!("Zoom {percent}% preparation: {elapsed:?}");
        assert!(
            elapsed < INTERACTIVE_ZOOM_BUDGET,
            "Zoom {percent}% exceeded the interactive budget: {elapsed:?}"
        );
    }

    Ok(())
}

#[test]
fn zoom_changes_visible_page_content() -> anyhow::Result<()> {
    let mut preview = Preview::default();
    preview.set_source(Some(patterned_page(120, 200)));
    let area = Rect::new(0, 0, 24, 12);
    let content = preview_content(area);
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(24, 12))?;
    finish_viewport(&mut preview, content);
    terminal.draw(|frame| preview.draw(frame, frame.area()))?;
    let original = terminal.backend().buffer().clone();

    for percent in [125, 200, 100, 75, 100] {
        preview.set_zoom(percent, area, Position::new(content.x, content.y));
        finish_viewport(&mut preview, content);
        terminal.draw(|frame| preview.draw(frame, frame.area()))?;

        if percent == FIT_WIDTH_PERCENT {
            assert_eq!(&original, terminal.backend().buffer());
        } else {
            assert_ne!(&original, terminal.backend().buffer());
        }
    }

    Ok(())
}

#[test]
fn source_viewport_keeps_fractional_edges_and_stays_inside_the_page() {
    let source = (1200, 1600);
    let scaled = (3600, 4800);
    let fractional = source_viewport(
        source,
        scaled,
        image::math::Rect {
            x: 1,
            y: 1,
            width: 1200,
            height: 800,
        },
    );
    assert_eq!(
        fractional,
        image::math::Rect {
            x: 0,
            y: 0,
            width: 401,
            height: 267
        }
    );

    let bottom_right = source_viewport(
        source,
        scaled,
        image::math::Rect {
            x: 2400,
            y: 4000,
            width: 1200,
            height: 800,
        },
    );
    assert_eq!(bottom_right.x + bottom_right.width, source.0);
    assert_eq!(bottom_right.y + bottom_right.height, source.1);
}

#[test]
fn trackpad_zoom_uses_paced_steps_without_replaying_a_swipe() {
    let mut preview = Preview::default();
    preview.set_source(Some(patterned_page(120, 200)));
    let area = Rect::new(0, 0, 24, 12);
    let pointer = Position::new(12, 6);
    let started = Instant::now();

    // A burst of wheel reports represents one short trackpad swipe.
    for elapsed_ms in 0..100 {
        preview.zoom_from_wheel_at(
            true,
            area,
            pointer,
            started + Duration::from_millis(elapsed_ms),
        );
    }
    assert_eq!(preview.zoom_percent(), 120);

    preview.zoom_from_wheel_at(true, area, pointer, started + WHEEL_ZOOM_INTERVAL);
    assert_eq!(preview.zoom_percent(), 140);

    preview.zoom_from_wheel_at(false, area, pointer, started + WHEEL_ZOOM_INTERVAL * 2);
    assert_eq!(preview.zoom_percent(), 120);

    // There is no queued gesture work after input ends, and keyboard steps stay immediate.
    finish_viewport(&mut preview, preview_content(area));
    assert_eq!(preview.zoom_percent(), 120);
    preview.zoom_at(true, area, pointer);
    preview.zoom_at(true, area, pointer);
    assert_eq!(preview.zoom_percent(), 160);
}

#[test]
fn zoom_out_stops_at_the_whole_page_and_tracks_viewport_changes() {
    let mut preview = Preview::default();
    preview.set_source(Some(patterned_page(600, 900)));
    let area = Rect::new(0, 0, 84, 32);
    let pointer = Position::new(42, 16);
    let content = preview_content(area);

    for _ in 0..30 {
        preview.zoom_at(false, area, pointer);
    }
    finish_viewport(&mut preview, content);
    assert_eq!(preview.zoom_percent(), 50);
    assert_eq!(preview.scaled_page_size, Some((400, 600)));
    assert_eq!(preview.scroll_pixels, 0);
    assert_eq!(preview.horizontal_pixels, 0);

    preview.zoom_at(false, area, pointer);
    preview.prepare_viewport(content);
    assert!(
        preview.pending.is_none(),
        "The minimum must not trigger another render"
    );

    // A taller viewport can fit the whole page at its full width.
    finish_viewport(&mut preview, Rect::new(0, 0, 80, 80));
    assert_eq!(preview.zoom_percent(), FIT_WIDTH_PERCENT);
    assert_eq!(preview.scaled_page_size, Some((800, 1200)));

    preview.zoom_at(true, area, pointer);
    preview.reset_zoom(area);
    assert_eq!(preview.zoom_percent(), 50);

    preview.set_source(Some(patterned_page(900, 600)));
    preview.reset_zoom(area);
    assert_eq!(preview.zoom_percent(), FIT_WIDTH_PERCENT);
}

#[test]
fn zoom_anchors_the_cursor_clamps_limits_and_resets_to_fit_page() {
    let mut preview = Preview::default();
    preview.set_source(Some(DynamicImage::new_rgb8(120, 200)));
    let area = Rect::new(0, 0, 14, 7);
    let content = preview_content(area);
    let pointer = Position::new(content.x + 4, content.y + 2);
    finish_viewport(&mut preview, content);

    preview.zoom_at(true, area, pointer);
    assert_eq!(preview.zoom_percent(), 120);
    assert_eq!(preview.horizontal_pixels, 8);
    assert_eq!(preview.scroll_pixels, 8);
    finish_viewport(&mut preview, content);
    assert_eq!(preview.scaled_page_size.unwrap().0, 120);

    for _ in 0..20 {
        preview.zoom_at(true, area, pointer);
    }
    assert_eq!(preview.zoom_percent(), MAX_ZOOM_PERCENT);
    finish_viewport(&mut preview, content);
    for _ in 0..20 {
        preview.pan_horizontal(false);
    }
    assert_eq!(preview.horizontal_pixels, 200);
    preview.pan_horizontal(true);
    assert_eq!(preview.horizontal_pixels, 170);

    preview.reset_zoom(area);
    finish_viewport(&mut preview, content);
    assert_eq!(preview.zoom_percent(), 60);
    assert_eq!(preview.horizontal_pixels, 0);
    assert_eq!(preview.scaled_page_size.unwrap().0, 60);

    for _ in 0..20 {
        preview.zoom_at(false, area, pointer);
    }
    finish_viewport(&mut preview, content);
    assert_eq!(preview.zoom_percent(), 60);
    assert_eq!(preview.horizontal_pixels, 0);
    assert_eq!(preview.scroll_pixels, 0);

    preview.clear();
    preview.zoom_at(true, area, pointer);
    assert_eq!(preview.zoom_percent(), FIT_WIDTH_PERCENT);
    preview.set_source(Some(DynamicImage::new_rgb8(120, 200)));
    preview.zoom_at(true, Rect::default(), pointer);
    assert_eq!(preview.zoom_percent(), FIT_WIDTH_PERCENT);
}

#[test]
fn late_zoom_work_cannot_replace_the_latest_scale_or_pan() {
    let mut preview = Preview::default();
    preview.set_source(Some(DynamicImage::new_rgb8(120, 200)));
    let area = Rect::new(0, 0, 14, 7);
    let content = preview_content(area);
    let source = preview.source_page.clone().unwrap();
    let (sender, result) = mpsc::channel();
    preview.pending = Some(ViewportJob {
        hovered_link: None,
        source: source.clone(),
        pixels: (100, 100),
        scroll: 0,
        result,
        zoom_percent: FIT_WIDTH_PERCENT,
        horizontal: 0,
    });

    preview.zoom_at(true, area, Position::new(content.x + 4, content.y + 2));
    sender
        .send(
            prepare_image(
                Picker::halfblocks(),
                source,
                content,
                0,
                0,
                FIT_WIDTH_PERCENT,
            )
            .map(PreparedImage::Raster),
        )
        .unwrap();
    preview.prepare_viewport(content);
    assert!(preview.image.is_none());

    finish_viewport(&mut preview, content);
    assert_eq!(preview.scaled_page_size.unwrap().0, 120);
    assert_eq!(preview.displayed_horizontal, Some(8));
    assert_eq!(preview.displayed_scroll, Some(8));
    preview.prepare_viewport(content);
    assert!(preview.pending.is_none());

    preview.pan_horizontal(false);
    preview.prepare_viewport(content);
    preview.pan_horizontal(true);
    finish_viewport(&mut preview, content);
    assert_eq!(preview.displayed_horizontal, Some(0));
}

#[test]
fn kitty_reuses_uploaded_page_after_zooming() -> anyhow::Result<()> {
    let mut preview = Preview::default();
    preview.picker.set_protocol_type(ProtocolType::Kitty);
    preview.set_source(Some(DynamicImage::new_rgb8(120, 200)));
    let area = Rect::new(0, 0, 20, 12);
    let content = preview_content(area);
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(20, 12))?;
    finish_viewport(&mut preview, content);
    terminal.draw(|frame| preview.draw(frame, frame.area()))?;

    preview.zoom_at(true, area, Position::new(10, 6));
    finish_viewport(&mut preview, content);
    terminal.draw(|frame| preview.draw(frame, frame.area()))?;
    let output: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(output.contains("a=p"));
    assert!(!output.contains("a=T"));
    assert!(preview.pending.is_none());
    assert_eq!(preview.scaled_page_size.unwrap().0, 192);
    Ok(())
}

#[test]
fn zoomed_out_page_is_centered_and_clears_old_image_cells() -> anyhow::Result<()> {
    let mut preview = Preview::default();
    preview.set_source(Some(DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
        120,
        200,
        image::Rgb([255, 255, 255]),
    ))));
    let area = Rect::new(0, 0, 24, 12);
    let content = preview_content(area);
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(24, 12))?;
    finish_viewport(&mut preview, content);
    terminal.draw(|frame| preview.draw(frame, frame.area()))?;

    preview.zoom_at(false, area, Position::new(12, 6));
    preview.zoom_at(false, area, Position::new(12, 6));
    finish_viewport(&mut preview, content);
    terminal.draw(|frame| preview.draw(frame, frame.area()))?;
    let buffer = terminal.backend().buffer();
    assert_eq!(
        buffer[(content.x, content.y)].fg,
        ratatui::style::Color::Rgb(0, 0, 0)
    );
    assert_eq!(
        buffer[(content.x + 10, content.y)].fg,
        ratatui::style::Color::Rgb(255, 255, 255)
    );
    assert_eq!(preview.zoom_percent(), 60);
    Ok(())
}

#[test]
fn viewport_scroll_reaches_bottom_without_reading_past_paper() {
    assert_eq!(viewport_crop(1200, 800, 150), (150, 800));
    assert_eq!(viewport_crop(1200, 800, 1000), (400, 800));
    assert_eq!(viewport_crop(1200, 1500, 400), (0, 1200));
}

#[test]
fn native_page_preparation_remains_valid_when_zoom_changes_before_completion() {
    let mut preview = Preview::default();
    preview.picker.set_protocol_type(ProtocolType::Kitty);
    preview.set_source(Some(patterned_page(120, 200)));
    let area = Rect::new(0, 0, 24, 12);
    let content = preview_content(area);
    let source = preview.source_page.clone().unwrap();
    let (sender, result) = mpsc::channel();
    preview.pending = Some(ViewportJob {
        hovered_link: None,
        source: source.clone(),
        pixels: (200, 200),
        scroll: 0,
        result,
        zoom_percent: FIT_WIDTH_PERCENT,
        horizontal: 0,
    });
    preview.set_zoom(200, area, Position::new(10, 6));
    sender
        .send(
            RetainedImage::new(
                &preview.picker,
                source.as_ref().clone(),
                preview.terminal_images.clone(),
            )
            .map(PreparedImage::Retained),
        )
        .unwrap();

    preview.prepare_viewport(content);
    assert!(matches!(preview.image, Some(PreparedImage::Retained(_))));
    assert!(preview.pending.is_none());
    assert_eq!(preview.scaled_page_size.unwrap().0, 400);
    assert!(!preview.is_preparing());
}

#[test]
fn native_zoom_pan_and_resize_do_not_start_another_worker() -> anyhow::Result<()> {
    let mut preview = Preview::default();
    preview.picker.set_protocol_type(ProtocolType::Kitty);
    preview.set_source(Some(patterned_page(120, 200)));
    let area = Rect::new(0, 0, 24, 12);
    finish_viewport(&mut preview, preview_content(area));
    let source = preview.image_source.clone().unwrap();

    for percent in [110, 150, 200, 250, 300, 200, 100] {
        preview.set_zoom(percent, area, Position::new(10, 6));
        preview.prepare_viewport(preview_content(area));
        preview.pan_horizontal(false);
        preview.scroll(false);
        preview.prepare_viewport(preview_content(area));

        assert!(preview.pending.is_none());
        assert!(!preview.is_preparing());
        assert!(Arc::ptr_eq(preview.image_source.as_ref().unwrap(), &source));
        let viewport = preview.retained_viewport(&source, preview_content(area));
        assert_eq!(viewport.view.area.width, preview_content(area).width);
        assert!(viewport.view.offset.x + viewport.view.area.width <= viewport.view.page_size.width);
        assert!(
            viewport.view.offset.y + viewport.view.area.height <= viewport.view.page_size.height
        );
    }

    preview.prepare_viewport(Rect::new(2, 2, 8, 5));
    assert!(preview.pending.is_none());
    assert!(!preview.is_preparing());
    Ok(())
}

#[test]
fn a_cached_native_page_stays_inside_a_resized_pane() -> anyhow::Result<()> {
    let mut preview = Preview::default();
    preview.picker.set_protocol_type(ProtocolType::Kitty);
    preview.set_source(Some(patterned_page(120, 200)));
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(40, 20))?;
    finish_viewport(&mut preview, preview_content(Rect::new(0, 0, 40, 20)));
    terminal.draw(|frame| preview.draw(frame, frame.area()))?;

    // A new source is queued while the modal keeps the already displayed page underneath it.
    let previous = preview.image_source.clone().unwrap();
    preview.set_source(Some(patterned_page(200, 120)));
    for (width, height) in [(12, 6), (2, 2), (60, 30)] {
        terminal.backend_mut().resize(width, height);
        terminal.resize(Rect::new(0, 0, width, height))?;
        terminal.draw(|frame| preview.draw_cached(frame, frame.area()))?;
        assert!(
            preview.pending.is_none(),
            "A modal must not consume a new upload"
        );
        assert!(preview.is_preparing(), "The new source must remain queued");
        assert!(Arc::ptr_eq(
            preview.image_source.as_ref().unwrap(),
            &previous
        ));
    }
    Ok(())
}

#[test]
#[ignore = "measures complete retained-image draw and terminal output latency"]
fn high_resolution_native_zoom_draws_without_reuploading_pixels() -> anyhow::Result<()> {
    use ratatui::{Terminal, TerminalOptions, Viewport, backend::CrosstermBackend};
    use std::io::{BufWriter, Read, Seek, SeekFrom};

    const INTERACTIVE_FRAME_BUDGET: Duration = Duration::from_millis(50);
    let area = Rect::new(0, 0, 160, 60);
    let mut preview = Preview::default();
    preview.picker.set_protocol_type(ProtocolType::Kitty);
    preview.set_source(Some(patterned_page(1200, 1600)));
    finish_viewport(&mut preview, preview_content(area));
    let mut wire = tempfile::tempfile()?;

    // Match buffered stdout; unbuffered file writes distort the rendering latency.
    let mut terminal = Terminal::with_options(
        CrosstermBackend::new(BufWriter::new(wire.try_clone()?)),
        TerminalOptions {
            viewport: Viewport::Fixed(area),
        },
    )?;
    terminal.draw(|frame| preview.draw(frame, frame.area()))?;
    terminal.draw(|frame| preview.draw(frame, frame.area()))?;
    let upload_bytes = wire.metadata()?.len();

    for percent in [110, 125, 150, 200, 250, 300, 200, 100] {
        let start = wire.metadata()?.len();
        let started = Instant::now();
        preview.set_zoom(percent, area, Position::new(80, 30));
        terminal.draw(|frame| preview.draw(frame, frame.area()))?;
        let elapsed = started.elapsed();
        let bytes = wire.metadata()?.len() - start;
        eprintln!(
            "zoom={percent}%: {elapsed:?}, {bytes} bytes (initial upload {upload_bytes} bytes)"
        );

        wire.seek(SeekFrom::Start(start))?;
        let mut output = String::new();
        wire.read_to_string(&mut output)?;
        assert!(output.contains("a=p"));
        assert!(!output.contains("a=T"));
        assert!(preview.pending.is_none());
        assert!(
            elapsed < INTERACTIVE_FRAME_BUDGET,
            "Zoom draw exceeded the interactive budget"
        );
    }
    Ok(())
}

#[test]
#[ignore = "requires the Kitty executable for headless terminal rendering"]
fn terminal_renderer_actually_zooms_and_pans_the_page() -> anyhow::Result<()> {
    // Kitty uses foreground colors as image IDs, even when the test runner sets NO_COLOR.
    ratatui::crossterm::style::force_color_output(true);

    use ratatui::{Terminal, TerminalOptions, Viewport, backend::CrosstermBackend};
    use std::{
        io::{Read, Seek, SeekFrom},
        process::Command,
    };

    let directory = tempfile::tempdir()?;
    let area = Rect::new(0, 0, 40, 20);
    let content = preview_content(area);
    let mut preview = Preview::default();
    preview.picker.set_protocol_type(ProtocolType::Kitty);
    preview.set_source(Some(patterned_page(120, 200)));
    let mut wire = tempfile::tempfile()?;
    let mut terminal = Terminal::with_options(
        CrosstermBackend::new(wire.try_clone()?),
        TerminalOptions {
            viewport: Viewport::Fixed(area),
        },
    )?;

    for phase in ["initial", "zoom", "pan", "drag", "fit"] {
        match phase {
            "zoom" => preview.set_zoom(200, area, content.as_position()),
            "pan" => {
                preview.pan_horizontal(false);
                preview.scroll(false);
            }
            "drag" => {
                let start = Position::new(content.x + 8, content.y + 6);
                preview.begin_drag(area, start);
                assert!(preview.drag_to(Position::new(start.x - 2, start.y - 2)));
                preview.end_drag();
            }
            "fit" => preview.reset_zoom(area),
            _ => {}
        }
        finish_viewport(&mut preview, content);
        let start = wire.metadata()?.len();
        terminal.draw(|frame| preview.draw(frame, frame.area()))?;
        wire.seek(SeekFrom::Start(start))?;
        let mut bytes = Vec::new();
        wire.read_to_end(&mut bytes)?;
        std::fs::write(directory.path().join(format!("{phase}.bin")), bytes)?;
    }

    let kitty = if cfg!(target_os = "macos") {
        "/Applications/kitty.app/Contents/MacOS/kitty"
    } else {
        "kitty"
    };
    let result = Command::new(kitty)
        .args([
            "+runpy",
            concat!(
                "exec(compile(open('tests/kitty_geometry.py').read(), ",
                "'tests/kitty_geometry.py', 'exec', optimize=0), {'__name__': '__main__'})",
            ),
        ])
        .arg(directory.path())
        .output()?;
    anyhow::ensure!(
        result.status.success(),
        "Terminal renderer regression: {}{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    eprintln!("{}", String::from_utf8_lossy(&result.stdout));
    Ok(())
}

#[test]
fn scrolling_beyond_native_coordinates_uses_raster_and_can_return_to_native() {
    let mut preview = Preview::default();
    preview.picker.set_protocol_type(ProtocolType::Kitty);
    preview.set_source(Some(patterned_page(120, 2400)));
    let content = preview_content(Rect::new(0, 0, 40, 20));
    finish_viewport(&mut preview, content);
    assert!(matches!(preview.image, Some(PreparedImage::Retained(_))));

    preview.scroll_pixels = u32::MAX;
    finish_viewport(&mut preview, content);
    assert!(matches!(preview.image, Some(PreparedImage::Raster(_))));
    assert_eq!(preview.picker.protocol_type(), ProtocolType::Kitty);

    preview.scroll_pixels = 0;
    finish_viewport(&mut preview, content);
    assert!(matches!(preview.image, Some(PreparedImage::Retained(_))));
    assert_eq!(preview.displayed_scroll, Some(0));
}

#[test]
fn dragging_moves_paper_in_both_axes_and_reverses_immediately_at_edges() {
    let mut preview = Preview::default();
    preview.picker.set_protocol_type(ProtocolType::Kitty);
    preview.set_source(Some(patterned_page(120, 200)));
    let area = Rect::new(0, 0, 24, 12);
    preview.set_zoom(200, area, preview_content(area).as_position());
    finish_viewport(&mut preview, preview_content(area));

    preview.begin_drag(area, Position::new(12, 6));
    assert!(preview.drag_to(Position::new(9, 4)));
    assert_eq!((preview.horizontal_pixels, preview.scroll_pixels), (30, 40));
    preview.prepare_viewport(preview_content(area));
    assert!(
        preview.pending.is_none(),
        "Dragging must reuse the uploaded page"
    );

    assert!(preview.drag_to(Position::new(u16::MAX, u16::MAX)));
    assert_eq!((preview.horizontal_pixels, preview.scroll_pixels), (0, 0));
    assert!(preview.drag_to(Position::new(u16::MAX - 1, u16::MAX - 1)));
    assert_eq!((preview.horizontal_pixels, preview.scroll_pixels), (10, 20));

    assert!(preview.drag_to(Position::new(0, 0)));
    let page = preview.scaled_page_size.unwrap();
    let viewport = preview.viewport_pixels.unwrap();
    assert_eq!(preview.horizontal_pixels, page.0 - viewport.0);
    assert_eq!(preview.scroll_pixels, page.1 - viewport.1);
    preview.end_drag();
    assert!(!preview.drag_to(Position::new(12, 6)));
}

#[test]
fn dragging_requires_page_content_and_cancels_when_the_source_changes() {
    let mut preview = Preview::default();
    let area = Rect::new(0, 0, 24, 12);
    preview.begin_drag(area, Position::new(12, 6));
    assert!(!preview.drag_to(Position::new(10, 4)));

    preview.set_source(Some(patterned_page(120, 200)));
    finish_viewport(&mut preview, preview_content(area));
    preview.begin_drag(area, Position::new(0, 0));
    assert!(!preview.drag_to(Position::new(10, 4)));

    preview.begin_drag(area, Position::new(12, 6));
    preview.set_source(Some(patterned_page(120, 240)));
    assert!(!preview.drag_to(Position::new(10, 4)));
}

#[test]
fn link_hit_testing_uses_visible_cells_and_rejects_gutters() {
    let mapping = DisplayedPage {
        pane: Rect::new(0, 0, 20, 10),
        image: [5.5, 1.0, 15.5, 6.0],
        source: [0.0, 0.0, 1.0, 1.0],
    };
    let links = vec![crate::pdf::PdfLink {
        bounds: [0.21, 0.21, 0.22, 0.22],
        uri: "https://example.com/tiny-icon".into(),
    }];
    assert_eq!(
        mapping.link_at(&links, Position::new(7, 2)),
        Some(links[0].uri.as_str())
    );
    assert_eq!(mapping.link_at(&links, Position::new(4, 2)), None);
    assert_eq!(mapping.link_at(&links, Position::new(7, 6)), None);
}

#[test]
fn retained_link_mapping_uses_the_actual_rounded_cell_grid() {
    let mapping = retained_mapping(
        Rect::new(0, 0, 20, 10),
        ImageView {
            page_size: Size::new(40, 30),
            offset: Position::new(8, 5),
            area: Rect::new(2, 1, 16, 8),
        },
    );
    let links = vec![crate::pdf::PdfLink {
        bounds: [0.25, 0.2, 0.275, 7.0 / 30.0],
        uri: "https://example.com/icon".into(),
    }];
    assert_eq!(
        mapping.link_at(&links, Position::new(4, 2)),
        Some(links[0].uri.as_str())
    );
    assert_eq!(mapping.link_at(&links, Position::new(2, 1)), None);
}

#[test]
fn raster_link_mapping_keeps_fractional_crop_edges() {
    let source = Arc::new(DynamicImage::new_rgb8(103, 217));
    let area = Rect::new(2, 3, 12, 8);
    let prepared = prepare_image(Picker::halfblocks(), source, area, 31, 17, 180).unwrap();
    assert_eq!(
        prepared.mapping.source,
        [8.0 / 103.0, 14.0 / 217.0, 66.0 / 103.0, 92.0 / 217.0]
    );
    assert_eq!(prepared.mapping.image[0], f64::from(area.x));
}

fn draw_link_preview(preview: &mut Preview, area: Rect) {
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 60)).unwrap();
    finish_viewport(preview, preview_content(area));
    terminal.draw(|frame| preview.draw(frame, area)).unwrap();
}

fn link_position(preview: &Preview, area: Rect, uri: &str) -> Position {
    for row in area.y..area.bottom() {
        for column in area.x..area.right() {
            let position = Position::new(column, row);
            if preview.link_at(position) == Some(uri) {
                return position;
            }
        }
    }
    panic!("No visible cell for {uri}");
}

#[test]
#[ignore = "requires Poppler"]
fn links_follow_rendered_pages_zoom_pan_and_resize_without_activating_drags() -> Result<()> {
    let area = Rect::new(2, 2, 80, 40);
    for protocol in [ProtocolType::Halfblocks, ProtocolType::Kitty] {
        let pages = PdfPages::render(
            std::path::Path::new("tests/fixtures/links.pdf"),
            &std::sync::atomic::AtomicBool::new(false),
        )?;
        let mut preview = Preview::default();
        preview.picker.set_protocol_type(protocol);
        preview.set_pages(pages)?;
        let uri = "https://example.com/icon";
        assert_eq!(preview.link_at(Position::new(15, 5)), None);

        draw_link_preview(&mut preview, area);
        let point = link_position(&preview, area, uri);
        preview.begin_drag(area, point);
        assert_eq!(preview.release_link(area, point).as_deref(), Some(uri));
        assert_eq!(preview.release_link(area, point), None);

        preview.begin_drag(area, point);
        preview.drag_to(Position::new(point.x + 1, point.y));
        preview.drag_to(point);
        assert_eq!(preview.release_link(area, point), None);

        preview.zoom_at(true, area, point);
        draw_link_preview(&mut preview, area);
        let point = link_position(&preview, area, uri);
        preview.begin_drag(area, point);
        assert_eq!(preview.release_link(area, point).as_deref(), Some(uri));

        preview.horizontal_pixels = 23;
        preview.scroll_pixels = 19;
        draw_link_preview(&mut preview, area);
        let point = link_position(&preview, area, uri);
        preview.begin_drag(area, point);
        let resized = Rect::new(5, 3, 60, 30);
        assert_eq!(preview.release_link(resized, point), None);
        draw_link_preview(&mut preview, resized);
        let point = link_position(&preview, resized, uri);
        preview.begin_drag(resized, point);
        assert_eq!(preview.release_link(resized, point).as_deref(), Some(uri));

        preview.begin_drag(resized, point);
        preview.next_page()?;
        assert_eq!(preview.release_link(resized, point), None);
        assert_eq!(preview.link_at(point), None);
        preview.reset_zoom(resized);
        draw_link_preview(&mut preview, resized);
        let email = "mailto:person@example.com";
        let point = link_position(&preview, resized, email);
        preview.begin_drag(resized, point);
        assert_eq!(preview.release_link(resized, point).as_deref(), Some(email));

        preview.begin_drag(resized, point);
        preview.begin_loading();
        assert_eq!(preview.release_link(resized, point), None);
        assert_eq!(preview.link_at(point), None);
        preview.clear();
        assert_eq!(preview.link_at(point), None);
    }
    Ok(())
}

#[test]
fn links_sharing_one_cell_require_a_single_distinct_target() {
    let mapping = DisplayedPage {
        pane: Rect::new(0, 0, 10, 10),
        image: [0.0, 0.0, 10.0, 10.0],
        source: [0.0, 0.0, 1.0, 1.0],
    };
    let mut links = vec![
        crate::pdf::PdfLink {
            bounds: [0.21, 0.21, 0.23, 0.23],
            uri: "https://example.com/first".into(),
        },
        crate::pdf::PdfLink {
            bounds: [0.27, 0.21, 0.29, 0.23],
            uri: "https://example.com/second".into(),
        },
    ];
    assert_eq!(mapping.link_at(&links, Position::new(2, 2)), None);
    links[1].uri = links[0].uri.clone();
    assert_eq!(
        mapping.link_at(&links, Position::new(2, 2)),
        Some(links[0].uri.as_str())
    );
}

#[test]
fn hover_tint_changes_only_the_annotation_and_preserves_legibility() {
    let mut pixels = image::RgbImage::from_pixel(20, 20, image::Rgb([255; 3]));
    pixels.put_pixel(10, 10, image::Rgb([0; 3]));
    let mut image = DynamicImage::ImageRgb8(pixels);
    highlight_link(&mut image, [0.25, 0.25, 0.75, 0.75]);
    let pixels = image.to_rgb8();

    assert_eq!(pixels.get_pixel(4, 10).0, [255; 3]);
    assert_eq!(pixels.get_pixel(15, 10).0, [255; 3]);
    let tinted_paper = pixels.get_pixel(6, 6).0;
    assert!(tinted_paper[0] < tinted_paper[2]);
    assert!(tinted_paper.iter().all(|channel| *channel > 220));
    assert!(
        pixels
            .get_pixel(10, 10)
            .0
            .iter()
            .all(|channel| *channel < 32)
    );
}

#[test]
#[ignore = "requires Poppler; reports hover preparation timing"]
fn hover_reuses_preparation_until_target_changes_and_preserves_clicks() -> Result<()> {
    let area = Rect::new(2, 2, 80, 40);
    for protocol in [ProtocolType::Halfblocks, ProtocolType::Kitty] {
        let pages = PdfPages::render(
            std::path::Path::new("tests/fixtures/links.pdf"),
            &std::sync::atomic::AtomicBool::new(false),
        )?;
        let mut preview = Preview::default();
        preview.picker.set_protocol_type(protocol);
        preview.set_pages(pages)?;
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 60))?;
        finish_viewport(&mut preview, preview_content(area));
        terminal.draw(|frame| preview.draw(frame, area))?;
        let original_frame = terminal.backend().buffer().clone();
        let source = preview.source_page.clone().unwrap();
        let original_pixels = source.as_bytes().to_vec();
        let position = link_position(&preview, area, "https://example.com/icon");
        let original_geometry = preview.displayed_page;

        let enter_started = Instant::now();
        preview.hover_at(area, position);
        assert_eq!(preview.hovered_link, Some(0));
        assert!(preview.is_refreshing());
        assert!(
            !preview.is_preparing(),
            "hover must not change the loading title"
        );
        preview.prepare_viewport(preview_content(area));
        assert!(!preview.is_preparing());
        preview.begin_drag(area, position);
        finish_viewport(&mut preview, preview_content(area));
        terminal.draw(|frame| preview.draw(frame, area))?;
        let enter_time = enter_started.elapsed();
        assert_eq!(preview.image_hovered_link, Some(0));
        assert_eq!(preview.displayed_page, original_geometry);
        assert_eq!(
            preview.release_link(area, position).as_deref(),
            Some("https://example.com/icon")
        );
        assert_ne!(terminal.backend().buffer(), &original_frame);
        assert!(Arc::ptr_eq(preview.source_page.as_ref().unwrap(), &source));
        assert_eq!(source.as_bytes(), original_pixels);

        let motion_started = Instant::now();
        for _ in 0..1000 {
            preview.hover_at(area, position);
            preview.prepare_viewport(preview_content(area));
            assert!(
                preview.pending.is_none(),
                "same-link motion must not schedule encoding"
            );
        }
        let motion_time = motion_started.elapsed();

        let leave_started = Instant::now();
        preview.hover_at(area, Position::new(0, 0));
        assert_eq!(preview.hovered_link, None);
        finish_viewport(&mut preview, preview_content(area));
        terminal.draw(|frame| preview.draw(frame, area))?;
        let leave_time = leave_started.elapsed();
        assert_eq!(preview.image_hovered_link, None);
        if protocol == ProtocolType::Halfblocks {
            assert_eq!(terminal.backend().buffer(), &original_frame);
        }
        eprintln!(
            "{protocol:?}: enter {enter_time:?}, 1000 same-link moves {motion_time:?} (0 preparations), leave {leave_time:?}"
        );
    }
    Ok(())
}

#[test]
fn late_hover_preparation_cannot_replace_a_cleared_highlight() {
    let mut preview = Preview::default();
    preview.set_source(Some(patterned_page(120, 200)));
    let area = Rect::new(0, 0, 12, 8);
    finish_viewport(&mut preview, area);
    let source = preview.source_page.clone().unwrap();
    let font = preview.picker.font_size();
    let pixels = (
        u32::from(area.width) * u32::from(font.width),
        u32::from(area.height) * u32::from(font.height),
    );
    let (sender, result) = mpsc::channel();
    preview.pending = Some(ViewportJob {
        hovered_link: Some(0),
        source: source.clone(),
        pixels,
        scroll: 0,
        result,
        zoom_percent: FIT_WIDTH_PERCENT,
        horizontal: 0,
    });
    sender
        .send(
            prepare_image(
                Picker::halfblocks(),
                source.clone(),
                area,
                0,
                0,
                FIT_WIDTH_PERCENT,
            )
            .map(PreparedImage::Raster),
        )
        .unwrap();

    preview.clear_hover();
    assert!(preview.receive_prepared_viewport(&source, pixels));
    assert_eq!(preview.image_hovered_link, None);
    assert!(preview.pending.is_none());
}

#[test]
fn failed_hover_encoding_does_not_retry_every_frame() {
    let mut preview = Preview::default();
    preview.set_source(Some(patterned_page(120, 200)));
    let area = Rect::new(0, 0, 12, 8);
    finish_viewport(&mut preview, area);
    let source = preview.source_page.clone().unwrap();
    let pixels = preview.viewport_pixels.unwrap();
    let (sender, result) = mpsc::channel();
    preview.hovered_link = Some(0);
    preview.pending = Some(ViewportJob {
        hovered_link: Some(0),
        source: source.clone(),
        pixels,
        scroll: 0,
        result,
        zoom_percent: FIT_WIDTH_PERCENT,
        horizontal: 0,
    });
    sender
        .send(Err(anyhow!("Synthetic hover encoding failure")))
        .unwrap();

    assert!(!preview.receive_prepared_viewport(&source, pixels));
    preview.prepare_viewport(area);
    assert!(preview.pending.is_none());
    assert!(!preview.is_refreshing());
}
