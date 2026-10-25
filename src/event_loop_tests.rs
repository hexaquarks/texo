use std::collections::VecDeque;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::*;

fn key(character: char) -> Event {
    Event::Key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE))
}

#[test]
fn queued_input_is_handled_in_order_before_the_next_draw() {
    let first = key('a');
    let mut remaining = VecDeque::from([key('b'), key('c'), key('d')]);
    let mut handled = Vec::new();
    let quit = drain_ready_events(
        first,
        || Ok(remaining.pop_front()),
        |event| {
            handled.push(event);
            false
        },
    )
    .unwrap();

    assert!(!quit);
    assert_eq!(handled, [key('a'), key('b'), key('c'), key('d')]);
    assert!(remaining.is_empty());
}

#[test]
fn a_continuous_stream_yields_without_dropping_the_next_event() {
    let first = key('a');
    let mut remaining = VecDeque::from(vec![key('b'); MAX_INPUT_EVENTS_PER_FRAME]);
    let mut count = 0;
    drain_ready_events(
        first,
        || Ok(remaining.pop_front()),
        |_| {
            count += 1;
            false
        },
    )
    .unwrap();

    assert_eq!(count, MAX_INPUT_EVENTS_PER_FRAME);
    assert_eq!(remaining, [key('b')]);
}

#[test]
fn quit_and_resize_stop_before_consuming_another_event() {
    for (first, quit) in [(key('q'), true), (Event::Resize(120, 40), false)] {
        let mut remaining = VecDeque::from([key('a')]);
        let mut handled = Vec::new();
        let result = drain_ready_events(
            first,
            || Ok(remaining.pop_front()),
            |event| {
                handled.push(event);
                quit
            },
        )
        .unwrap();

        assert_eq!(result, quit);
        assert_eq!(handled.len(), 1);
        assert_eq!(remaining, [key('a')]);
    }
}

#[test]
fn an_input_failure_is_propagated() {
    let result = drain_ready_events(
        key('a'),
        || Err(io::Error::other("input failed")),
        |_| false,
    );
    assert_eq!(result.unwrap_err().to_string(), "input failed");
}

#[test]
fn image_completion_is_polled_quickly_without_busy_polling_while_idle() {
    assert_eq!(poll_interval(true, false), IMAGE_POLL_INTERVAL);
    assert_eq!(poll_interval(true, true), IMAGE_POLL_INTERVAL);
    assert_eq!(poll_interval(false, true), EVENT_POLL_INTERVAL);
    assert_eq!(poll_interval(false, false), IDLE_POLL_INTERVAL);
}

#[test]
#[ignore = "requires Poppler"]
fn shutdown_releases_current_proposed_and_cached_resume_images() -> anyhow::Result<()> {
    use crate::app::{resume_session::ResumeSession, test_support::application};
    use crate::editor::Editor;
    use ratatui::{Terminal, backend::TestBackend};
    use ratatui_image::picker::{Picker, ProtocolType};
    use std::{path::Path, sync::atomic::AtomicBool, time::Instant};
    use texo::{pdf::PdfPages, preview::Preview};

    const PREVIEW_TIMEOUT: Duration = Duration::from_secs(5);
    const PREVIEW_POLL_INTERVAL: Duration = Duration::from_millis(5);
    let (_directory, mut app) = application()?;
    let mut picker = Picker::halfblocks();
    picker.set_protocol_type(ProtocolType::Kitty);
    app.session.preview = Preview::with_picker(picker);
    app.session.proposed_preview = app.session.preview.empty_like();
    let cached_path = app.session.editor.path.with_file_name("cached.tex");
    let cached_editor = Editor::placeholder(cached_path.clone())?;
    app.sessions.insert(
        cached_path.clone(),
        ResumeSession::new(cached_editor, app.session.preview.empty_like()),
    );
    let mut terminal = Terminal::new(TestBackend::new(40, 20))?;

    for preview in [
        &mut app.session.preview,
        &mut app.session.proposed_preview,
        &mut app.sessions.get_mut(&cached_path).unwrap().preview,
    ] {
        preview.set_pages(PdfPages::render(
            Path::new("tests/fixtures/two-pages.pdf"),
            &AtomicBool::new(false),
        )?)?;
        let deadline = Instant::now() + PREVIEW_TIMEOUT;
        loop {
            terminal.draw(|frame| preview.draw(frame, frame.area()))?;
            if !preview.is_preparing() {
                break;
            }
            anyhow::ensure!(Instant::now() < deadline, "Preview did not become ready");
            std::thread::sleep(PREVIEW_POLL_INTERVAL);
        }
        assert!(preview.has_presented_image());
    }

    let mut wire = Vec::new();
    app.release_terminal_images(&mut wire)?;
    let commands = String::from_utf8(wire)?;
    assert_eq!(commands.matches("d=I").count(), 3);
    assert!(app.sessions.is_empty());
    assert!(!app.session.preview.has_presented_image());
    assert!(!app.session.proposed_preview.has_presented_image());

    let mut wire = Vec::new();
    app.release_terminal_images(&mut wire)?;
    assert!(wire.is_empty());
    Ok(())
}
