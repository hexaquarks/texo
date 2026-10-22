use std::io;

use ratatui::crossterm::{
    event::{DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture},
    execute,
};

pub struct InputMode;

impl InputMode {
    pub fn enable() -> io::Result<Self> {
        execute!(io::stdout(), EnableBracketedPaste, EnableMouseCapture)?;
        Ok(Self)
    }
}

impl Drop for InputMode {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), DisableBracketedPaste, DisableMouseCapture);
    }
}
