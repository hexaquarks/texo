use super::test_support::*;
use super::*;
use ratatui::{Terminal, backend::TestBackend};

#[test]
#[ignore = "requires macOS, Tectonic cache, and Poppler"]
fn startup_displays_pdf_and_manual_build_recovers_after_failure() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("resume.tex");
    let source = r"\documentclass{article}\begin{document}Startup resume\end{document}";
    std::fs::write(&path, source)?;
    let editor = Editor::open(path.clone())?;
    let mut app = App::start(editor, Preview::default(), directory.path().to_path_buf())?;
    let mut terminal = Terminal::new(TestBackend::new(160, 45))?;

    assert!(
        app.session.job.is_some(),
        "Startup must build without a key press"
    );
    wait_for_visible_pdf(&mut app, &mut terminal)?;
    assert!(path.with_extension("pdf").is_file());
    assert_eq!(app.session.preview.page_status().as_deref(), Some("1 / 1"));

    app.session.editor.replace_source(
        r"\documentclass{missing-texo-test-class}\begin{document}Invalid\end{document}",
    );
    app.handle_prefix_action(KeyCode::Char('b'), Rect::new(0, 0, 160, 45));
    assert!(finish_test_render(&mut app).is_err());
    assert!(
        app.session.diagnostics.is_some(),
        "Failed builds must show diagnostics"
    );
    terminal.draw(|frame| app.draw(frame))?;
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("missing-texo-test-class"));

    app.session.editor.replace_source(source);
    app.handle_prefix_action(KeyCode::Char('b'), Rect::new(0, 0, 160, 45));
    wait_for_visible_pdf(&mut app, &mut terminal)?;
    assert!(app.session.diagnostics.is_none());
    Ok(())
}

#[test]
fn pending_review_protects_the_editor_and_exposes_controls() -> anyhow::Result<()> {
    let (_directory, mut app) = application()?;
    let original = app.session.editor.source();
    app.session.proposal_review = Some(ProposalReview::new(&ProposedEdit {
        source_before: original.clone(),
        source_after: "Changed bullet".into(),
    }));
    app.session.show_proposed = true;

    let mut terminal = Terminal::new(TestBackend::new(160, 45))?;
    terminal.draw(|frame| app.draw(frame))?;
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(text.contains(&format!("Accept {} a", app.prefix.label())));
    assert!(text.contains(&format!("Modify {} m", app.prefix.label())));
    assert!(text.contains(&format!("Discard {} d", app.prefix.label())));
    assert!(text.contains("[ Current ]"));
    assert!(text.contains("[ Proposed ]"));
    assert!(text.contains("Changed bullet"));
    assert!(text.contains("Unapproved changes"));
    assert!(!text.contains("Original document"));

    app.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
    app.handle_event(Event::Paste("hidden edit".into()), Rect::new(0, 0, 160, 45));
    assert_eq!(app.session.editor.source(), original);

    app.apply_proposal();
    assert_eq!(app.session.editor.source(), original);
    assert!(app.session.status.contains("build successfully"));

    app.modify_proposal();
    assert!(app.focus == Focus::Assistant);
    assert!(app.session.proposal_review.is_some());
    assert_eq!(app.assistant_source(), "Changed bullet");

    app.discard_proposal();
    assert!(app.session.proposal_review.is_none());
    assert!(!app.session.show_proposed);
    assert_eq!(app.session.editor.source(), original);
    Ok(())
}

#[test]
fn review_tabs_and_buttons_use_their_visible_hit_areas() -> anyhow::Result<()> {
    let (_directory, mut app) = application()?;
    app.session.proposal_review = Some(ProposalReview::new(&ProposedEdit {
        source_before: app.session.editor.source(),
        source_after: "Pending".into(),
    }));
    let area = Rect::new(0, 0, 160, 45);
    let layout = WorkspaceLayout::new(area, true, Focus::Editor);
    let [_, current, proposed] = preview_header(pane_content(layout.preview));

    for (tab, expected) in [(proposed, true), (current, false)] {
        app.handle_event(
            Event::Mouse(event::MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: tab.x,
                row: tab.y,
                modifiers: KeyModifiers::NONE,
            }),
            area,
        );
        assert_eq!(app.session.show_proposed, expected);
    }

    let discard = app.review_buttons(pane_content(layout.assistant))[2];
    app.handle_event(
        Event::Mouse(event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: discard.x,
            row: discard.y,
            modifiers: KeyModifiers::NONE,
        }),
        area,
    );
    assert!(app.session.proposal_review.is_none());
    Ok(())
}

#[test]
#[ignore = "requires macOS, Tectonic cache, and Poppler"]
fn review_keeps_original_files_and_promotes_the_cached_pdf_only_on_accept() -> anyhow::Result<()> {
    let (directory, mut app) = application()?;
    let original = r"\documentclass{article}\begin{document}Original\end{document}";
    let proposed = r"\documentclass{article}\begin{document}Revised\end{document}";
    app.session.editor.replace_source(original);
    app.session.editor.save()?;
    app.start_render();
    wait_for_reviews(&mut app)?;
    let original_pdf = std::fs::read(app.session.editor.path.with_extension("pdf"))?;

    app.stage_proposal(ProposedEdit {
        source_before: original.into(),
        source_after: proposed.into(),
    });
    wait_for_reviews(&mut app)?;
    assert!(app.session.show_proposed);
    assert_eq!(app.session.editor.source(), original);
    assert_eq!(std::fs::read_to_string(&app.session.editor.path)?, original);
    assert_eq!(
        std::fs::read(app.session.editor.path.with_extension("pdf"))?,
        original_pdf
    );
    let proposed_pdf = std::fs::read(&app.session.proposed_build.as_ref().unwrap().pdf_path)?;

    app.session.show_proposed = false;
    assert!(app.active_preview().page_status().is_some());
    app.session.show_proposed = true;
    assert!(app.active_preview().page_status().is_some());
    assert!(app.session.job.is_none() && app.session.proposed_job.is_none());

    app.apply_proposal();
    assert_eq!(app.session.editor.source(), proposed);
    assert_eq!(std::fs::read_to_string(&app.session.editor.path)?, original);
    assert_eq!(
        std::fs::read(app.session.editor.path.with_extension("pdf"))?,
        proposed_pdf
    );
    assert_eq!(app.session.rendered_source.as_deref(), Some(proposed));
    assert!(app.session.proposal_review.is_none());
    assert!(app.session.job.is_none() && app.session.proposed_job.is_none());

    // A discarded follow-up leaves the accepted preview and unsaved buffer intact.
    app.stage_proposal(ProposedEdit {
        source_before: proposed.into(),
        source_after: original.into(),
    });
    app.discard_proposal();
    assert_eq!(app.session.editor.source(), proposed);
    assert_eq!(
        std::fs::read(app.session.editor.path.with_extension("pdf"))?,
        proposed_pdf
    );
    drop(directory);
    Ok(())
}

#[test]
#[ignore = "requires macOS, Tectonic cache, and Poppler"]
fn failed_or_stale_proposals_cannot_replace_current_content() -> anyhow::Result<()> {
    let (_directory, mut app) = application()?;
    let original = r"\documentclass{article}\begin{document}Original\end{document}";
    app.session.editor.replace_source(original);
    app.session.editor.save()?;
    app.stage_proposal(ProposedEdit {
        source_before: original.into(),
        source_after: r"\undefinedcommand".into(),
    });
    wait_for_reviews(&mut app)?;
    assert!(app.session.proposed_build.is_none());
    app.apply_proposal();
    assert_eq!(app.session.editor.source(), original);

    app.session.editor.text.insert_str("newer ");
    let newer = app.session.editor.source();
    app.apply_proposal();
    assert_eq!(app.session.editor.source(), newer);
    assert!(app.session.status.contains("stale"));
    app.discard_proposal();
    assert_eq!(app.session.editor.source(), newer);
    Ok(())
}

#[test]
fn assistant_edits_refresh_buffer_and_preserve_focus() -> anyhow::Result<()> {
    let (_directory, mut app) = application()?;
    app.session.editor.text.insert_str("Unsaved ");
    let original = app.session.editor.source();
    app.session.rendered_source = Some(original.clone());
    app.focus = Focus::Assistant;
    let edit = ProposedEdit {
        source_before: original.clone(),
        source_after: "Revised resume".into(),
    };
    assert!(app.apply_edit(&edit));
    assert_eq!(app.session.editor.source(), "Revised resume");
    assert!(app.session.editor.is_modified());
    assert!(app.focus == Focus::Assistant);
    assert_eq!(
        std::fs::read_to_string(&app.session.editor.path)?,
        "Original document"
    );

    let mut terminal = Terminal::new(TestBackend::new(160, 50))?;
    terminal.draw(|frame| app.draw(frame))?;
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("Revised resume"));
    assert!(text.contains("PDF outdated") || text.contains("PDF missing"));
    app.session.editor.text.undo();
    app.session.editor.text.undo();
    assert_eq!(app.session.editor.source(), original);
    Ok(())
}

#[test]
fn stale_and_unchanged_assistant_edits_preserve_buffer() -> anyhow::Result<()> {
    let (_directory, mut app) = application()?;
    let original = app.session.editor.source();
    let mut edit = ProposedEdit {
        source_before: original.clone(),
        source_after: original.clone(),
    };
    assert!(app.apply_edit(&edit));
    assert!(!app.session.editor.is_modified());
    assert!(app.session.status.contains("unchanged"));

    app.session.editor.text.insert_str("Newer typing ");
    let newer = app.session.editor.source();
    edit.source_after = "AI revision".into();
    assert!(!app.apply_edit(&edit));
    assert_eq!(app.session.editor.source(), newer);
    assert!(app.session.status.contains("not applied"));
    Ok(())
}

#[test]
fn preview_focus_keeps_keystrokes_out_of_the_editor() -> anyhow::Result<()> {
    let (_directory, mut app) = application()?;
    let original = app.session.editor.source();
    app.focus = Focus::Preview;
    app.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
    assert_eq!(app.session.editor.source(), original);
    app.focus = Focus::Editor;
    app.handle_event(
        Event::Paste("Pasted\ntext".into()),
        Rect::new(0, 0, 100, 30),
    );
    assert!(app.session.editor.source().starts_with("Pasted\ntext"));
    Ok(())
}

#[test]
#[ignore = "requires Tectonic and Poppler"]
fn stale_build_never_publishes_over_a_newer_editor_revision() -> anyhow::Result<()> {
    let (_directory, mut app) = application()?;
    let source = r"\documentclass{article}\begin{document}First\end{document}";
    app.session.editor.replace_source(source);
    app.session.editor.save()?;
    app.start_render();
    app.session.editor.replace_source("Newer unsaved text");

    finish_test_render(&mut app)?;
    assert!(!app.session.editor.path.with_extension("pdf").exists());
    assert!(app.session.rendered_source.is_none());
    assert!(app.session.preview.page_status().is_none());
    assert_eq!(app.session.editor.source(), "Newer unsaved text");
    assert!(app.session.status.contains("outdated"));
    Ok(())
}

#[test]
fn review_actions_stay_above_the_composer_when_resized_or_maximized() -> anyhow::Result<()> {
    let (_directory, mut app) = application()?;
    let source = app.session.editor.source();
    app.session.proposal_review = Some(ProposalReview::new(&ProposedEdit {
        source_before: source.clone(),
        source_after: "Pending proposal".into(),
    }));
    app.focus = Focus::Assistant;

    for maximized in [false, true] {
        app.session.column_expansion = maximized.then_some(Focus::Assistant);
        for (width, height) in [(160, 45), (80, 24), (40, 18)] {
            for draft in [
                "",
                "Please change the heading\nand keep the original details",
            ] {
                app.session.assistant.input.select_all();
                app.session.assistant.input.insert_str(draft);
                let mut terminal = Terminal::new(TestBackend::new(width, height))?;
                terminal.draw(|frame| app.draw(frame))?;
                let area = Rect::new(0, 0, width, height);
                let pane = pane_content(app.workspace_layout(area).assistant);
                let composer = app.session.assistant.input_area(pane);
                let buttons = app.review_buttons(pane);
                let buffer = terminal.backend().buffer();

                for (button, (label, color)) in buttons.into_iter().zip([
                    ("Accept", theme::TEAL),
                    ("Modify", theme::ACCENT),
                    ("Discard", theme::ERROR),
                ]) {
                    assert_eq!(button.bottom() + 1, composer.y);
                    let text: String = (button.x..button.right())
                        .map(|x| buffer[(x, button.y)].symbol())
                        .collect();
                    assert!(text.contains(label), "{width}x{height}: {text:?}");
                    assert_eq!(buffer[(button.x, button.y)].fg, color);
                    assert_eq!(buffer[(button.x, button.y)].bg, theme::SELECTION);
                }
                assert_eq!(app.session.editor.source(), source);
                assert!(app.session.proposal_review.is_some());
            }
        }
    }
    Ok(())
}
