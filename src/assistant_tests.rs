use super::chat_view::MAX_INPUT_ROWS;
use super::*;

#[test]
fn review_confirmation_is_appended_and_scrolls_into_view() {
    let mut assistant = Assistant::new();
    assistant.notice("Earlier summary".into(), false);
    assistant.notice("Build failed".into(), true);
    assistant.scroll_from_bottom = SCROLL_LINES;

    assistant.report_edit_result("Proposal discarded; no changes accepted.");

    assert_eq!(assistant.messages.len(), 3);
    assert_eq!(assistant.messages[0].content, "Earlier summary");
    assert_eq!(assistant.messages[1].content, "Build failed");
    assert_eq!(
        assistant.messages[2].content,
        "Proposal discarded; no changes accepted."
    );
    assert!(!assistant.messages[2].error);
    assert_eq!(assistant.scroll_from_bottom, 0);
}

#[test]
fn slash_commands_complete_dynamically_and_never_enter_model_history() {
    let mut assistant = Assistant::new();
    assistant.input.insert_str("/lo");
    assert_eq!(assistant.command_matches().len(), 2);
    assert!(assistant.complete_command(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)));
    assert_eq!(assistant.input.lines()[0], "/login");
    assistant
        .submit("resume".into(), Path::new("resume.tex"))
        .unwrap();
    assert!(assistant.login_open());
    assert!(!assistant.is_running());
    assert!(assistant.messages.is_empty());
    assistant.handle_login_event(
        Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        Rect::new(0, 0, 80, 20),
    );
    assert!(!assistant.login_open());
    assistant.input.insert_str("/unknown accidental-secret");
    assistant
        .submit("resume".into(), Path::new("resume.tex"))
        .unwrap();
    assert!(!assistant.is_running());
    assert!(
        !assistant
            .messages
            .last()
            .unwrap()
            .content
            .contains("accidental-secret")
    );
    assert!(assistant.messages.last().unwrap().error);
}

#[test]
fn switching_providers_clears_old_conversation_without_changing_the_document() {
    let mut assistant = Assistant::new();
    assistant.messages.push(Message::new(
        "user",
        "Earlier private conversation".into(),
        false,
    ));
    assistant.connected(Profile::new(Provider::Anthropic));
    assert_eq!(assistant.messages.len(), 1);
    assert!(assistant.messages[0].content.contains("Anthropic"));
    assert!(!assistant.messages[0].content.contains("Earlier private"));
    assistant.input.insert_str("/logi");
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(90, 20)).unwrap();
    terminal
        .draw(|frame| assistant.draw(frame, frame.area(), true, false, "Ctrl+x", false))
        .unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("/login"));
    assert!(!text.contains("/logout"));
}

#[test]
fn model_command_completes_and_current_model_renders_below_input() {
    let mut assistant = Assistant::new();
    assistant.profile = Profile::new(Provider::OpenAi);
    assistant.profile.model = "gpt-selected".into();
    assistant.input.insert_str("/mo");
    assert!(assistant.complete_command(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)));
    assert_eq!(assistant.input.lines()[0], "/model");
    assistant.input = new_input();
    let area = Rect::new(0, 0, 100, 20);
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 20)).unwrap();
    terminal
        .draw(|frame| assistant.draw(frame, area, true, false, "Ctrl+x", false))
        .unwrap();
    let input = assistant.input_area(area);
    let buffer = terminal.backend().buffer();
    let footer: String = (input.x..area.right())
        .map(|x| buffer[(x, input.bottom())].symbol())
        .collect();
    assert!(footer.contains("gpt-selected"));
    assert_eq!(buffer[(input.x, input.bottom())].bg, theme::BACKGROUND);
    assert_eq!(input.x, area.x + theme::PANEL_PADDING);
    assert!(!assistant.click(area, input.x + 1, input.bottom()));
    for width in [8, 15, 30] {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, 10)).unwrap();
        terminal
            .draw(|frame| assistant.draw(frame, frame.area(), true, false, "Ctrl+x", false))
            .unwrap();
    }
}

#[test]
fn transcript_uses_prompt_markers_and_compact_plain_responses() {
    use ratatui::{Terminal, backend::TestBackend};
    let mut assistant = Assistant::new();
    assistant
        .messages
        .push(Message::new("user", "Shorten my bullets.".into(), false));
    assistant.messages.push(Message::new(
        "assistant",
        "Tighter wording.\nFacts preserved.".into(),
        false,
    ));
    let mut terminal = Terminal::new(TestBackend::new(60, 6)).unwrap();
    terminal
        .draw(|frame| assistant.draw_history(frame, frame.area()))
        .unwrap();
    let buffer = terminal.backend().buffer();
    let rendered: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
    assert!(rendered.contains("> Shorten my bullets."));
    assert!(rendered.contains("* Tighter wording."));
    assert!(rendered.contains("Facts preserved."));
    assert_eq!(buffer[(59, 0)].bg, theme::ELEVATED);
    assert_eq!(buffer[(59, 3)].bg, theme::BACKGROUND);
    assert!(!rendered.contains("◉"));
}

#[test]
fn click_moves_cursor_inside_composer_and_ignores_history() {
    let mut assistant = Assistant::new();
    assistant.input.insert_str("hello world");
    let area = Rect::new(20, 10, 70, 15);
    let input = assistant.input_area(area);
    assert!(assistant.click(area, input.x + 5, input.y + 1));
    assert_eq!(assistant.input.cursor(), (0, 2));
    assert!(!assistant.click(area, input.x + 5, input.y));
    assert!(!assistant.click(area, area.x + 2, area.y + 2));
    assert_eq!(assistant.input.cursor(), (0, 2));
}

#[test]
fn submit_empty_message_does_not_launch_cli() {
    let mut assistant = Assistant::new();
    assert!(
        assistant
            .submit("source".into(), Path::new("resume.tex"))
            .is_err()
    );
    assert!(!assistant.is_running());
}

fn composer_content(assistant: &Assistant, area: Rect) -> Rect {
    assistant
        .input
        .block()
        .unwrap()
        .inner(assistant.input_area(area))
}

#[test]
fn chat_header_shades_the_configured_shortcut_without_state_labels_or_send_button() {
    use ratatui::{Terminal, backend::TestBackend};

    let mut assistant = Assistant::new();
    let mut terminal = Terminal::new(TestBackend::new(70, 20)).unwrap();
    terminal
        .draw(|frame| assistant.draw(frame, frame.area(), true, false, "Ctrl+a", false))
        .unwrap();
    let buffer = terminal.backend().buffer();
    let header: String = (0..70).map(|column| buffer[(column, 1)].symbol()).collect();
    let header = header.split_whitespace().collect::<Vec<_>>().join(" ");

    assert!(header.contains("Ctrl+a u Maximize"));
    assert!(!header.contains("Split"));
    assert!(!header.contains("Maximized"));

    let key = (0..70)
        .find(|column| buffer[(*column, 1)].symbol() == "C")
        .unwrap();
    let description = (0..70)
        .find(|column| buffer[(*column, 1)].symbol() == "M")
        .unwrap();
    assert_eq!(buffer[(key, 1)].bg, theme::KEYCAP);
    assert_eq!(buffer[(description, 1)].bg, theme::BACKGROUND);
}

fn composer_text(
    assistant: &Assistant,
    terminal: &ratatui::Terminal<ratatui::backend::TestBackend>,
    area: Rect,
) -> String {
    let content = composer_content(assistant, area);
    let mut text = String::new();

    for row in content.y..content.bottom() {
        for column in content.x..content.right() {
            text.push_str(terminal.backend().buffer()[(column, row)].symbol());
        }
    }

    text
}

#[test]
fn composer_wraps_grows_and_shrinks_without_changing_the_prompt_or_undo() {
    use ratatui::{Terminal, backend::TestBackend};

    let area = Rect::new(0, 0, 70, 20);
    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
    let mut assistant = Assistant::new();
    terminal
        .draw(|frame| assistant.draw(frame, area, true, false, "Ctrl+x", false))
        .unwrap();
    let empty_height = assistant.input_area(area).height;
    let prompt = "Please improve my backend experience while preserving all dates, technologies, and achievements.";

    assistant.input.insert_str(prompt);
    terminal
        .draw(|frame| assistant.draw(frame, area, true, false, "Ctrl+x", false))
        .unwrap();

    assert!(assistant.input_area(area).height > empty_height);
    assert_eq!(assistant.input.lines(), &[prompt]);
    assert_eq!(assistant.input.scroll_offset().1, 0);
    let rendered = composer_text(&assistant, &terminal, area);
    let non_whitespace = |text: &str| {
        text.chars()
            .filter(|character| !character.is_whitespace())
            .collect::<String>()
    };
    assert_eq!(non_whitespace(&rendered), non_whitespace(prompt));

    let input = assistant.input_area(area);
    assert_eq!(input.right(), area.right() - theme::PANEL_PADDING);
    let all_text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(!all_text.contains("[send]"));

    assistant.input.undo();
    terminal
        .draw(|frame| assistant.draw(frame, area, true, false, "Ctrl+x", false))
        .unwrap();
    assert_eq!(assistant.input.lines(), &[""]);
    assert_eq!(assistant.input_area(area).height, empty_height);
}

#[test]
fn composer_bounds_long_pastes_and_keeps_the_cursor_visible_after_resize() {
    use ratatui::{Terminal, backend::TestBackend};

    let mut assistant = Assistant::new();
    let prompt = format!("START {} END", "unbrokenword".repeat(1000));
    assistant.input.insert_str(&prompt);
    let area = Rect::new(0, 0, 100, 30);
    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
    terminal
        .draw(|frame| assistant.draw(frame, area, true, false, "Ctrl+x", false))
        .unwrap();

    assert_eq!(composer_content(&assistant, area).height, MAX_INPUT_ROWS);
    assert!(assistant.input.scroll_offset().0 > 0);
    assert_eq!(assistant.input.scroll_offset().1, 0);
    assert!(composer_text(&assistant, &terminal, area).contains("END"));

    for (width, height) in [(30, 12), (15, 10), (8, 10), (1, 20), (4, 10), (100, 30)] {
        let area = Rect::new(0, 0, width, height);
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| assistant.draw(frame, area, true, false, "Ctrl+x", false))
            .unwrap();
        assert_eq!(assistant.input.lines(), &[prompt.as_str()]);
        assert_eq!(assistant.input.scroll_offset().1, 0);
    }
}

#[test]
fn wrapped_composer_clicks_and_navigation_follow_visual_rows() {
    use ratatui::{Terminal, backend::TestBackend};
    use ratatui_textarea::CursorMove;

    let area = Rect::new(0, 0, 70, 20);
    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
    let mut assistant = Assistant::new();
    assistant.input.insert_str("界".repeat(60));
    terminal
        .draw(|frame| assistant.draw(frame, area, true, false, "Ctrl+x", false))
        .unwrap();
    let content = composer_content(&assistant, area);
    let second_row_character = usize::from(content.width / 2) + 1;

    assert!(assistant.click(area, content.x + 2, content.y + 1));
    assert_eq!(assistant.input.cursor(), (0, second_row_character));
    assistant
        .input
        .input(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(assistant.input.cursor(), (0, 1));
    assistant
        .input
        .input(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(assistant.input.cursor(), (0, second_row_character));

    assistant.input.move_cursor(CursorMove::Top);
    assistant.input.select_all();
    assistant.input.insert_str("🙂e\u{301}".repeat(30));
    terminal
        .draw(|frame| assistant.draw(frame, area, true, false, "Ctrl+x", false))
        .unwrap();
    let symbols: Vec<_> = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(symbols.contains(&"🙂"));
    assert!(symbols.contains(&"e\u{301}"));
    assert!(!symbols.contains(&"\u{301}"));
}

#[test]
fn composer_preserves_explicit_newlines_and_shrinks_after_clear() {
    use ratatui::{Terminal, backend::TestBackend};

    let area = Rect::new(0, 0, 70, 20);
    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
    let mut assistant = Assistant::new();
    assistant
        .input
        .insert_str("First line\nSecond line\n\nFourth line");
    terminal
        .draw(|frame| assistant.draw(frame, area, true, false, "Ctrl+x", false))
        .unwrap();
    assert_eq!(composer_content(&assistant, area).height, 4);
    assert_eq!(
        assistant.input.lines(),
        &["First line", "Second line", "", "Fourth line"]
    );

    assistant.clear_input();
    terminal
        .draw(|frame| assistant.draw(frame, area, true, false, "Ctrl+x", false))
        .unwrap();
    assert_eq!(composer_content(&assistant, area).height, 1);
    assert_eq!(
        assistant.input.wrap_mode(),
        ratatui_textarea::WrapMode::WordOrGlyph
    );
}

#[test]
fn maximize_hitbox_matches_the_rendered_shortcut_and_action() {
    use ratatui::{Terminal, backend::TestBackend};

    for width in [40, 70] {
        for prefix in ["Ctrl+x", "Alt+a"] {
            for maximized in [false, true] {
                let mut assistant = Assistant::new();
                let mut terminal = Terminal::new(TestBackend::new(width, 20)).unwrap();
                let area = Rect::new(0, 0, width, 20);
                terminal
                    .draw(|frame| assistant.draw(frame, area, true, maximized, prefix, false))
                    .unwrap();

                let button = Assistant::maximize_button(area, maximized, prefix);
                let buffer = terminal.backend().buffer();
                let action = if maximized { "Restore" } else { "Maximize" };
                let hint = theme::shortcuts(&[(&format!("{prefix} u"), action)]);
                let rendered: String = (button.x..button.right())
                    .map(|x| buffer[(x, button.y)].symbol())
                    .collect();

                assert_eq!(rendered, hint.to_string());
                assert_eq!(buffer[(button.x, button.y)].bg, theme::KEYCAP);
                assert_eq!(buffer[(button.x - 1, button.y)].symbol(), " ");
                assert_eq!(buffer[(button.right(), button.y)].symbol(), " ");
            }
        }
    }
    assert!(Assistant::maximize_button(Rect::new(0, 0, 40, 2), false, "Ctrl+x").is_empty());
}
