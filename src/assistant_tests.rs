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
        .draw(|frame| assistant.draw(frame, frame.area(), true, false, "Ctrl+x"))
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
        .draw(|frame| assistant.draw(frame, area, true, false, "Ctrl+x"))
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
            .draw(|frame| assistant.draw(frame, frame.area(), true, false, "Ctrl+x"))
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
fn reset_removes_document_specific_conversation_and_proposal() {
    let mut assistant = Assistant::new();
    assistant
        .messages
        .push(Message::new("user", "private details".into(), false));
    assistant.input.insert_str("draft prompt");
    assistant.proposed_edit = Some(ProposedEdit {
        source_before: "old".into(),
        source_after: "new".into(),
    });
    assistant.reset();
    assert!(assistant.messages.is_empty());
    assert!(assistant.proposal().is_none());
    assert_eq!(assistant.input.lines(), &[""]);
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
