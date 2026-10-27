use std::io;

use ratatui::{
    Frame,
    crossterm::{
        cursor::SetCursorStyle,
        event::{
            DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste,
            EnableFocusChange, EnableMouseCapture,
        },
        execute,
    },
    layout::{Position, Rect},
};
use ratatui_textarea::TextArea;

pub struct InputMode;

impl InputMode {
    pub fn enable() -> io::Result<Self> {
        execute!(
            io::stdout(),
            EnableBracketedPaste,
            EnableMouseCapture,
            EnableFocusChange
        )?;
        Ok(Self)
    }
}

impl Drop for InputMode {
    fn drop(&mut self) {
        let _ = execute!(
            io::stdout(),
            DisableBracketedPaste,
            DisableMouseCapture,
            DisableFocusChange,
            SetCursorStyle::DefaultUserShape
        );
    }
}

/// Place the terminal caret using the textarea's visual coordinates after rendering.
pub fn show_text_cursor(frame: &mut Frame, text: &TextArea<'_>, area: Rect) {
    let content = text.block().map_or(area, |block| block.inner(area));
    let cursor = text.screen_cursor();
    let (top, left) = text.scroll_offset();
    let row = cursor.row.checked_sub(usize::from(top));
    let column =
        (cursor.col + usize::from(text.line_number_width())).checked_sub(usize::from(left));
    let (Some(row), Some(column)) = (row, column) else {
        return;
    };

    let x = u16::try_from(usize::from(content.x) + column);
    let y = u16::try_from(usize::from(content.y) + row);
    let (Ok(x), Ok(y)) = (x, y) else {
        return;
    };

    let position = Position::new(x, y);
    if content.contains(position) {
        frame.set_cursor_position(position);
    }
}
