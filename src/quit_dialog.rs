use ratatui::{
    Frame,
    crossterm::event::{Event, KeyCode, KeyEventKind, MouseButton, MouseEventKind},
    layout::{Position, Rect},
    widgets::Paragraph,
};

use super::App;
use crate::dialog_ui::{self, ButtonFocus, DialogAction, FormLayout};

const QUIT_LABEL: &str = "Quit & discard";

pub(super) struct QuitDialog {
    losses: Vec<String>,
    focus: ButtonFocus,
    scroll: u16,
}

impl QuitDialog {
    fn new(losses: Vec<String>) -> Self {
        Self {
            losses,
            focus: ButtonFocus::Cancel,
            scroll: 0,
        }
    }

    fn message(&self) -> String {
        format!(
            "Quitting will discard:\n\n{}\n\nCancel to return to these resumes and save or review your work.",
            self.losses.join("\n\n")
        )
    }

    fn wrapped_message(&self, screen: Rect) -> String {
        let width = FormLayout::new(screen, 0, 0, QUIT_LABEL).body.width.max(1);
        textwrap::fill(&self.message(), usize::from(width))
    }

    fn layout(&self, screen: Rect) -> FormLayout {
        let rows = self
            .wrapped_message(screen)
            .lines()
            .count()
            .min(u16::MAX as usize) as u16;
        FormLayout::new(screen, rows, 0, QUIT_LABEL)
    }

    fn max_scroll(&self, screen: Rect) -> u16 {
        let rows = self
            .wrapped_message(screen)
            .lines()
            .count()
            .min(u16::MAX as usize) as u16;
        rows.saturating_sub(self.layout(screen).body.height)
    }

    fn handle(&mut self, event: Event, screen: Rect) -> Option<DialogAction> {
        match event {
            Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
                KeyCode::Esc => return Some(DialogAction::Cancel),
                KeyCode::Left => self.focus = ButtonFocus::Cancel,
                KeyCode::Right => self.focus = ButtonFocus::Action,
                KeyCode::Up => self.scroll = self.scroll.saturating_sub(1),
                KeyCode::Down => self.scroll = self.scroll.saturating_add(1),
                KeyCode::PageUp => {
                    self.scroll = self.scroll.saturating_sub(self.layout(screen).body.height);
                }
                KeyCode::PageDown => {
                    self.scroll = self.scroll.saturating_add(self.layout(screen).body.height);
                }
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
            Event::Mouse(mouse) if mouse.kind == MouseEventKind::ScrollUp => {
                self.scroll = self.scroll.saturating_sub(1);
            }
            Event::Mouse(mouse) if mouse.kind == MouseEventKind::ScrollDown => {
                self.scroll = self.scroll.saturating_add(1);
            }
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

        self.scroll = self.scroll.min(self.max_scroll(screen));
        None
    }

    pub(super) fn draw(&self, frame: &mut Frame) {
        let layout = self.layout(frame.area());
        layout.draw(frame, "Quit Texo?");
        frame.render_widget(
            Paragraph::new(self.wrapped_message(frame.area()))
                .scroll((self.scroll.min(self.max_scroll(frame.area())), 0)),
            layout.body,
        );
        dialog_ui::buttons(frame, layout.buttons, QUIT_LABEL, self.focus, true);
        if self.max_scroll(frame.area()) > 0 {
            frame.render_widget(
                Paragraph::new(texo::theme::shortcuts(&[
                    ("↑/↓", "scroll"),
                    ("Tab", "move"),
                    ("Enter", "activate"),
                    ("Esc", "cancel"),
                ])),
                layout.help,
            );
        }
    }
}

impl App {
    fn quit_losses(&self) -> Vec<String> {
        let mut sessions: Vec<_> = std::iter::once(&self.session)
            .chain(self.sessions.values())
            .collect();
        sessions.sort_by_key(|session| &session.editor.path);

        let mut losses = Vec::new();
        for session in sessions {
            let mut reasons = Vec::new();
            if session.editor.is_modified() && !session.empty_workspace {
                reasons.push("unsaved text");
            }
            if session.assistant.is_running() || session.proposal_review.is_some() {
                reasons.push("pending AI work or proposals");
            }
            if session.assistant.has_chat() {
                reasons.push("chat");
            }
            if reasons.is_empty() {
                continue;
            }

            let path = session
                .editor
                .path
                .strip_prefix(self.projects.root())
                .unwrap_or(&session.editor.path);
            let label = match self.projects.project_for(&session.editor.path) {
                Some(project) => format!("{} ({})", project.manifest.name, path.display()),
                None => path.display().to_string(),
            };
            losses.push(format!("{label}\n  {}", reasons.join(", ")));
        }

        if self
            .settings
            .as_ref()
            .is_some_and(|settings| settings.has_staged_preferences())
        {
            losses.push("unapplied settings".into());
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
