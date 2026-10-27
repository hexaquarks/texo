use std::io;

use ratatui::crossterm::{
    event::{
        DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste,
        EnableFocusChange, EnableMouseCapture,
    },
    execute,
};

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
            DisableFocusChange
        );
    }
}
