use super::*;
use ratatui::{Terminal, backend::TestBackend};

struct UnavailableClipboard;

impl ClipboardAccess for UnavailableClipboard {
    fn read(&mut self) -> Result<String> {
        anyhow::bail!("No system clipboard in this test")
    }

    fn write(&mut self, _text: String) -> Result<()> {
        anyhow::bail!("No system clipboard in this test")
    }
}

fn editor(source: &str, row: u16, column: u16) -> Result<Editor> {
    let mut editor = Editor::placeholder(PathBuf::from("unused.tex"))?;
    editor.replace_source(source);
    editor.apply_preferences(EditorPreferences {
        mode: EditingMode::Vim,
        ..EditorPreferences::default()
    });
    editor.text.move_cursor(CursorMove::Jump(row, column));
    editor.clipboard = Box::new(UnavailableClipboard);
    Ok(editor)
}

fn commands(editor: &mut Editor, commands: &str) {
    for character in commands.chars() {
        editor.handle_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
    }
}

fn control(editor: &mut Editor, character: char) {
    editor.handle_key(KeyEvent::new(
        KeyCode::Char(character),
        KeyModifiers::CONTROL,
    ));
}

#[test]
fn inner_and_around_objects_use_vim_ranges_in_the_existing_buffer() -> Result<()> {
    // These expected selections were checked against Vim with its default configuration.
    let cases = [
        ("李 résumé work\n", 0, 5, "iw", "résumé"),
        ("re\u{301}sume work\n", 0, 3, "iw", "re\u{301}sume"),
        ("some.foo-bar next\n", 0, 5, "iw", "foo"),
        ("some.foo-bar next\n", 0, 4, "iw", "."),
        ("some.foo-bar next\n", 0, 5, "iW", "some.foo-bar"),
        ("some.foo-bar next\n", 0, 5, "aW", "some.foo-bar "),
        ("one  two three\n", 0, 3, "iw", "  "),
        ("one  two three\n", 0, 3, "aw", "  two"),
        ("one  two\n", 0, 7, "aw", "  two"),
        ("one \n  two\n", 0, 3, "aw", " \n  two"),
        ("before {outer {inner} text} after\n", 0, 17, "i{", "inner"),
        (
            "before {outer {inner} text} after\n",
            0,
            17,
            "aB",
            "{inner}",
        ),
        ("before\n{\n  inside\n  }\n", 0, 0, "iB", "  inside\n"),
        ("before (inside) after\n", 0, 10, "ib", "inside"),
        ("before [inside] after\n", 0, 10, "a]", "[inside]"),
        ("before <inside> after\n", 0, 10, "i<", "inside"),
        ("before \"hello\"  after\n", 0, 9, "a\"", "\"hello\"  "),
        ("before \"hello\"\n", 0, 9, "a\"", " \"hello\""),
        ("before \"hello\" after\n", 0, 0, "i\"", "hello"),
        ("before 'hello' after\n", 0, 10, "i'", "hello"),
        ("before `hello` after\n", 0, 10, "i`", "hello"),
        (
            r#"before "say \"hello\"" after"#,
            0,
            12,
            "i\"",
            r#"say \"hello\""#,
        ),
        (r"before \{ literal {real}", 0, 1, "i{", "real"),
    ];

    for (source, row, column, object, expected) in cases {
        let mut editor = editor(source, row, column)?;
        commands(&mut editor, &format!("y{object}"));
        assert_eq!(
            editor.text.yank_text(),
            expected,
            "object {object}, source {source:?}"
        );
        assert_eq!(editor.source(), source);
        assert!(!editor.text.is_selecting());
    }
    Ok(())
}

#[test]
fn change_delete_and_visual_objects_preserve_unicode_highlighting_and_native_undo() -> Result<()> {
    let source = "\\section{Résumé 李 and a long description of experience}\n\n";
    let mut editor = editor(source, 0, 18)?;
    let mut terminal = Terminal::new(TestBackend::new(22, 10))?;
    terminal.draw(|frame| editor.draw(frame, frame.area(), true))?;
    commands(&mut editor, "ci{");
    assert_eq!(editor.source(), "\\section{}\n\n");
    assert!(editor.vim.as_ref().unwrap().mode == vim::Mode::Insert);
    editor.paste_text("Work 李");
    editor.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    terminal.draw(|frame| editor.draw(frame, frame.area(), true))?;
    assert!(
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .any(|cell| cell.fg == theme::SYNTAX_COMMAND)
    );
    commands(&mut editor, "uu");
    assert_eq!(editor.source(), source);

    editor.text.move_cursor(CursorMove::Jump(0, 11));
    commands(&mut editor, "viwy");
    assert_eq!(editor.text.yank_text(), "Résumé");
    editor.text.move_cursor(CursorMove::Jump(0, 11));
    commands(&mut editor, "diw");
    assert_eq!(
        editor.source(),
        "\\section{ 李 and a long description of experience}\n\n"
    );
    editor.apply_preferences(EditorPreferences::default());
    control(&mut editor, 'z');
    assert_eq!(editor.source(), source);
    Ok(())
}

#[test]
fn failed_and_cancelled_objects_cannot_insert_command_keys() -> Result<()> {
    let source = "Résumé without delimiters\n";
    let mut editor = editor(source, 0, 2)?;
    commands(&mut editor, "ci{");
    assert!(editor.vim.as_ref().unwrap().mode == vim::Mode::Normal);
    commands(&mut editor, "diq");
    assert_eq!(editor.source(), source);
    assert!(!editor.text.is_selecting());
    commands(&mut editor, "ci");
    editor.cancel_pending_commands();
    commands(&mut editor, "w");
    assert_eq!(editor.source(), source);
    commands(&mut editor, "vi");
    editor.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    commands(&mut editor, "w");
    assert_eq!(editor.source(), source);
    assert!(!editor.text.is_selecting());
    Ok(())
}

#[test]
fn jump_history_uses_ctrl_o_ctrl_i_and_tab_without_recording_ordinary_motions() -> Result<()> {
    let source = "first\nsecond\nthird\nfourth\nfifth\n";
    let mut editor = editor(source, 2, 2)?;
    commands(&mut editor, "j");
    control(&mut editor, 'o');
    assert_eq!(editor.text.cursor(), (3, 2));
    editor.text.move_cursor(CursorMove::Jump(2, 2));
    commands(&mut editor, "Ggg");
    assert_eq!(editor.text.cursor(), (0, 0));
    control(&mut editor, 'o');
    assert_eq!(editor.text.cursor(), (5, 0));
    control(&mut editor, 'o');
    assert_eq!(editor.text.cursor(), (2, 2));
    control(&mut editor, 'o');
    assert_eq!(editor.text.cursor(), (2, 2));
    control(&mut editor, 'i');
    assert_eq!(editor.text.cursor(), (5, 0));
    editor.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert_eq!(editor.text.cursor(), (0, 0));
    assert_eq!(editor.source(), source);

    commands(&mut editor, "i");
    editor.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert!(editor.source().starts_with("    first"));
    Ok(())
}

#[test]
fn search_jumps_commit_one_origin_and_history_resets_for_replaced_documents() -> Result<()> {
    let source = "target\nsecond\nthird\nfourth\nfifth\n";
    let mut editor = editor(source, 3, 2)?;
    editor.handle_key(KeyEvent::new(
        KeyCode::Char('f'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    ));
    for character in "target".chars() {
        editor.handle_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
    }
    editor.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(editor.text.cursor(), (0, 0));
    control(&mut editor, 'o');
    assert_eq!(editor.text.cursor(), (3, 2));
    control(&mut editor, 'i');
    assert_eq!(editor.text.cursor(), (0, 0));

    editor.replace_source("Replaced document\n");
    let cursor = editor.text.cursor();
    control(&mut editor, 'o');
    assert_eq!(editor.text.cursor(), cursor);
    assert_eq!(editor.source(), "Replaced document\n");
    Ok(())
}

#[test]
fn ciw_saves_through_the_native_buffer_without_losing_trailing_newlines() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("resume.tex");
    let source = "one résumé\n\n";
    fs::write(&path, source)?;
    let mut editor = Editor::open(path.clone())?;
    editor.apply_preferences(EditorPreferences {
        mode: EditingMode::Vim,
        ..EditorPreferences::default()
    });
    editor.clipboard = Box::new(UnavailableClipboard);
    editor.text.move_cursor(CursorMove::Jump(0, 6));
    commands(&mut editor, "ciw");
    editor.paste_text("skill");
    editor.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    editor.save()?;
    assert_eq!(fs::read_to_string(path)?, "one skill\n\n");
    commands(&mut editor, "uu");
    assert_eq!(editor.source(), source);
    Ok(())
}

#[test]
fn older_jumps_are_bounded_and_locations_clamp_after_text_is_deleted() -> Result<()> {
    let source = "word\n".repeat(140);
    let mut editor = editor(&source, 0, 0)?;
    for row in 0..140 {
        editor.text.move_cursor(CursorMove::Jump(row, 0));
        commands(&mut editor, "G");
    }
    for _ in 0..150 {
        control(&mut editor, 'o');
    }
    assert_eq!(editor.text.cursor(), (41, 0));

    editor.text.select_all();
    editor.text.insert_str("李");
    control(&mut editor, 'i');
    assert_eq!(editor.text.cursor(), (0, 0));
    assert_eq!(editor.source(), "李");
    Ok(())
}
