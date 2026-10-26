use ratatui::crossterm::event::KeyCode;
use ratatui_textarea::{CursorMove, DataCursor, TextArea};

use super::{FindMotion, Mode, Vim, VimInput};

impl Vim {
    pub(super) fn take_count(&mut self) -> usize {
        self.count.take().unwrap_or(1)
    }

    pub(super) fn is_motion(code: KeyCode) -> bool {
        matches!(
            code,
            KeyCode::Char(
                'h' | 'j' | 'k' | 'l' | 'w' | 'b' | 'e' | 'W' | 'B' | 'E' | '0' | '^' | '$'
            ) | KeyCode::Left
                | KeyCode::Right
                | KeyCode::Up
                | KeyCode::Down
                | KeyCode::Home
                | KeyCode::End
        )
    }

    pub(super) fn move_motion(
        &mut self,
        code: KeyCode,
        count: usize,
        text: &mut TextArea<'static>,
    ) {
        let vertical = matches!(code, KeyCode::Char('j' | 'k') | KeyCode::Down | KeyCode::Up);
        if vertical {
            let cursor = text.cursor();
            let column = *self.preferred_column.get_or_insert(cursor.1);
            let row = if matches!(code, KeyCode::Char('j') | KeyCode::Down) {
                cursor
                    .0
                    .saturating_add(count)
                    .min(Self::last_source_row(text))
            } else {
                cursor.0.saturating_sub(count)
            };
            let column = column.min(text.lines()[row].chars().count().saturating_sub(1));
            Self::jump(text, DataCursor(row, column));
            return;
        }
        self.preferred_column = None;

        for _ in 0..count {
            let before = text.cursor();
            if matches!(code, KeyCode::Char('W' | 'B' | 'E')) {
                Self::big_word_motion(code, text);
            } else if let Some(motion) = Self::motion(code, text) {
                text.move_cursor(motion);
                if code == KeyCode::Char('b') && text.cursor().0 != before.0 {
                    text.move_cursor(CursorMove::WordBack);
                }
                if code == KeyCode::Char('w') && text.cursor().0 != before.0 {
                    let row = text.cursor().0;
                    Self::jump(text, DataCursor(row, Self::first_nonblank(text, row)));
                }
            }
            if text.cursor() == before {
                break;
            }
        }
        if self.operator.is_none() && self.mode == Mode::Normal {
            Self::clamp_normal_cursor(text);
        }
    }

    fn big_word_motion(code: KeyCode, text: &mut TextArea<'static>) {
        let native = match code {
            KeyCode::Char('W') => CursorMove::WordForward,
            KeyCode::Char('B') => CursorMove::WordBack,
            _ => CursorMove::WordEnd,
        };
        loop {
            let before = text.cursor();
            text.move_cursor(native);
            let cursor = text.cursor();
            let line = &text.lines()[cursor.0];
            let boundary = if code == KeyCode::Char('E') {
                line.chars()
                    .nth(cursor.1 + 1)
                    .is_none_or(char::is_whitespace)
            } else {
                cursor.1 == 0
                    || line
                        .chars()
                        .nth(cursor.1 - 1)
                        .is_some_and(char::is_whitespace)
            };
            if cursor == before || boundary {
                break;
            }
        }
    }

    pub(super) fn find_motion(code: KeyCode) -> Option<FindMotion> {
        match code {
            KeyCode::Char(character @ ('f' | 'F' | 't' | 'T')) => Some(FindMotion {
                forward: character.is_ascii_lowercase(),
                till: matches!(character, 't' | 'T'),
            }),
            _ => None,
        }
    }

    pub(super) fn find_character(
        &mut self,
        motion: FindMotion,
        code: KeyCode,
        repeat: bool,
        text: &mut TextArea<'static>,
    ) -> VimInput {
        let KeyCode::Char(character) = code else {
            self.cancel_pending(text);
            return VimInput::Consumed;
        };
        let count = self.take_count().saturating_mul(self.operator_count.max(1));
        let cursor = text.cursor();
        let mut matches: Vec<_> = text.lines()[cursor.0]
            .chars()
            .enumerate()
            .filter(|(column, value)| {
                *value == character
                    && if motion.forward {
                        *column > cursor.1 + usize::from(repeat && motion.till)
                    } else {
                        *column < cursor.1.saturating_sub(usize::from(repeat && motion.till))
                    }
            })
            .map(|(column, _)| column)
            .collect();
        if !motion.forward {
            matches.reverse();
        }
        let Some(&column) = matches.get(count - 1) else {
            self.cancel_pending(text);
            return VimInput::Consumed;
        };
        let target = if !motion.till {
            column
        } else if motion.forward {
            column - 1
        } else {
            column + 1
        };
        self.last_find = Some((motion, character));
        self.preferred_column = None;
        Self::jump(text, DataCursor(cursor.0, target));
        if let Some(operator) = self.operator {
            if motion.forward {
                Self::include_visual_endpoint(text);
            }
            return self.finish_selection(operator, text);
        }
        VimInput::Consumed
    }

    pub(super) fn replace_character(
        &mut self,
        code: KeyCode,
        text: &mut TextArea<'static>,
    ) -> VimInput {
        let count = self.take_count();
        let cursor = text.cursor();
        let remaining = text.lines()[cursor.0]
            .chars()
            .count()
            .saturating_sub(cursor.1);
        if let KeyCode::Char(character) = code
            && count <= remaining
        {
            text.start_selection();
            Self::jump(text, DataCursor(cursor.0, cursor.1 + count));
            text.insert_str(character.to_string().repeat(count));
            Self::jump(text, DataCursor(cursor.0, cursor.1 + count - 1));
        }
        VimInput::Consumed
    }

    fn motion(code: KeyCode, text: &TextArea<'static>) -> Option<CursorMove> {
        let cursor = text.cursor();
        let last_column = text.lines()[cursor.0].chars().count().saturating_sub(1);
        let target = match code {
            KeyCode::Char('h') | KeyCode::Left => DataCursor(cursor.0, cursor.1.saturating_sub(1)),
            KeyCode::Char('l') | KeyCode::Right => {
                DataCursor(cursor.0, (cursor.1 + 1).min(last_column))
            }
            KeyCode::Char('j') | KeyCode::Down => {
                let row = (cursor.0 + 1).min(Self::last_source_row(text));
                DataCursor(
                    row,
                    cursor
                        .1
                        .min(text.lines()[row].chars().count().saturating_sub(1)),
                )
            }
            KeyCode::Char('k') | KeyCode::Up => {
                let row = cursor.0.saturating_sub(1);
                DataCursor(
                    row,
                    cursor
                        .1
                        .min(text.lines()[row].chars().count().saturating_sub(1)),
                )
            }
            KeyCode::Char('0') | KeyCode::Home => DataCursor(cursor.0, 0),
            KeyCode::Char('^') => DataCursor(cursor.0, Self::first_nonblank(text, cursor.0)),
            KeyCode::Char('$') | KeyCode::End => DataCursor(cursor.0, last_column),
            KeyCode::Char('w') => return Some(CursorMove::WordForward),
            KeyCode::Char('b') => return Some(CursorMove::WordBack),
            KeyCode::Char('e') => return Some(CursorMove::WordEnd),
            _ => return None,
        };

        Some(CursorMove::Jump(
            u16::try_from(target.0).unwrap_or(u16::MAX),
            u16::try_from(target.1).unwrap_or(u16::MAX),
        ))
    }
}
