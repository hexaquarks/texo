// Adapted from ratatui-textarea 0.9.3 examples/vim.rs (MIT).
// Copyright (c) 2022 rhysd; Copyright (c) The Ratatui Developers.
// See THIRD_PARTY_LICENSES/ratatui-textarea.txt for the license.

use ratatui::{
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
    text::Line,
};
use ratatui_textarea::{CursorMove, Scrolling, TextArea};
use texo::theme;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum Mode {
    #[default]
    Normal,
    Insert,
    Visual,
}

#[derive(Default)]
pub(super) struct Vim {
    pub(super) mode: Mode,
    pending_g: bool,
}

impl Vim {
    pub(super) fn label(&self) -> &'static str {
        match self.mode {
            Mode::Normal if self.pending_g => "NORMAL g…",
            Mode::Normal => "NORMAL",
            Mode::Insert => "INSERT",
            Mode::Visual if self.pending_g => "VISUAL g…",
            Mode::Visual => "VISUAL",
        }
    }

    pub(super) fn hints(&self) -> Line<'static> {
        match self.mode {
            Mode::Normal => {
                theme::shortcuts(&[("i", "Insert"), ("v", "Visual"), ("Ctrl+Shift+F", "Find")])
            }
            Mode::Insert => theme::shortcuts(&[("Esc", "Normal"), ("Ctrl+Shift+F", "Find")]),
            Mode::Visual => theme::shortcuts(&[("Esc/v", "Normal"), ("hjkl/wbe", "Select")]),
        }
    }

    pub(super) fn finish_mouse_selection(&mut self, selected: bool) {
        self.cancel_pending();
        if self.mode != Mode::Insert {
            self.mode = if selected { Mode::Visual } else { Mode::Normal };
        }
    }

    pub(super) fn cancel_pending(&mut self) {
        self.pending_g = false;
    }

    // Only Insert mode may pass input to the ordinary editor. Unknown commands are consumed.
    pub(super) fn handle_key(&mut self, key: KeyEvent, text: &mut TextArea<'static>) -> bool {
        if key.code == KeyCode::Esc
            || (key.code == KeyCode::Char('[') && key.modifiers == KeyModifiers::CONTROL)
        {
            self.mode = Mode::Normal;
            self.cancel_pending();
            text.cancel_selection();
            return true;
        }

        if self.mode == Mode::Insert {
            return false;
        }

        let was_pending_g = std::mem::take(&mut self.pending_g);
        if key.modifiers.contains(KeyModifiers::ALT) {
            return true;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            self.handle_scroll(key, text);
            return true;
        }

        if let Some(motion) = Self::motion(key.code) {
            text.move_cursor(motion);
            return true;
        }

        match key.code {
            KeyCode::Char('g') if was_pending_g => text.move_cursor(CursorMove::Top),
            KeyCode::Char('g') => self.pending_g = true,
            KeyCode::Char('G') => text.move_cursor(CursorMove::Bottom),
            KeyCode::Char('i' | 'a' | 'I' | 'A' | 'o' | 'O') if self.mode == Mode::Normal => {
                self.enter_insert(key.code, text);
            }
            KeyCode::Char('v') if self.mode == Mode::Visual => {
                text.cancel_selection();
                self.mode = Mode::Normal;
            }
            KeyCode::Char('v' | 'V') if self.mode == Mode::Normal => {
                if key.code == KeyCode::Char('V') {
                    text.move_cursor(CursorMove::Head);
                }
                text.start_selection();
                if key.code == KeyCode::Char('V') {
                    text.move_cursor(CursorMove::End);
                }
                self.mode = Mode::Visual;
            }
            KeyCode::PageUp => text.scroll(Scrolling::PageUp),
            KeyCode::PageDown => text.scroll(Scrolling::PageDown),
            _ => {}
        }

        true
    }

    fn motion(code: KeyCode) -> Option<CursorMove> {
        match code {
            KeyCode::Char('h') | KeyCode::Left => Some(CursorMove::Back),
            KeyCode::Char('j') | KeyCode::Down => Some(CursorMove::Down),
            KeyCode::Char('k') | KeyCode::Up => Some(CursorMove::Up),
            KeyCode::Char('l') | KeyCode::Right => Some(CursorMove::Forward),
            KeyCode::Char('w') => Some(CursorMove::WordForward),
            KeyCode::Char('b') => Some(CursorMove::WordBack),
            KeyCode::Char('e') => Some(CursorMove::WordEnd),
            KeyCode::Char('0' | '^') | KeyCode::Home => Some(CursorMove::Head),
            KeyCode::Char('$') | KeyCode::End => Some(CursorMove::End),
            _ => None,
        }
    }

    fn handle_scroll(&self, key: KeyEvent, text: &mut TextArea<'static>) {
        match key.code {
            KeyCode::Char('e') => text.scroll((1, 0)),
            KeyCode::Char('y') => text.scroll((-1, 0)),
            KeyCode::Char('d') => text.scroll(Scrolling::HalfPageDown),
            KeyCode::Char('u') => text.scroll(Scrolling::HalfPageUp),
            KeyCode::Char('f') => text.scroll(Scrolling::PageDown),
            KeyCode::Char('b') => text.scroll(Scrolling::PageUp),
            _ => {}
        }
    }

    fn enter_insert(&mut self, code: KeyCode, text: &mut TextArea<'static>) {
        text.cancel_selection();
        match code {
            KeyCode::Char('a') => {
                let cursor = text.cursor();
                if cursor.1 < text.lines()[cursor.0].chars().count() {
                    text.move_cursor(CursorMove::Forward);
                }
            }
            KeyCode::Char('I') => text.move_cursor(CursorMove::Head),
            KeyCode::Char('A') => text.move_cursor(CursorMove::End),
            KeyCode::Char('o') => {
                text.move_cursor(CursorMove::End);
                text.insert_newline();
            }
            KeyCode::Char('O') => {
                text.move_cursor(CursorMove::Head);
                text.insert_newline();
                text.move_cursor(CursorMove::Up);
            }
            _ => {}
        }
        self.mode = Mode::Insert;
    }
}
