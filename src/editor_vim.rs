// Adapted from ratatui-textarea 0.9.3 examples/vim.rs (MIT).
// Copyright (c) 2022 rhysd; Copyright (c) The Ratatui Developers.
// See THIRD_PARTY_LICENSES/ratatui-textarea.txt for the license.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui_textarea::{CursorMove, DataCursor, Scrolling, TextArea};

use super::clipboard::ClipboardAction;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum Mode {
    #[default]
    Normal,
    Insert,
    Visual,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Operator {
    Delete,
    Change,
    Yank,
}

impl Operator {
    fn from_key(code: KeyCode) -> Option<Self> {
        match code {
            KeyCode::Char('d') => Some(Self::Delete),
            KeyCode::Char('c') => Some(Self::Change),
            KeyCode::Char('y') => Some(Self::Yank),
            _ => None,
        }
    }

    fn clipboard_action(self) -> ClipboardAction {
        match self {
            Self::Delete | Self::Change => ClipboardAction::Cut,
            Self::Yank => ClipboardAction::Copy,
        }
    }
}

pub(super) enum VimInput {
    Consumed,
    Insert,
    Clipboard(ClipboardAction),
}

#[derive(Default)]
pub(super) struct Vim {
    pub(super) mode: Mode,
    pending_g: bool,
    operator: Option<Operator>,
}

impl Vim {
    pub(super) fn label(&self) -> &'static str {
        match self.operator {
            Some(Operator::Delete) => "NORMAL d…",
            Some(Operator::Change) => "NORMAL c…",
            Some(Operator::Yank) => "NORMAL y…",
            None => match self.mode {
                Mode::Normal if self.pending_g => "NORMAL g…",
                Mode::Normal => "NORMAL",
                Mode::Insert => "INSERT",
                Mode::Visual if self.pending_g => "VISUAL g…",
                Mode::Visual => "VISUAL",
            },
        }
    }

    pub(super) fn finish_mouse_selection(&mut self, text: &mut TextArea<'static>) {
        self.cancel_pending(text);
        if self.mode != Mode::Insert {
            self.mode = if text.is_selecting() {
                Mode::Visual
            } else {
                Mode::Normal
            };
        }
    }

    pub(super) fn cancel_pending(&mut self, text: &mut TextArea<'static>) {
        self.pending_g = false;
        if self.operator.take().is_some() {
            text.cancel_selection();
        }
    }

    // Only Insert mode may pass input to the ordinary editor. Unknown commands are consumed.
    pub(super) fn handle_key(&mut self, key: KeyEvent, text: &mut TextArea<'static>) -> VimInput {
        let cancel = key.code == KeyCode::Esc
            || (matches!(key.code, KeyCode::Char('[' | 'c'))
                && key.modifiers == KeyModifiers::CONTROL);
        if cancel {
            self.mode = Mode::Normal;
            self.cancel_pending(text);
            text.cancel_selection();
            return VimInput::Consumed;
        }
        if self.mode == Mode::Insert {
            return VimInput::Insert;
        }

        if key
            .modifiers
            .intersects(KeyModifiers::ALT | KeyModifiers::CONTROL)
        {
            self.cancel_pending(text);
            self.handle_control_key(key, text);
            return VimInput::Consumed;
        }

        let was_pending_g = std::mem::take(&mut self.pending_g);
        if let Some(operator) = self.operator {
            return self.handle_operator(operator, key.code, was_pending_g, text);
        }
        if let Some(motion) = Self::motion(key.code) {
            text.move_cursor(motion);
            return VimInput::Consumed;
        }
        if let Some(operator) = Operator::from_key(key.code) {
            if self.mode == Mode::Visual {
                Self::include_visual_endpoint(text);
                return self.finish_operator(operator);
            }

            text.cancel_selection();
            text.start_selection();
            self.operator = Some(operator);
            return VimInput::Consumed;
        }

        self.handle_command(key.code, was_pending_g, text)
    }

    fn handle_operator(
        &mut self,
        operator: Operator,
        code: KeyCode,
        was_pending_g: bool,
        text: &mut TextArea<'static>,
    ) -> VimInput {
        if was_pending_g && code != KeyCode::Char('g') {
            self.cancel_pending(text);
            return VimInput::Consumed;
        }

        if Operator::from_key(code) == Some(operator) {
            Self::select_source_line(text);
        } else if let Some(motion) = Self::motion(code) {
            text.move_cursor(motion);
            if code == KeyCode::Char('e') {
                text.move_cursor(CursorMove::Forward);
            }
        } else {
            match code {
                KeyCode::Char('g') if was_pending_g => text.move_cursor(CursorMove::Top),
                KeyCode::Char('g') => {
                    self.pending_g = true;
                    return VimInput::Consumed;
                }
                KeyCode::Char('G') => text.move_cursor(CursorMove::Bottom),
                _ => {
                    self.cancel_pending(text);
                    return VimInput::Consumed;
                }
            }
        }

        self.finish_operator(operator)
    }

    fn finish_operator(&mut self, operator: Operator) -> VimInput {
        self.operator = None;
        self.pending_g = false;
        self.mode = if operator == Operator::Change {
            Mode::Insert
        } else {
            Mode::Normal
        };

        VimInput::Clipboard(operator.clipboard_action())
    }

    fn select_source_line(text: &mut TextArea<'static>) {
        text.cancel_selection();
        text.move_cursor(CursorMove::Head);
        text.start_selection();

        let next_row = text.cursor().0 + 1;
        if next_row < text.lines().len() {
            // Down follows wrapped display rows; a line operator needs the next source row.
            Self::jump(text, DataCursor(next_row, 0));
        } else {
            text.move_cursor(CursorMove::End);
        }
    }

    fn include_visual_endpoint(text: &mut TextArea<'static>) {
        let Some((start, end)) = text.selection_range() else {
            return;
        };

        // The component selects an exclusive range. Include the final character in either direction.
        text.cancel_selection();
        Self::jump(text, start.into());
        text.start_selection();
        Self::jump(text, end.into());
        text.move_cursor(CursorMove::Forward);
    }

    fn jump(text: &mut TextArea<'static>, cursor: DataCursor) {
        text.move_cursor(CursorMove::Jump(
            u16::try_from(cursor.0).unwrap_or(u16::MAX),
            u16::try_from(cursor.1).unwrap_or(u16::MAX),
        ));
    }

    fn handle_command(
        &mut self,
        code: KeyCode,
        was_pending_g: bool,
        text: &mut TextArea<'static>,
    ) -> VimInput {
        match code {
            KeyCode::Char('g') if was_pending_g => text.move_cursor(CursorMove::Top),
            KeyCode::Char('g') => self.pending_g = true,
            KeyCode::Char('G') => text.move_cursor(CursorMove::Bottom),
            KeyCode::Char('i' | 'a' | 'I' | 'A' | 'o' | 'O') if self.mode == Mode::Normal => {
                self.enter_insert(code, text);
            }
            KeyCode::Char('v') if self.mode == Mode::Visual => {
                text.cancel_selection();
                self.mode = Mode::Normal;
            }
            KeyCode::Char('v' | 'V') if self.mode == Mode::Normal => {
                if code == KeyCode::Char('V') {
                    text.move_cursor(CursorMove::Head);
                }
                text.start_selection();
                if code == KeyCode::Char('V') {
                    text.move_cursor(CursorMove::End);
                }
                self.mode = Mode::Visual;
            }
            KeyCode::Char('D' | 'C' | 'S') => {
                if self.mode == Mode::Visual {
                    Self::include_visual_endpoint(text);
                } else {
                    text.cancel_selection();
                    if code == KeyCode::Char('S') {
                        text.move_cursor(CursorMove::Head);
                    }
                    let start = text.cursor();
                    text.start_selection();
                    text.move_cursor(CursorMove::End);
                    if text.cursor() == start {
                        text.move_cursor(CursorMove::Forward);
                    }
                }
                let operator = if code == KeyCode::Char('D') {
                    Operator::Delete
                } else {
                    Operator::Change
                };
                return self.finish_operator(operator);
            }
            KeyCode::Char('x') if self.mode == Mode::Normal => {
                let cursor = text.cursor();
                if cursor.1 < text.lines()[cursor.0].chars().count()
                    || text.lines()[cursor.0].is_empty()
                {
                    text.start_selection();
                    text.move_cursor(CursorMove::Forward);
                    return self.finish_operator(Operator::Delete);
                }
            }
            KeyCode::Char('J') => {
                self.join_lines(text);
            }
            KeyCode::Char('p') => {
                if self.mode == Mode::Visual {
                    Self::include_visual_endpoint(text);
                }
                text.paste();
                self.mode = Mode::Normal;
            }
            KeyCode::Char('u') => {
                text.cancel_selection();
                text.undo();
                self.mode = Mode::Normal;
            }
            KeyCode::PageUp => text.scroll(Scrolling::PageUp),
            KeyCode::PageDown => text.scroll(Scrolling::PageDown),
            _ => {}
        }

        VimInput::Consumed
    }

    fn join_lines(&mut self, text: &mut TextArea<'static>) {
        let row = text.cursor().0;
        let (start_row, end_row) = if self.mode == Mode::Visual {
            text.selection_range()
                .map_or((row, row), |(start, end)| (start.0, end.0))
        } else {
            let next_row = if row + 1 < text.lines().len() {
                row + 1
            } else {
                row
            };
            (row, next_row)
        };

        text.cancel_selection();
        Self::jump(text, DataCursor(start_row, 0));
        for _ in start_row..end_row {
            text.move_cursor(CursorMove::End);
            text.delete_next_char();
            text.insert_char(' ');
        }
        self.mode = Mode::Normal;
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

    fn handle_control_key(&mut self, key: KeyEvent, text: &mut TextArea<'static>) {
        if !key.modifiers.contains(KeyModifiers::CONTROL)
            || key.modifiers.contains(KeyModifiers::ALT)
        {
            return;
        }

        match key.code {
            KeyCode::Char('e') => text.scroll((1, 0)),
            KeyCode::Char('y') => text.scroll((-1, 0)),
            KeyCode::Char('d') => text.scroll(Scrolling::HalfPageDown),
            KeyCode::Char('u') => text.scroll(Scrolling::HalfPageUp),
            KeyCode::Char('f') => text.scroll(Scrolling::PageDown),
            KeyCode::Char('b') => text.scroll(Scrolling::PageUp),
            KeyCode::Char('r') => {
                text.cancel_selection();
                text.redo();
                self.mode = Mode::Normal;
            }
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
