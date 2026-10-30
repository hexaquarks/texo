use super::test_support::application;
use super::workspace_view::below_header;
use super::*;

use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    sync::atomic::AtomicBool,
};

use ratatui::{
    Terminal, TerminalOptions, Viewport,
    backend::CrosstermBackend,
    buffer::Buffer,
    crossterm::terminal::{Clear, ClearType},
    layout::{Margin, Position},
};
use ratatui_image::picker::{Picker, ProtocolType};

const PREVIEW_TIMEOUT: Duration = Duration::from_secs(5);
const PREVIEW_POLL_INTERVAL: Duration = Duration::from_millis(5);
// Kitty's Unicode placeholder character; ratatui-image keeps this constant private.
const KITTY_IMAGE_PLACEHOLDER: char = '\u{10eeee}';

type WireTerminal = Terminal<CrosstermBackend<File>>;

fn draw(app: &mut App, terminal: &mut WireTerminal) -> anyhow::Result<Buffer> {
    Ok(terminal.draw(|frame| app.draw(frame))?.buffer.clone())
}

fn ready_preview(app: &mut App, terminal: &mut WireTerminal) -> anyhow::Result<Buffer> {
    let deadline = std::time::Instant::now() + PREVIEW_TIMEOUT;
    loop {
        let screen = draw(app, terminal)?;
        let has_image = screen
            .content
            .iter()
            .any(|cell| cell.symbol().contains(KITTY_IMAGE_PLACEHOLDER) || cell.symbol() == "▀");
        if !app.session.preview.is_preparing() && has_image {
            // The first frame uploads Kitty data; subsequent frames contain only placements.
            return draw(app, terminal);
        }

        anyhow::ensure!(
            std::time::Instant::now() < deadline,
            "Preview did not become ready"
        );
        std::thread::sleep(PREVIEW_POLL_INTERVAL);
    }
}

fn image_area(app: &App, area: Rect) -> Rect {
    below_header(pane_content(app.preview_area(area)), 1)
        .inner(Margin::new(theme::PANEL_PADDING, 1))
}

fn assert_image_matches(before: &Buffer, after: &Buffer, image: Rect, covered: Rect) {
    for y in image.y..image.bottom() {
        for x in image.x..image.right() {
            if !covered.contains(Position::new(x, y)) {
                assert_eq!(
                    before[(x, y)],
                    after[(x, y)],
                    "PDF changed outside the dialog at {x},{y}"
                );
            }
        }
    }
}

fn wire_since(wire: &mut File, start: u64) -> anyhow::Result<String> {
    wire.seek(SeekFrom::Start(start))?;
    let mut output = String::new();
    wire.read_to_string(&mut output)?;
    Ok(output)
}

fn set_settings_open(app: &mut App, area: Rect, open: bool) {
    let was_open = app.overlay_open();
    let code = if open {
        KeyCode::Char('?')
    } else {
        KeyCode::Esc
    };
    app.handle_event(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)), area);

    assert_eq!(app.settings.is_some(), open);
    assert!(
        !app.overlays_require_clear(was_open),
        "Settings must not clear a Kitty/basic terminal"
    );
}

#[test]
fn only_inline_graphics_need_a_clear_when_an_overlay_closes() -> anyhow::Result<()> {
    for (protocol, needs_clear) in [
        (ProtocolType::Kitty, false),
        (ProtocolType::Halfblocks, false),
        (ProtocolType::Sixel, true),
        (ProtocolType::Iterm2, true),
    ] {
        let (_directory, mut app) = application()?;
        let mut picker = Picker::halfblocks();
        picker.set_protocol_type(protocol);
        app.session.preview = Preview::with_picker(picker);
        app.open_settings();
        assert!(!app.overlays_require_clear(false));

        app.settings = None;
        assert_eq!(app.overlays_require_clear(true), needs_clear);
    }
    Ok(())
}

#[test]
#[ignore = "requires Poppler"]
fn settings_preserve_visible_pdf_cells_and_do_not_emit_a_screen_clear() -> anyhow::Result<()> {
    for protocol in [ProtocolType::Kitty, ProtocolType::Halfblocks] {
        for area in [
            Rect::new(0, 0, 160, 50),
            Rect::new(0, 0, 160, 22),
            Rect::new(0, 0, 40, 16),
        ] {
            let (_directory, mut app) = application()?;
            let mut picker = Picker::halfblocks();
            picker.set_protocol_type(protocol);
            app.session.preview = Preview::with_picker(picker);
            app.session.preview.set_pages(texo::pdf::PdfPages::render(
                std::path::Path::new("tests/fixtures/two-pages.pdf"),
                &AtomicBool::new(false),
            )?)?;
            app.focus = Focus::Preview;

            let mut wire = tempfile::tempfile()?;
            let mut terminal = Terminal::with_options(
                CrosstermBackend::new(wire.try_clone()?),
                TerminalOptions {
                    viewport: Viewport::Fixed(area),
                },
            )?;
            let before = ready_preview(&mut app, &mut terminal)?;
            let start = wire.metadata()?.len();

            set_settings_open(&mut app, area, true);
            let during = draw(&mut app, &mut terminal)?;
            let image = image_area(&app, area);
            assert_image_matches(&before, &during, image, SettingsDialog::area(area));

            set_settings_open(&mut app, area, false);
            let after = draw(&mut app, &mut terminal)?;
            assert_image_matches(&before, &after, image, Rect::default());

            let output = wire_since(&mut wire, start)?;
            let screen_clear = Clear(ClearType::All).to_string();
            assert!(
                !output.contains(&screen_clear),
                "Opening or closing settings erased the screen"
            );
            assert!(
                !output.contains("a=T"),
                "Settings needlessly re-uploaded the cached PDF"
            );
        }
    }
    Ok(())
}

#[test]
#[ignore = "requires Poppler"]
fn a_new_page_waits_behind_settings_without_losing_its_first_upload() -> anyhow::Result<()> {
    let (_directory, mut app) = application()?;
    let mut picker = Picker::halfblocks();
    picker.set_protocol_type(ProtocolType::Kitty);
    app.session.preview = Preview::with_picker(picker);
    app.session.preview.set_pages(texo::pdf::PdfPages::render(
        std::path::Path::new("tests/fixtures/two-pages.pdf"),
        &AtomicBool::new(false),
    )?)?;
    app.focus = Focus::Preview;

    // At this height, settings covers the PDF's upload cell but leaves its right edge visible.
    let area = Rect::new(0, 0, 160, 22);
    let mut wire = tempfile::tempfile()?;
    let mut terminal = Terminal::with_options(
        CrosstermBackend::new(wire.try_clone()?),
        TerminalOptions {
            viewport: Viewport::Fixed(area),
        },
    )?;
    let before = ready_preview(&mut app, &mut terminal)?;
    let image = image_area(&app, area);
    let covered = SettingsDialog::area(area);
    assert!(covered.contains(image.as_position()));

    set_settings_open(&mut app, area, true);
    app.session.preview.next_page()?;
    let start = wire.metadata()?.len();
    let during = draw(&mut app, &mut terminal)?;
    assert!(app.session.preview.is_preparing());
    assert_image_matches(&before, &during, image, covered);
    assert!(!wire_since(&mut wire, start)?.contains("a=T"));

    set_settings_open(&mut app, area, false);
    let after = ready_preview(&mut app, &mut terminal)?;
    assert_ne!(before, after);
    assert!(wire_since(&mut wire, start)?.contains("a=T"));
    Ok(())
}

#[test]
#[ignore = "requires Poppler"]
fn a_completed_proposal_keeps_current_pdf_visible_until_settings_closes() -> anyhow::Result<()> {
    let (_directory, mut app) = application()?;
    let mut picker = Picker::halfblocks();
    picker.set_protocol_type(ProtocolType::Kitty);
    app.session.preview = Preview::with_picker(picker);
    app.session.preview.set_pages(texo::pdf::PdfPages::render(
        std::path::Path::new("tests/fixtures/two-pages.pdf"),
        &AtomicBool::new(false),
    )?)?;
    app.focus = Focus::Preview;

    let area = Rect::new(0, 0, 160, 22);
    let mut wire = tempfile::tempfile()?;
    let mut terminal = Terminal::with_options(
        CrosstermBackend::new(wire.try_clone()?),
        TerminalOptions {
            viewport: Viewport::Fixed(area),
        },
    )?;
    let before = ready_preview(&mut app, &mut terminal)?;
    let image = image_area(&app, area);
    set_settings_open(&mut app, area, true);

    app.session.proposal_review = Some(ProposalReview::new(&ProposedEdit {
        source_before: app.session.editor.source(),
        source_after: "Proposed document".into(),
    }));
    app.session.proposed_preview = app.session.preview.empty_like();
    app.session
        .proposed_preview
        .set_pages(texo::pdf::PdfPages::render(
            std::path::Path::new("tests/fixtures/two-pages.pdf"),
            &AtomicBool::new(false),
        )?)?;
    app.session.show_proposed = true;
    let start = wire.metadata()?.len();
    let during = draw(&mut app, &mut terminal)?;

    assert!(!app.session.proposed_preview.has_presented_image());
    assert_image_matches(&before, &during, image, SettingsDialog::area(area));
    assert!(!wire_since(&mut wire, start)?.contains("a=T"));

    set_settings_open(&mut app, area, false);
    let deadline = std::time::Instant::now() + PREVIEW_TIMEOUT;
    while app.session.proposed_preview.is_preparing() {
        draw(&mut app, &mut terminal)?;
        anyhow::ensure!(
            std::time::Instant::now() < deadline,
            "Proposed PDF did not become ready"
        );
        std::thread::sleep(PREVIEW_POLL_INTERVAL);
    }
    assert!(app.session.proposed_preview.has_presented_image());
    assert!(wire_since(&mut wire, start)?.contains("a=T"));
    Ok(())
}

#[test]
#[ignore = "requires Poppler"]
fn shortcuts_preserve_cached_pdf_pixels_without_uploading_again() -> anyhow::Result<()> {
    for protocol in [ProtocolType::Kitty, ProtocolType::Halfblocks] {
        let (_directory, mut app) = application()?;
        let area = Rect::new(0, 0, 160, 50);
        let mut picker = Picker::halfblocks();
        picker.set_protocol_type(protocol);
        app.session.preview = Preview::with_picker(picker);
        app.session.preview.set_pages(texo::pdf::PdfPages::render(
            std::path::Path::new("tests/fixtures/two-pages.pdf"),
            &AtomicBool::new(false),
        )?)?;
        app.focus = Focus::Preview;
        app.workspace_expansion = Some(Focus::Preview);

        let mut wire = tempfile::tempfile()?;
        let mut terminal = Terminal::with_options(
            CrosstermBackend::new(wire.try_clone()?),
            TerminalOptions {
                viewport: Viewport::Fixed(area),
            },
        )?;
        let before = ready_preview(&mut app, &mut terminal)?;
        let start = wire.metadata()?.len();
        let image = image_area(&app, area);

        app.handle_prefix_action(KeyCode::Char('?'), area);
        let during = draw(&mut app, &mut terminal)?;
        assert_image_matches(&before, &during, image, app.shortcut_modal_area(area));
        app.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            area,
        );
        let after = draw(&mut app, &mut terminal)?;
        assert_image_matches(&before, &after, image, Rect::default());
        assert!(app.workspace_expansion == Some(Focus::Preview));

        let output = wire_since(&mut wire, start)?;
        assert!(!output.contains(&Clear(ClearType::All).to_string()));
        assert!(!output.contains("a=T"), "Help must reuse the uploaded PDF");
    }
    Ok(())
}
