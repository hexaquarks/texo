use ratatui::{Terminal, backend::TestBackend, crossterm::event::KeyModifiers};

use super::*;
use crate::assistant::Assistant;

fn mouse(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }
}

fn draw(assistant: &mut Assistant, terminal: &mut Terminal<TestBackend>) {
    terminal
        .draw(|frame| assistant.draw_history(frame, frame.area()))
        .unwrap();
}

#[test]
fn reverse_selection_copies_original_wrapped_unicode_reply() {
    let mut assistant = Assistant::new();
    assistant.messages.push(Message::new(
        "assistant",
        "repeat   repeat\n李e\u{301} done".into(),
        false,
    ));
    let mut terminal = Terminal::new(TestBackend::new(13, 5)).unwrap();
    draw(&mut assistant, &mut terminal);

    // Start after the wide glyph and combining cluster, then drag to the reply marker.
    assistant.begin_transcript_selection(Position::new(6, 2), false);
    assert!(matches!(
        assistant.drag_transcript_selection(mouse(MouseEventKind::Drag(MouseButton::Left), 1, 0)),
        TranscriptGesture::Selection
    ));
    assert!(matches!(
        assistant.drag_transcript_selection(mouse(MouseEventKind::Up(MouseButton::Left), 1, 0)),
        TranscriptGesture::Selection
    ));
    draw(&mut assistant, &mut terminal);

    let text = assistant.transcript.selected_text(&assistant.messages);
    assert_eq!(text.as_deref(), Some("repeat   repeat\n李e\u{301}"));

    let buffer = terminal.backend().buffer();
    assert!(buffer[(3, 1)].modifier.contains(Modifier::REVERSED));
    assert!(buffer[(5, 2)].modifier.contains(Modifier::REVERSED));
    assert!(!buffer[(1, 0)].modifier.contains(Modifier::REVERSED));
    assert!(!buffer[(7, 2)].modifier.contains(Modifier::REVERSED));
}

#[test]
fn selection_reaches_offscreen_messages_and_survives_reflow() {
    let mut assistant = Assistant::new();
    let reply = "One\nTwo\nThree\nFour\nFive\nSix\nSeven\nEight";
    assistant
        .messages
        .push(Message::new("user", "Prompt".into(), false));
    assistant
        .messages
        .push(Message::new("assistant", reply.into(), false));

    let mut terminal = Terminal::new(TestBackend::new(30, 3)).unwrap();
    for _ in 0..3 {
        assistant.scroll(true);
    }
    draw(&mut assistant, &mut terminal);
    assistant.begin_transcript_selection(Position::new(3, 1), false);

    for _ in 0..3 {
        assistant.scroll(false);
    }
    draw(&mut assistant, &mut terminal);
    assert!(matches!(
        assistant.drag_transcript_selection(mouse(MouseEventKind::Up(MouseButton::Left), 50, 5)),
        TranscriptGesture::Selection
    ));

    let mut narrower = Terminal::new(TestBackend::new(9, 4)).unwrap();
    draw(&mut assistant, &mut narrower);

    let expected = format!("Prompt\n\n{reply}");
    let text = assistant.transcript.selected_text(&assistant.messages);
    assert_eq!(text.as_deref(), Some(expected.as_str()));
    assert!(
        narrower.backend().buffer()[(3, 3)]
            .modifier
            .contains(Modifier::REVERSED)
    );

    assistant.begin_transcript_selection(Position::new(8, 2), true);
    let shortened = expected.strip_suffix("\nEight").unwrap();
    let text = assistant.transcript.selected_text(&assistant.messages);
    assert_eq!(text.as_deref(), Some(shortened));

    assistant.transcript.reset();
    let text = assistant.transcript.selected_text(&assistant.messages);
    assert!(text.is_none());
}

#[test]
fn long_words_wrap_at_grapheme_boundaries() {
    let cluster = "👩‍💻";
    let reply = format!("a{cluster}b{cluster}c");
    let messages = [Message::new("assistant", reply.clone(), false)];
    let width = 8;
    let rows = transcript_rows(&messages, width, false);
    let boundaries: Vec<_> = reply.grapheme_indices(true).map(|(byte, _)| byte).collect();

    assert!(rows.len() > 1);
    for row in rows {
        assert!(boundaries.contains(&row.bytes.start));
        assert!(row.bytes.end == reply.len() || boundaries.contains(&row.bytes.end));

        let columns: usize = reply[row.bytes].graphemes(true).map(grapheme_width).sum();
        assert!(columns <= usize::from(width) - CONTINUATION.len() - usize::from(RIGHT_PADDING));
    }
}

#[test]
fn a_wrap_never_detaches_a_combining_mark_from_its_base() {
    let mut assistant = Assistant::new();
    assistant
        .messages
        .push(Message::new("assistant", "a \u{301}b".into(), false));
    let mut terminal = Terminal::new(TestBackend::new(6, 3)).unwrap();
    draw(&mut assistant, &mut terminal);

    assert_eq!(terminal.backend().buffer()[(3, 1)].symbol(), " \u{301}");

    assistant.begin_transcript_selection(Position::new(3, 1), false);
    assistant.drag_transcript_selection(mouse(MouseEventKind::Up(MouseButton::Left), 4, 1));
    let copied = assistant.transcript.selected_text(&assistant.messages);
    assert_eq!(copied.as_deref(), Some(" \u{301}"));
}

#[test]
fn copy_highlight_keeps_cell_positions_after_ligatures_and_tabs() {
    let mut assistant = Assistant::new();
    assistant
        .messages
        .push(Message::new("assistant", "لا\tend".into(), false));
    let mut terminal = Terminal::new(TestBackend::new(15, 3)).unwrap();
    draw(&mut assistant, &mut terminal);
    assert_eq!(terminal.backend().buffer()[(5, 0)].symbol(), "e");

    assistant.begin_transcript_selection(Position::new(5, 0), false);
    assistant.drag_transcript_selection(mouse(MouseEventKind::Up(MouseButton::Left), 8, 0));
    draw(&mut assistant, &mut terminal);

    let buffer = terminal.backend().buffer();
    assert_eq!(buffer[(5, 0)].symbol(), "e");
    assert!(buffer[(5, 0)].modifier.contains(Modifier::REVERSED));
    assert_eq!(buffer[(7, 0)].symbol(), "d");
    assert!(buffer[(7, 0)].modifier.contains(Modifier::REVERSED));

    let copied = assistant.transcript.selected_text(&assistant.messages);
    assert_eq!(copied.as_deref(), Some("end"));
}

#[test]
fn scroll_reaches_start_of_a_transcript_with_more_than_u16_rows() {
    let mut assistant = Assistant::new();
    let viewport_rows = 3;
    let blank_rows = usize::from(u16::MAX) + usize::from(viewport_rows);
    let reply = format!("First row\n{}Last row", "\n".repeat(blank_rows));
    assistant
        .messages
        .push(Message::new("assistant", reply, false));
    let mut terminal = Terminal::new(TestBackend::new(20, viewport_rows)).unwrap();

    draw(&mut assistant, &mut terminal);
    assert_eq!(terminal.backend().buffer()[(3, 2)].symbol(), "L");

    assistant.scroll_from_bottom = usize::MAX;
    draw(&mut assistant, &mut terminal);
    assert_eq!(terminal.backend().buffer()[(3, 0)].symbol(), "F");
}

#[cfg(unix)]
#[test]
fn unavailable_native_clipboard_sends_osc52_and_reports_terminal_write_failure() {
    struct UnavailableClipboard;

    impl ClipboardAccess for UnavailableClipboard {
        fn read(&mut self) -> anyhow::Result<String> {
            anyhow::bail!("Clipboard unavailable")
        }

        fn write(&mut self, _text: String) -> anyhow::Result<()> {
            anyhow::bail!("Clipboard unavailable")
        }
    }

    let mut terminal_output = Vec::new();
    let notice = copy_text(
        &mut UnavailableClipboard,
        &mut terminal_output,
        "Texo".into(),
    );

    assert_eq!(notice, "Sent to terminal clipboard");
    assert_eq!(terminal_output, b"\x1b]52;c;VGV4bw==\x1b\\");

    let read_only_terminal = io::Cursor::new(&mut [][..]);
    let notice = copy_text(&mut UnavailableClipboard, read_only_terminal, "Texo".into());
    assert_eq!(notice, "Could not copy chat selection");
}

#[test]
fn wrapped_link_clicks_open_a_url_but_dragging_copies_displayed_text() {
    let url = "https://example.com/licenses/mit";
    let raw = format!("[**MIT** license documentation]({url})");
    let mut assistant = Assistant::new();
    assistant.messages.push(Message::reply(raw.clone()));
    let mut terminal = Terminal::new(TestBackend::new(23, 8)).unwrap();
    draw(&mut assistant, &mut terminal);

    let first = Position::new(3, 0);
    let continuation = Position::new(3, 1);
    let last_row = u16::try_from(assistant.transcript.visible_rows.len() - 1).unwrap();
    let past_end = Position::new(22, last_row);
    let buffer = terminal.backend().buffer();

    assert_eq!(buffer[(3, 1)].symbol(), "d");
    assert!(buffer[first].modifier.contains(Modifier::BOLD));
    assert!(buffer[continuation].modifier.contains(Modifier::UNDERLINED));

    for position in [first, continuation] {
        assistant.begin_transcript_selection(position, false);
        let release = mouse(
            MouseEventKind::Up(MouseButton::Left),
            position.x,
            position.y,
        );
        let TranscriptGesture::OpenLink(opened) = assistant.drag_transcript_selection(release)
        else {
            panic!("Stationary click on the link should open its destination");
        };
        assert_eq!(opened.as_str(), url);
        assert!(
            assistant
                .transcript
                .selected_text(&assistant.messages)
                .is_none()
        );
    }

    // Cross back over the starting cell, then drag past the wrapped URL.
    assistant.begin_transcript_selection(first, false);
    for point in [continuation, first, past_end] {
        let drag = mouse(MouseEventKind::Drag(MouseButton::Left), point.x, point.y);
        assert!(matches!(
            assistant.drag_transcript_selection(drag),
            TranscriptGesture::Selection
        ));
    }
    let release = mouse(
        MouseEventKind::Up(MouseButton::Left),
        past_end.x,
        past_end.y,
    );
    assert!(matches!(
        assistant.drag_transcript_selection(release),
        TranscriptGesture::Selection
    ));

    let expected = format!("MIT license documentation ({url})");
    let copied = assistant.transcript.selected_text(&assistant.messages);
    assert_eq!(copied.as_deref(), Some(expected.as_str()));
    assert_eq!(assistant.messages[0].content, raw);

    let mut wider = Terminal::new(TestBackend::new(80, 8)).unwrap();
    draw(&mut assistant, &mut wider);
    let selected = wider.backend().buffer()[first].modifier;
    assert!(selected.contains(Modifier::BOLD | Modifier::UNDERLINED | Modifier::REVERSED));
}

#[test]
fn mouse_gestures_over_padding_and_unsafe_links_do_not_open_anything() {
    let mut assistant = Assistant::new();
    assistant.messages.push(Message::reply(
        "[MIT](https://example.com)\n[File](file:///tmp/example)".into(),
    ));
    let mut terminal = Terminal::new(TestBackend::new(50, 4)).unwrap();
    draw(&mut assistant, &mut terminal);

    for (position, extend) in [
        (Position::new(1, 0), false),  // Reply marker
        (Position::new(35, 0), false), // Padding after the link
        (Position::new(3, 0), true),   // Shift-click
        (Position::new(3, 1), false),  // File link
    ] {
        assistant.begin_transcript_selection(position, extend);
        let release = mouse(
            MouseEventKind::Up(MouseButton::Left),
            position.x,
            position.y,
        );
        assert!(matches!(
            assistant.drag_transcript_selection(release),
            TranscriptGesture::Selection
        ));
    }

    let link = Position::new(3, 0);
    let release = mouse(MouseEventKind::Up(MouseButton::Left), link.x, link.y);
    assistant.begin_transcript_selection(link, false);
    let scroll = mouse(MouseEventKind::ScrollUp, link.x, link.y);
    assert!(matches!(
        assistant.drag_transcript_selection(scroll),
        TranscriptGesture::Ignored
    ));
    assert!(matches!(
        assistant.drag_transcript_selection(release),
        TranscriptGesture::Selection
    ));

    assistant.begin_transcript_selection(link, false);
    for column in [4, 3] {
        let drag = mouse(MouseEventKind::Drag(MouseButton::Left), column, 0);
        assert!(matches!(
            assistant.drag_transcript_selection(drag),
            TranscriptGesture::Selection
        ));
    }
    assert!(matches!(
        assistant.drag_transcript_selection(mouse(MouseEventKind::Up(MouseButton::Left), 3, 0)),
        TranscriptGesture::Selection
    ));
}

#[test]
fn styled_unicode_wraps_as_whole_graphemes_and_remains_copyable() {
    let raw = "a ***e***&#769; 👩‍💻 字 a \u{301}b";
    let messages = [Message::reply(raw.into())];
    let content = messages[0].display_text();
    assert_eq!(content, "a e\u{301} 👩‍💻 字 a \u{301}b");

    let mut boundaries: Vec<_> = content
        .grapheme_indices(true)
        .map(|(byte, _)| byte)
        .collect();
    boundaries.push(content.len());

    for width in 5..20 {
        let rows = transcript_rows(&messages, width, false);
        for row in rows {
            assert!(boundaries.contains(&row.bytes.start));
            assert!(boundaries.contains(&row.bytes.end));
        }
    }

    let mut assistant = Assistant::new();
    assistant.messages.extend(messages);
    let mut terminal = Terminal::new(TestBackend::new(48, 5)).unwrap();
    draw(&mut assistant, &mut terminal);

    let accented = &terminal.backend().buffer()[(5, 0)];
    assert_eq!(accented.symbol(), "e\u{301}");
    assert!(
        accented
            .modifier
            .contains(Modifier::BOLD | Modifier::ITALIC)
    );

    assistant.begin_transcript_selection(Position::new(5, 0), false);
    let release = mouse(MouseEventKind::Up(MouseButton::Left), 6, 0);
    assert!(matches!(
        assistant.drag_transcript_selection(release),
        TranscriptGesture::Selection
    ));

    let copied = assistant.transcript.selected_text(&assistant.messages);
    assert_eq!(copied.as_deref(), Some("e\u{301}"));
}
