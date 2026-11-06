use ratatui::{
    Frame,
    crossterm::event::{Event, KeyCode, KeyEventKind, MouseButton, MouseEventKind},
    layout::{Position, Rect},
    widgets::{Paragraph, Wrap},
};

use super::App;
use crate::dialog_ui::{self, ButtonFocus, DialogAction, FormLayout};

const QUIT_LABEL: &str = "Quit & discard";

pub(super) struct QuitDialog {
    losses: Vec<&'static str>,
    focus: ButtonFocus,
}

impl QuitDialog {
    fn new(losses: Vec<&'static str>) -> Self {
        Self {
            losses,
            focus: ButtonFocus::Cancel,
        }
    }

    fn message(&self) -> String {
        format!(
            "Quitting will discard {} across open resumes.\n\nCancel to save your resume or review AI changes.",
            self.losses.join(", ")
        )
    }

    fn layout(&self, screen: Rect) -> FormLayout {
        let width = FormLayout::new(screen, 0, 0, QUIT_LABEL).body.width.max(1);
        let rows = textwrap::wrap(&self.message(), usize::from(width)).len() as u16;
        FormLayout::new(screen, rows, 0, QUIT_LABEL)
    }

    fn handle(&mut self, event: Event, screen: Rect) -> Option<DialogAction> {
        match event {
            Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
                KeyCode::Esc => return Some(DialogAction::Cancel),
                KeyCode::Left => self.focus = ButtonFocus::Cancel,
                KeyCode::Right => self.focus = ButtonFocus::Action,
                KeyCode::Tab | KeyCode::BackTab => {
                    self.focus = if self.focus == ButtonFocus::Cancel {
                        ButtonFocus::Action
                    } else {
                        ButtonFocus::Cancel
                    };
                }
                KeyCode::Enter => {
                    return Some(if self.focus == ButtonFocus::Action {
                        DialogAction::Submit
                    } else {
                        DialogAction::Cancel
                    });
                }
                _ => {}
            },
            Event::Mouse(mouse) if mouse.kind == MouseEventKind::Down(MouseButton::Left) => {
                let [cancel, quit] = self.layout(screen).buttons;
                let position = Position::new(mouse.column, mouse.row);
                if cancel.contains(position) {
                    return Some(DialogAction::Cancel);
                }
                if quit.contains(position) {
                    return Some(DialogAction::Submit);
                }
            }
            _ => {}
        }

        None
    }

    pub(super) fn draw(&self, frame: &mut Frame) {
        let layout = self.layout(frame.area());
        layout.draw(frame, "Quit Texo?");
        frame.render_widget(
            Paragraph::new(self.message()).wrap(Wrap { trim: false }),
            layout.body,
        );
        dialog_ui::buttons(frame, layout.buttons, QUIT_LABEL, self.focus, true);
    }
}

impl App {
    fn quit_losses(&self) -> Vec<&'static str> {
        let mut losses = self.workspace_losses();
        if self
            .settings
            .as_ref()
            .is_some_and(|settings| settings.has_staged_preferences())
        {
            losses.push("unapplied settings");
        }

        losses
    }

    pub(super) fn request_quit(&mut self) -> bool {
        let losses = self.quit_losses();
        if losses.is_empty() {
            return true;
        }

        self.prefix.cancel();
        self.quit_dialog = Some(QuitDialog::new(losses));
        false
    }

    pub(super) fn handle_quit_dialog(&mut self, event: Event, screen: Rect) -> bool {
        if matches!(&event, Event::Key(key) if key.kind == KeyEventKind::Press && key.code == KeyCode::Esc)
        {
            self.quit_dialog = None;
            return false;
        }

        let losses = self.quit_losses();
        let dialog = self.quit_dialog.as_mut().unwrap();

        // A worker can finish while the dialog is open. Show the updated losses
        // and require a fresh choice before allowing that work to be discarded.
        if dialog.losses != losses {
            *dialog = QuitDialog::new(losses);
            return false;
        }

        match dialog.handle(event, screen) {
            Some(DialogAction::Cancel) => self.quit_dialog = None,
            Some(DialogAction::Submit) => return true,
            None => {}
        }

        false
    }
}

#[cfg(test)]
#[path = "quit_dialog_tests.rs"]
mod tests;
