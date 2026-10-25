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
    assert_eq!(preview.zoom_percent(), 110);

    preview.zoom_from_wheel_at(true, area, pointer, started + WHEEL_ZOOM_INTERVAL);
    assert_eq!(preview.zoom_percent(), 120);

    preview.zoom_from_wheel_at(false, area, pointer, started + WHEEL_ZOOM_INTERVAL * 2);
    assert_eq!(preview.zoom_percent(), 110);

    // There is no queued gesture work after input ends, and keyboard steps stay immediate.
    finish_viewport(&mut preview, preview_content(area));
    assert_eq!(preview.zoom_percent(), 110);
    preview.zoom_at(true, area, pointer);
    preview.zoom_at(true, area, pointer);
    assert_eq!(preview.zoom_percent(), 130);
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
    assert_eq!(preview.zoom_percent(), 110);
    assert_eq!(preview.horizontal_pixels, 4);
    assert_eq!(preview.scroll_pixels, 4);
    finish_viewport(&mut preview, content);
    assert_eq!(preview.scaled_page_size.unwrap().0, 110);

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
    assert_eq!(preview.scaled_page_size.unwrap().0, 110);
    assert_eq!(preview.displayed_horizontal, Some(4));
    assert_eq!(preview.displayed_scroll, Some(4));
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
    assert_eq!(preview.scaled_page_size.unwrap().0, 176);
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
    assert_eq!(preview.zoom_percent(), 80);
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
        assert!(viewport.view.source.x + viewport.view.source.width <= source.width());
        assert!(viewport.view.source.y + viewport.view.source.height <= source.height());
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
    use std::io::{Read, Seek, SeekFrom};

    const INTERACTIVE_FRAME_BUDGET: Duration = Duration::from_millis(50);
    let area = Rect::new(0, 0, 160, 60);
    let mut preview = Preview::default();
    preview.picker.set_protocol_type(ProtocolType::Kitty);
    preview.set_source(Some(patterned_page(1200, 1600)));
    finish_viewport(&mut preview, preview_content(area));
    let mut wire = tempfile::tempfile()?;
    let mut terminal = Terminal::with_options(
        CrosstermBackend::new(wire.try_clone()?),
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
