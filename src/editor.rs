use std::{fs, io::ErrorKind, path::PathBuf};

use anyhow::{Context, Result};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    widgets::Block,
};
use ratatui_textarea::TextArea;

const TAB_WIDTH: u8 = 4;
const STARTER_DOCUMENT: &str =
    "\\documentclass{article}\n\\begin{document}\nYour resume starts here.\n\\end{document}\n";

pub struct Editor {
    pub path: PathBuf,
    pub text: TextArea<'static>,
    saved_source: Option<String>,
}

impl Editor {
    pub fn open(path: PathBuf) -> Result<Self> {
        let path = std::env::current_dir()?.join(path);
        let saved_source = match fs::read_to_string(&path) {
            Ok(source) => Some(source),
            Err(error) if error.kind() == ErrorKind::NotFound => None,
            Err(error) => return Err(error).context("Could not open the TeX document"),
        };

        let source = saved_source.as_deref().unwrap_or(STARTER_DOCUMENT);
        // Keep the final empty line so opening and saving preserves a trailing newline.
        let mut text = TextArea::new(source.split('\n').map(String::from).collect());
        text.set_tab_length(TAB_WIDTH);
        text.set_line_number_style(Style::default().fg(Color::DarkGray));

        Ok(Self {
            path,
            text,
            saved_source,
        })
    }

    pub fn source(&self) -> String {
        self.text.lines().join("\n")
    }

    pub fn is_modified(&self) -> bool {
        self.saved_source.as_deref() != Some(self.source().as_str())
    }

    pub fn save(&mut self) -> Result<()> {
        let source = self.source();
        let directory = self
            .path
            .parent()
            .context("The document needs a parent directory")?;
        let mut temporary = tempfile::NamedTempFile::new_in(directory)?;

        use std::io::Write;
        temporary.write_all(source.as_bytes())?;
        temporary
            .persist(&self.path)
            .context("Could not save the TeX document")?;
        self.saved_source = Some(source);
        Ok(())
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect, focused: bool) {
        let modified = if self.is_modified() { " *" } else { "" };
        let border_color = if focused {
            Color::Cyan
        } else {
            Color::DarkGray
        };
        let title = format!(" {}{} ", self.path.display(), modified);
        self.text.set_block(
            Block::bordered()
                .title(title)
                .border_style(Style::default().fg(border_color)),
        );
        frame.render_widget(&self.text, area);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opening_and_saving_preserves_unicode_and_trailing_newlines() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("resume.tex");
        let source = "Résumé — 李\n\n";
        fs::write(&path, source)?;
        let mut editor = Editor::open(path.clone())?;

        assert!(!editor.is_modified());
        editor.save()?;
        assert_eq!(fs::read_to_string(path)?, source);
        Ok(())
    }

    #[test]
    fn undo_restores_the_saved_document() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("resume.tex");
        fs::write(&path, "Original")?;
        let mut editor = Editor::open(path)?;

        editor.text.insert_str("Changed ");
        assert!(editor.is_modified());
        editor.text.undo();
        assert!(!editor.is_modified());
        Ok(())
    }

    #[test]
    fn a_new_document_is_not_written_until_saved() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("new.tex");
        let mut editor = Editor::open(path.clone())?;

        assert!(!path.exists());
        assert!(editor.is_modified());
        editor.save()?;
        assert!(path.exists());
        assert!(!editor.is_modified());
        Ok(())
    }
}
