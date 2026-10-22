use ratatui::{
    DefaultTerminal, Frame,
    crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    layout::{Constraint, Layout},
    widgets::Paragraph,
};

use crate::editor::Editor;

pub fn run(terminal: &mut DefaultTerminal, mut editor: Editor) -> anyhow::Result<()> {
    let mut status = String::from("Ctrl+S save | Ctrl+Z undo | Ctrl+Y redo | Ctrl+Q quit");
    let mut confirm_quit = false;

    loop {
        terminal.draw(|frame| draw(frame, &mut editor, &status))?;

        match event::read()? {
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                if key.modifiers.contains(KeyModifiers::CONTROL) {
                    match key.code {
                        KeyCode::Char('q') => {
                            if !editor.is_modified() || confirm_quit {
                                return Ok(());
                            }
                            confirm_quit = true;
                            status = "Unsaved changes. Ctrl+Q again to discard, or Ctrl+S to save."
                                .into();
                            continue;
                        }
                        KeyCode::Char('s') => {
                            status = match editor.save() {
                                Ok(()) => "Saved.".into(),
                                Err(error) => format!("Save failed: {error:#}"),
                            };
                        }
                        KeyCode::Char('z') => {
                            editor.text.undo();
                        }
                        KeyCode::Char('y') => {
                            editor.text.redo();
                        }
                        _ => {
                            editor.text.input(key);
                        }
                    }
                } else {
                    editor.text.input(key);
                }
                confirm_quit = false;
            }
            Event::Paste(text) => {
                editor.text.insert_str(text);
                confirm_quit = false;
            }
            _ => {}
        }
    }
}

fn draw(frame: &mut Frame, editor: &mut Editor, status: &str) {
    let [document, footer] =
        Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(frame.area());

    editor.draw(frame, document, true);
    frame.render_widget(Paragraph::new(status), footer);
}
