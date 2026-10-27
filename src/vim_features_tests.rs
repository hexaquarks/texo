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
    assert_eq!(editor.text.cursor(), (4, 0));
    control(&mut editor, 'o');
    assert_eq!(editor.text.cursor(), (2, 2));
    control(&mut editor, 'o');
    assert_eq!(editor.text.cursor(), (2, 2));
    control(&mut editor, 'i');
    assert_eq!(editor.text.cursor(), (4, 0));
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
    assert_eq!(editor.text.cursor(), (40, 0));

    editor.text.select_all();
    editor.text.insert_str("李");
    control(&mut editor, 'i');
    assert_eq!(editor.text.cursor(), (0, 0));
    assert_eq!(editor.source(), "李");
    Ok(())
}

#[test]
fn visual_line_keeps_whole_source_lines_selected_in_both_directions() -> Result<()> {
    let source = "first\n\\section{Résumé 李 with a long wrapped heading}\nthird\nfourth\n";
    let mut editor = editor(source, 1, 15)?;
    let mut terminal = Terminal::new(TestBackend::new(22, 14))?;
    terminal.draw(|frame| editor.draw(frame, frame.area(), true))?;

    commands(&mut editor, "V");
    assert_eq!(editor.text.selection_range(), Some(((1, 0), (1, 46))));
    commands(&mut editor, "k");
    assert_eq!(editor.text.selection_range(), Some(((0, 0), (1, 46))));
    for width in [22, 28, 16] {
        let mut resized = Terminal::new(TestBackend::new(width, 14))?;
        resized.draw(|frame| editor.draw(frame, frame.area(), true))?;
        assert_eq!(editor.text.selection_range(), Some(((0, 0), (1, 46))));
        assert_eq!(editor.source(), source);
    }

    commands(&mut editor, "jj");
    assert_eq!(editor.text.selection_range(), Some(((1, 0), (2, 5))));
    commands(&mut editor, "y");
    assert_eq!(
        editor.text.yank_text(),
        "\\section{Résumé 李 with a long wrapped heading}\nthird\n"
    );
    assert!(editor.vim.as_ref().unwrap().mode == vim::Mode::Normal);
    assert_eq!(editor.text.cursor(), (1, 0));
    assert_eq!(editor.source(), source);
    Ok(())
}

#[test]
fn visual_line_pending_g_and_clipboard_shortcuts_clear_selection_metadata() -> Result<()> {
    let mut editor = editor("first\nsecond\nthird\n", 1, 3)?;
    commands(&mut editor, "Vgk");
    assert_eq!(editor.text.selection_range(), Some(((1, 0), (1, 6))));
    commands(&mut editor, "k");
    assert_eq!(editor.text.selection_range(), Some(((0, 0), (1, 6))));

    editor.handle_key(KeyEvent::new(KeyCode::Insert, KeyModifiers::CONTROL));
    editor.text.move_cursor(CursorMove::Jump(2, 1));
    editor.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(editor.text.cursor(), (2, 1));
    assert_eq!(editor.source(), "first\nsecond\nthird\n");
    Ok(())
}

#[test]
fn visual_line_delete_change_and_linewise_paste_match_source_line_operations() -> Result<()> {
    let mut editor = editor("first\nsecond\nthird\n", 1, 3)?;
    commands(&mut editor, "Vkd");
    assert_eq!(editor.source(), "third\n");
    assert_eq!(editor.text.yank_text(), "first\nsecond\n");
    commands(&mut editor, "p");
    assert_eq!(editor.source(), "third\nfirst\nsecond\n");
    commands(&mut editor, "uu");
    assert_eq!(editor.source(), "first\nsecond\nthird\n");

    editor.text.move_cursor(CursorMove::Jump(1, 2));
    commands(&mut editor, "Vjc");
    editor.paste_text("replacement");
    editor.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(editor.source(), "first\nreplacement\n");
    assert_eq!(editor.text.cursor(), (1, 10));

    let mut last = self::editor("first\nlast", 1, 2)?;
    commands(&mut last, "dd");
    assert_eq!(last.source(), "first");
    commands(&mut last, "p");
    assert_eq!(last.source(), "first\nlast");
    Ok(())
}

#[test]
#[ignore = "requires a Vim executable for differential integration checks"]
fn core_commands_match_default_vim() -> Result<()> {
    use std::process::Command;

    // Compare Texo's event adapter, source preservation, and cursor placement with an
    // unconfigured Vim. This deliberately exercises integration rather than widget internals.
    let cases = [
        ("first\nsecond\nthird\n", 1, 3, "Vky"),
        ("first\nsecond\nthird\n", 1, 3, "Vkd"),
        ("first\nsecond\nthird\nfourth\n", 1, 3, "Vjjky"),
        ("first\nsecond\nthird\n", 1, 3, "VjcNEW\u{1b}"),
        ("first\nlast", 1, 2, "ddp"),
        ("first\nlast\n", 1, 2, "ddp"),
        ("first\nsecond\n", 0, 0, "yyp"),
        ("first\nsecond\n", 1, 0, "yyP"),
        ("one résumé words\n", 0, 6, "ciwCV\u{1b}"),
        ("one résumé words\n", 0, 4, "cwCV\u{1b}"),
        ("one résumé words\n", 0, 4, "viwy"),
        ("one two three four five six seven\n", 0, 0, "2d3w"),
        ("one\ntwo\nthree\nfour\n", 0, 1, "d2j"),
        ("one\ntwo\nthree\nfour\n", 1, 1, "2dd"),
        ("one\ntwo\nthree\nfour\n", 1, 1, "2ccNEW\u{1b}"),
        ("  one\n two\n   three\n", 1, 2, "gg"),
        ("  one\n two\n   three\n", 0, 2, "G"),
        ("one\ntwo\nthree\nfour\n", 0, 2, "3G"),
        ("one\ntwo\nthree\nfour\n", 0, 2, "3gg"),
        ("  résumé words\n", 0, 5, "^"),
        ("  résumé words\n", 0, 5, "Inew\u{1b}"),
        ("first\nsecond\n", 1, 0, "h"),
        ("first\nsecond\n", 0, 4, "l"),
        ("long line here\nx\nanother long line\n", 0, 8, "jj"),
        ("some.foo-bar next.word end\n", 0, 0, "W"),
        ("some.foo-bar next.word end\n", 0, 0, "E"),
        ("some.foo-bar next.word end\n", 0, 18, "B"),
        ("some.foo-bar next.word end\n", 0, 0, "dW"),
        ("some.foo-bar next.word end\n", 0, 0, "cWNEW\u{1b}"),
        ("李 foo 李 bar 李 end\n", 0, 0, "2f李"),
        ("abc def ghi\n", 0, 0, "dfg"),
        ("abc def ghi\n", 0, 0, "dtg"),
        ("abc def ghi\n", 0, 8, "dFa"),
        ("abc def ghi\n", 0, 8, "dTa"),
        ("a,b,c,d\n", 0, 0, "f,;"),
        ("a,b,c,d\n", 0, 0, "f,;,"),
        ("李résumé words\n", 0, 1, "2rX"),
        ("李résumé words\n", 0, 1, "2sOK\u{1b}"),
        ("one two\n", 0, 1, "x"),
        ("one two\n", 0, 6, "x"),
        ("one two three\n", 0, 0, "ciwNEW\u{1b}w."),
        ("one two three\n", 0, 0, "dw."),
        ("one two three\n", 0, 0, "dwu."),
        ("abcdefghi\n", 0, 0, "2x3."),
        ("one two three four five six seven\n", 0, 0, "2dw3."),
        ("one two three four five six seven\n", 0, 0, "d2w3."),
        ("a,b,c,d\n", 0, 0, "t,;"),
        ("a,b,c,d\n", 0, 0, "f,d;"),
        ("one\n  two\nthree\n", 1, 0, "b"),
        ("one\ntwo\nthree\n", 0, 0, "yy3p"),
        ("one\ntwo\nthree\n", 1, 0, "yy2P"),
        ("one two three\n", 0, 0, "yiw3p"),
        ("one\ntwo\nthree\nfour\n", 0, 1, "3Vy"),
        ("one\n  two\nthree\n", 0, 0, "J"),
        ("one\n  two\nthree\n", 0, 0, "3J"),
        ("  one\n  two\nthree\n", 0, 0, "dd"),
        ("one\n  two\nthree\n", 1, 1, "yy"),
        ("one\n  two\nthree\n", 1, 1, "Y"),
        ("\nsecond\n", 0, 0, "D"),
        ("one\ntwo\nthree\n", 0, 0, "yyjVp"),
        ("one\ntwo\nthree\n", 0, 0, "yljVp"),
        ("one\ntwo\nthree\n", 0, 0, "yyjVP"),
    ];
    let directory = tempfile::tempdir()?;
    let input: Vec<_> = cases
        .iter()
        .map(|(source, row, column, keys)| {
            serde_json::json!({
                "source": source,
                "row": row,
                "column": column,
                "keys": keys,
            })
        })
        .collect();
    fs::write(
        directory.path().join("cases.json"),
        serde_json::to_vec(&input)?,
    )?;
    fs::write(
        directory.path().join("reference.vim"),
        r#"
set nocompatible
let output = []
for test in json_decode(join(readfile('cases.json'), "\n"))
    enew!
    let lines = split(test.source, "\n", 1)
    if test.source =~ "\n$"
        call remove(lines, -1)
        setlocal endofline
    else
        setlocal noendofline
    endif
    call setline(1, lines)
    let &undolevels = &undolevels
    call cursor(test.row + 1, byteidx(getline(test.row + 1), test.column) + 1)
    execute 'normal! ' . test.keys
    let source = join(getline(1, '$'), "\n") . (&endofline ? "\n" : '')
    let column = strchars(strpart(getline('.'), 0, col('.') - 1))
    call add(output, {'source': source, 'row': line('.') - 1, 'column': column})
endfor
call writefile([json_encode(output)], 'results.json')
qa!
"#,
    )?;
    let status = Command::new("vim")
        .args([
            "-Nu",
            "NONE",
            "-n",
            "-i",
            "NONE",
            "-es",
            "-S",
            "reference.vim",
        ])
        .current_dir(directory.path())
        .status()?;
    assert!(status.success(), "Vim reference process failed");

    let reference: Vec<serde_json::Value> =
        serde_json::from_slice(&fs::read(directory.path().join("results.json"))?)?;
    let mut failures = Vec::new();
    for ((source, row, column, keys), expected) in cases.into_iter().zip(reference) {
        let mut editor = editor(source, row, column)?;
        for character in keys.chars() {
            let code = if character == '\u{1b}' {
                KeyCode::Esc
            } else {
                KeyCode::Char(character)
            };
            editor.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
        }
        let cursor = editor.text.cursor();
        let actual =
            serde_json::json!({"source": editor.source(), "row": cursor.0, "column": cursor.1});
        if actual != expected {
            failures.push(format!(
                "{keys:?} on {source:?}:\nTexo: {actual}\nVim:  {expected}"
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
    Ok(())
}

#[test]
fn core_counts_find_motions_and_repeat_use_the_shared_document() -> Result<()> {
    let mut editor = editor("one two three four five six seven\n", 0, 0)?;
    commands(&mut editor, "2d3w");
    assert_eq!(editor.source(), "seven\n");
    commands(&mut editor, "u");
    assert_eq!(editor.source(), "one two three four five six seven\n");

    commands(&mut editor, "ciwNEW");
    editor.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    commands(&mut editor, "w.");
    assert_eq!(editor.source(), "NEW NEW three four five six seven\n");
    commands(&mut editor, "w2dw3.");
    assert_eq!(editor.source(), "NEW NEW \n");
    Ok(())
}

#[test]
fn cancelled_counts_find_and_replace_commands_never_type_the_pending_argument() -> Result<()> {
    for prefix in ["3", "2d", "df", "ct", "r", "g"] {
        let mut editor = editor("Résumé 李 words\n", 0, 0)?;
        commands(&mut editor, prefix);
        editor.cancel_pending_commands();
        commands(&mut editor, "l");
        assert_eq!(editor.source(), "Résumé 李 words\n", "prefix {prefix}");
        assert_eq!(editor.text.cursor(), (0, 1), "prefix {prefix}");
    }
    Ok(())
}

#[test]
fn vim_search_confirms_repeats_cancels_and_records_jumps() -> Result<()> {
    let mut editor = editor("target one\ntarget two\ntarget three\n", 0, 0)?;
    commands(&mut editor, "/target");
    editor.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(!editor.search.open);
    assert_eq!(editor.text.cursor(), (1, 0));
    commands(&mut editor, "n");
    assert_eq!(editor.text.cursor(), (2, 0));
    commands(&mut editor, "N");
    assert_eq!(editor.text.cursor(), (1, 0));
    control(&mut editor, 'o');
    assert_eq!(editor.text.cursor(), (2, 0));

    commands(&mut editor, "?one");
    editor.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(editor.text.cursor(), (2, 0));
    assert!(editor.text.search_pattern().unwrap().is_match("target"));
    assert_eq!(editor.source(), "target one\ntarget two\ntarget three\n");
    Ok(())
}

#[test]
fn dot_records_literal_paste_but_document_replacement_clears_it() -> Result<()> {
    let mut editor = editor("one two three\n", 0, 0)?;
    commands(&mut editor, "ciw");
    editor.paste_text("李");
    editor.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    commands(&mut editor, "w.");
    assert_eq!(editor.source(), "李 李 three\n");

    editor.replace_source("AI replacement\n");
    commands(&mut editor, "3.");
    assert_eq!(editor.source(), "AI replacement\n");
    commands(&mut editor, "V");
    editor.replace_source("short\n");
    commands(&mut editor, "k");
    assert_eq!(editor.source(), "short\n");
    assert!(editor.vim.as_ref().unwrap().mode == vim::Mode::Normal);
    Ok(())
}

#[test]
fn zz_centers_wrapped_and_unwrapped_views_without_editing() -> Result<()> {
    let source = "\\section{Résumé 李 with a longer heading}\n".repeat(80);
    for wrap in [true, false] {
        let mut editor = editor(&source, 30, 20)?;
        editor.apply_preferences(crate::config::EditorPreferences {
            mode: crate::config::EditingMode::Vim,
            soft_wrap: wrap,
        });
        for (width, height) in [(28, 14), (45, 20)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height))?;
            terminal.draw(|frame| editor.draw(frame, frame.area(), true))?;
            let cursor = editor.text.cursor();
            commands(&mut editor, "zz");
            terminal.draw(|frame| editor.draw(frame, frame.area(), true))?;
            assert_eq!(editor.text.cursor(), cursor);
            assert_eq!(
                editor.text.screen_cursor().row - usize::from(editor.text.scroll_offset().0),
                usize::from(editor.viewport_height / 2),
            );
            assert_eq!(editor.source(), source);
        }
        commands(&mut editor, "z");
        editor.cancel_pending_commands();
        commands(&mut editor, "ziX");
        assert_eq!(editor.source(), source);
    }
    Ok(())
}
