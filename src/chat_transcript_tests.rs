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
    assert!(assistant.drag_transcript_selection(mouse(
        MouseEventKind::Drag(MouseButton::Left),
        1,
        0
    )));
    assert!(assistant.drag_transcript_selection(mouse(
        MouseEventKind::Up(MouseButton::Left),
        1,
        0
    )));
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
    assert!(assistant.drag_transcript_selection(mouse(
        MouseEventKind::Up(MouseButton::Left),
        50,
        5
    )));

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
