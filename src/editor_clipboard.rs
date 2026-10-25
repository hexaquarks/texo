use anyhow::{Context, Result};

pub(super) trait ClipboardAccess {
    fn read(&mut self) -> Result<String>;
    fn write(&mut self, text: String) -> Result<()>;
}

#[derive(Default)]
pub(super) struct SystemClipboard {
    connection: Option<arboard::Clipboard>,
}

impl SystemClipboard {
    fn connection(&mut self) -> Result<&mut arboard::Clipboard> {
        if self.connection.is_none() {
            self.connection = Some(arboard::Clipboard::new().context("Clipboard unavailable")?);
        }

        Ok(self
            .connection
            .as_mut()
            .expect("Clipboard initialized above"))
    }
}

impl ClipboardAccess for SystemClipboard {
    fn read(&mut self) -> Result<String> {
        self.connection()?
            .get_text()
            .context("Could not read clipboard")
    }

    fn write(&mut self, text: String) -> Result<()> {
        self.connection()?
            .set_text(text)
            .context("Could not write clipboard")
    }
}

#[derive(Clone, Copy)]
pub(super) enum ClipboardAction {
    Copy,
    Cut,
    Paste,
}
