use std::{fs, io::ErrorKind, path::PathBuf, sync::OnceLock};

use anyhow::{Context, Result, ensure};
use ratatui::{
    Frame,
    layout::{Position, Rect},
    style::{Color, Style},
    widgets::{Block, Padding},
};
use ratatui_textarea::{CursorMove, TextArea};
use syntect::{
    easy::HighlightLines,
    highlighting::{StyleModifier, Theme, ThemeItem},
    parsing::SyntaxSet,
};
use texo::theme;

const TAB_WIDTH: u8 = 4;
const STARTER_DOCUMENT: &str =
    "\\documentclass{article}\n\\begin{document}\nYour resume starts here.\n\\end{document}\n";

pub struct Editor {
    pub path: PathBuf,
    pub text: TextArea<'static>,
    saved_source: Option<String>,
    highlighting: Highlighting,
}

impl Editor {
    pub fn open(path: PathBuf) -> Result<Self> {
        ensure!(
            path.extension().is_some_and(|extension| extension == "tex"),
            "Choose a document with a .tex extension"
        );
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
        text.set_style(Style::default().fg(theme::TEXT).bg(theme::EDITOR));
        text.set_line_number_style(theme::muted().bg(theme::EDITOR));
        text.set_cursor_line_style(Style::default());
        text.set_cursor_style(Style::default().fg(theme::BACKGROUND).bg(theme::ACCENT));
        text.set_selection_style(Style::default().bg(theme::SELECTION));

        Ok(Self {
            path,
            text,
            saved_source,
            highlighting: Highlighting::new()?,
        })
    }

    pub fn source(&self) -> String {
        self.text.lines().join("\n")
    }

    pub fn is_modified(&self) -> bool {
        self.saved_source.as_deref() != Some(self.source().as_str())
    }

    pub fn click(&mut self, area: Rect, column: u16, row: u16) {
        if !area.contains(Position::new(column, row)) {
            return;
        }

        let content = self.text.block().map_or(area, |block| block.inner(area));
        if !content.contains(Position::new(column, row)) {
            return;
        }
        let (scroll_row, scroll_column) = self.text.scroll_offset();
        let display_row = usize::from(row.saturating_sub(content.y)) + usize::from(scroll_row);
        let display_column =
            usize::from(column.saturating_sub(content.x)) + usize::from(scroll_column);
        let text_column = display_column.saturating_sub(usize::from(self.text.line_number_width()));
        let position = self.text.screen_to_data(display_row, text_column);

        self.text.cancel_selection();
        self.text.move_cursor(CursorMove::Jump(
            u16::try_from(position.0).unwrap_or(u16::MAX),
            u16::try_from(position.1).unwrap_or(u16::MAX),
        ));
    }

    pub fn replace_source(&mut self, source: &str) -> bool {
        if self.source() == source {
            return false;
        }

        self.text.select_all();
        self.text.insert_str(source);
        true
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
        let panel = Block::default()
            .style(Style::default().bg(theme::EDITOR))
            .padding(Padding::new(
                theme::PANEL_PADDING,
                theme::PANEL_PADDING,
                1,
                1,
            ));
        let content = panel.inner(area);
        self.text.set_block(panel);
        self.text.set_cursor_style(if focused {
            Style::default().fg(theme::BACKGROUND).bg(theme::ACCENT)
        } else {
            Style::default()
        });
        frame.render_widget(&self.text, area);

        self.highlighting.update(self.text.lines());
        self.draw_highlighting(frame, content, focused);
    }

    fn draw_highlighting(&self, frame: &mut Frame, content: Rect, focused: bool) {
        let (scroll_row, scroll_column) = self.text.scroll_offset();
        let number_width = self.text.line_number_width();
        let cursor = self.text.cursor();

        // Let the editor own glyphs, scrolling, tabs, selection, and cursor backgrounds.
        // Only recolor the text cells, using its screen mapping for wide Unicode characters.
        for y in 0..content.height {
            let row = usize::from(scroll_row + y);
            if row >= self.text.lines().len() {
                break;
            }

            for x in 0..content.width {
                let display_column = usize::from(scroll_column) + usize::from(x);
                let Some(column) = display_column.checked_sub(usize::from(number_width)) else {
                    continue;
                };
                let position = self.text.screen_to_data(row, column);
                if focused && position == cursor {
                    continue;
                }

                if let Some(color) = self.highlighting.colors[row].get(position.1) {
                    frame.buffer_mut()[(content.x + x, content.y + y)].set_fg(*color);
                }
            }
        }
    }
}

/// Reuse the editor's LaTeX palette when rendering read-only proposal lines.
pub fn source_colors(source: &str) -> Vec<Vec<Color>> {
    let Ok(mut highlighting) = Highlighting::new() else {
        return Vec::new();
    };
    let lines: Vec<String> = source.lines().map(str::to_owned).collect();
    highlighting.update(&lines);

    highlighting.colors
}

struct Highlighting {
    theme: Theme,
    source: Vec<String>,
    colors: Vec<Vec<Color>>,
}

impl Highlighting {
    fn new() -> Result<Self> {
        let mut syntax_theme = Theme::default();
        syntax_theme.settings.foreground = Some(syntax_color(theme::TEXT));
        // These are standard TextMate scopes provided by Syntect's bundled LaTeX grammar.
        for (scope, color) in [
            ("support.function, keyword, storage", theme::SYNTAX_COMMAND),
            (
                "string, entity.name, variable.parameter",
                theme::SYNTAX_ARGUMENT,
            ),
            ("comment", theme::MUTED),
            ("punctuation", theme::TEXT),
        ] {
            syntax_theme.scopes.push(ThemeItem {
                scope: scope.parse()?,
                style: StyleModifier {
                    foreground: Some(syntax_color(color)),
                    ..StyleModifier::default()
                },
            });
        }

        Ok(Self {
            theme: syntax_theme,
            source: Vec::new(),
            colors: Vec::new(),
        })
    }

    fn update(&mut self, lines: &[String]) {
        if self.source == lines {
            return;
        }

        static SYNTAXES: OnceLock<SyntaxSet> = OnceLock::new();
        let syntaxes = SYNTAXES.get_or_init(SyntaxSet::load_defaults_newlines);
        let syntax = syntaxes
            .find_syntax_by_extension("tex")
            .unwrap_or_else(|| syntaxes.find_syntax_plain_text());
        let mut highlighter = HighlightLines::new(syntax, &self.theme);

        // Preserve parser state across lines, including multiline comments and arguments.
        self.colors = lines
            .iter()
            .map(|line| {
                let terminated = format!("{line}\n");
                match highlighter.highlight_line(&terminated, syntaxes) {
                    Ok(tokens) => tokens
                        .into_iter()
                        .flat_map(|(style, text)| {
                            let color = style.foreground;
                            text.chars()
                                .map(move |_| Color::Rgb(color.r, color.g, color.b))
                        })
                        .take(line.chars().count())
                        .collect(),
                    Err(_) => vec![theme::TEXT; line.chars().count()],
                }
            })
            .collect();
        self.source = lines.to_vec();
    }
}

fn syntax_color(color: Color) -> syntect::highlighting::Color {
    let (r, g, b) = match color {
        Color::Rgb(r, g, b) => (r, g, b),
        _ => unreachable!("The workspace syntax palette uses RGB colors"),
    };
    syntect::highlighting::Color {
        r,
        g,
        b,
        a: u8::MAX,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacing_source_preserves_native_undo_history() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("resume.tex");
        fs::write(&path, "Original résumé\n")?;
        let mut editor = Editor::open(path)?;
        editor.text.insert_str("Earlier edit\n");
        let before_proposal = editor.source();

        assert!(editor.replace_source("Revised résumé\nSecond line\n"));
        assert_eq!(editor.source(), "Revised résumé\nSecond line\n");
        assert!(editor.text.undo());
        assert!(editor.text.undo());
        assert_eq!(editor.source(), before_proposal);
        assert!(editor.text.undo());
        assert_eq!(editor.source(), "Original résumé\n");
        Ok(())
    }

    #[test]
    fn clicking_maps_tabs_unicode_and_scrolled_rows_to_characters() -> Result<()> {
        use ratatui::{Terminal, backend::TestBackend};

        let directory = tempfile::tempdir()?;
        let path = directory.path().join("resume.tex");
        fs::write(&path, ["李\t\\section{Résumé}"; 8].join("\n"))?;
        let mut editor = Editor::open(path)?;
        editor.text.move_cursor(CursorMove::Bottom);
        let mut terminal = Terminal::new(TestBackend::new(40, 4))?;
        terminal.draw(|frame| editor.draw(frame, frame.area(), true))?;
        let (scroll_row, _) = editor.text.scroll_offset();
        assert!(scroll_row > 0);

        let text_start = theme::PANEL_PADDING + editor.text.line_number_width();
        editor.click(Rect::new(0, 0, 40, 4), text_start + u16::from(TAB_WIDTH), 1);
        assert_eq!(editor.text.cursor(), (usize::from(scroll_row), 2));
        editor.text.insert_str("CLICK");
        assert!(editor.text.lines()[usize::from(scroll_row)].starts_with("李\tCLICK\\section"));
        Ok(())
    }

    #[test]
    fn syntax_colors_follow_tex_commands_and_comments_after_edits() -> Result<()> {
        let mut highlighting = Highlighting::new()?;
        highlighting.update(&[r"\section{Experience} % Notes".to_owned()]);
        assert_eq!(highlighting.colors[0][1], theme::SYNTAX_COMMAND);
        assert_eq!(highlighting.colors[0][9], theme::SYNTAX_ARGUMENT);
        assert_eq!(*highlighting.colors[0].last().unwrap(), theme::MUTED);

        highlighting.update(&["Plain résumé".to_owned()]);
        assert_eq!(highlighting.colors[0], vec![theme::TEXT; 12]);
        Ok(())
    }

    #[test]
    fn rendering_preserves_selection_and_maps_scrolled_unicode() -> Result<()> {
        use ratatui::{Terminal, backend::TestBackend};
        use ratatui_textarea::CursorMove;

        let directory = tempfile::tempdir()?;
        let path = directory.path().join("resume.tex");
        fs::write(&path, "李\t\\section{Résumé} % Notes")?;
        let mut editor = Editor::open(path)?;
        editor.text.select_all();
        editor.text.move_cursor(CursorMove::End);
        let mut terminal = Terminal::new(TestBackend::new(18, 4))?;
        terminal.draw(|frame| editor.draw(frame, frame.area(), true))?;

        assert!(editor.text.scroll_offset().1 > 0);
        let buffer = terminal.backend().buffer();
        assert!(
            buffer
                .content()
                .iter()
                .any(|cell| cell.bg == theme::SELECTION)
        );
        assert!(buffer.content().iter().any(|cell| cell.fg == theme::MUTED));
        assert_eq!(editor.source(), "李\t\\section{Résumé} % Notes");
        Ok(())
    }

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
