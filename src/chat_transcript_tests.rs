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

        let line = row.line(&messages[0], width, None);
        assert!(line.width() <= usize::from(width));
    }
}
