use std::{fs, io::ErrorKind, path::PathBuf, sync::OnceLock};

use anyhow::{Context, Result, ensure};
use ratatui::{
    Frame,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind},
    layout::{Position, Rect},
    style::{Color, Style},
    widgets::{Block, Padding},
};
use ratatui_textarea::{CursorMove, TextArea, WrapMode};

use crate::config::EditorPreferences;
use syntect::{
    easy::HighlightLines,
    highlighting::{StyleModifier, Theme, ThemeItem},
    parsing::SyntaxSet,
};
use texo::theme;

const TAB_WIDTH: u8 = 4;
const HORIZONTAL_SCROLL_COLUMNS: i16 = 4;
pub const STARTER_DOCUMENT: &str =
    "\\documentclass{article}\n\\begin{document}\nYour resume starts here.\n\\end{document}\n";

#[derive(Debug, PartialEq, Eq)]
pub enum DiskState {
    Unchanged,
    Changed,
    Missing,
}

pub struct Editor {
    pub path: PathBuf,
    pub text: TextArea<'static>,
    saved_source: Option<String>,
    highlighting: Highlighting,
    dragging: bool,
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

        Self::from_source(path, saved_source)
    }

    pub fn placeholder(path: PathBuf) -> Result<Self> {
        Self::from_source(path, None)
    }

    fn from_source(path: PathBuf, saved_source: Option<String>) -> Result<Self> {
        let source = saved_source.as_deref().unwrap_or(STARTER_DOCUMENT);
        // Keep the final empty line so opening and saving preserves a trailing newline.
        let mut text = TextArea::new(source.split('\n').map(String::from).collect());
        text.set_tab_length(TAB_WIDTH);
        text.set_wrap_mode(WrapMode::WordOrGlyph);
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
            dragging: false,
        })
    }

    pub fn source(&self) -> String {
        self.text.lines().join("\n")
    }

    pub fn is_modified(&self) -> bool {
        self.saved_source.as_deref() != Some(self.source().as_str())
    }

    pub fn apply_preferences(&mut self, preferences: EditorPreferences) {
        self.text.set_wrap_mode(if preferences.soft_wrap {
            WrapMode::WordOrGlyph
        } else {
            WrapMode::None
        });
    }

    pub fn handle_key(&mut self, mut key: KeyEvent) {
        if key.modifiers == (KeyModifiers::ALT | KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Left | KeyCode::Right)
        {
            self.scroll_horizontal(key.code == KeyCode::Left);
            return;
        }

        if key.modifiers.contains(KeyModifiers::CONTROL) {
            if let KeyCode::Char(character) = key.code {
                key.code = KeyCode::Char(character.to_ascii_lowercase());
            }

            match key.code {
                KeyCode::Char('a') => {
                    self.text.select_all();
                    return;
                }
                KeyCode::Char('z') if key.modifiers.contains(KeyModifiers::SHIFT) => {
                    self.text.redo();
                    return;
                }
                KeyCode::Char('z') => {
                    self.text.undo();
                    return;
                }
                KeyCode::Char('y') => {
                    self.text.redo();
                    return;
                }
                KeyCode::Home | KeyCode::End => {
                    // The component uses Ctrl+Alt+Up/Down for document boundaries.
                    key.code = if key.code == KeyCode::Home {
                        KeyCode::Up
                    } else {
                        KeyCode::Down
                    };
                    key.modifiers.insert(KeyModifiers::ALT);
                }
                _ => {}
            }
        } else if key.modifiers.contains(KeyModifiers::ALT)
            && matches!(key.code, KeyCode::Left | KeyCode::Right)
        {
            // Accept terminal Option/Alt arrows as word motions, including selection.
            key.modifiers.remove(KeyModifiers::ALT);
            key.modifiers.insert(KeyModifiers::CONTROL);
        }

        if key.code == KeyCode::Esc {
            self.text.cancel_selection();
        } else {
            self.text.input(key);
        }
    }

    // Adapted from ratatui-textarea 0.9.3 examples/minimal.rs (MIT).
    // See THIRD_PARTY_LICENSES/ratatui-textarea.txt for attribution.
    fn clicked_cursor(&self, area: Rect, column: u16, row: u16) -> Option<CursorMove> {
        let content = self.text.block().map_or(area, |block| block.inner(area));
        if !content.contains(Position::new(column, row)) {
            return None;
        }

        let (scroll_row, scroll_column) = self.text.scroll_offset();
        let display_row = usize::from(row - content.y) + usize::from(scroll_row);
        // Horizontal scrolling also moves the gutter, so subtract it after the offset.
        let display_column = usize::from(column - content.x) + usize::from(scroll_column);
        let text_column = display_column.saturating_sub(usize::from(self.text.line_number_width()));
        let position = self.text.screen_to_data(display_row, text_column);

        Some(CursorMove::Jump(
            u16::try_from(position.0).unwrap_or(u16::MAX),
            u16::try_from(position.1).unwrap_or(u16::MAX),
        ))
    }

    pub fn handle_mouse(&mut self, area: Rect, mouse: MouseEvent) {
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(cursor) = self.clicked_cursor(area, mouse.column, mouse.row) {
                    self.text.cancel_selection();
                    self.text.move_cursor(cursor);
                    self.text.start_selection();
                    self.dragging = true;
                }
            }
            MouseEventKind::Drag(MouseButton::Left) if self.dragging => {
                if let Some(cursor) = self.clicked_cursor(area, mouse.column, mouse.row) {
                    self.text.move_cursor(cursor);
                }
            }
            MouseEventKind::Up(MouseButton::Left) => self.end_drag(),
            MouseEventKind::ScrollLeft => self.scroll_horizontal(true),
            MouseEventKind::ScrollRight => self.scroll_horizontal(false),
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                self.text.input(mouse);
            }
            _ => {}
        }
    }

    pub fn is_dragging(&self) -> bool {
        self.dragging
    }

    pub fn end_drag(&mut self) {
        self.dragging = false;
        if self
            .text
            .selection_range()
            .is_some_and(|(from, to)| from == to)
        {
            self.text.cancel_selection();
        }
    }

    fn scroll_horizontal(&mut self, left: bool) {
        if self.text.wrap_mode() != WrapMode::None {
            return;
        }

        let columns = if left {
            -HORIZONTAL_SCROLL_COLUMNS
        } else {
            HORIZONTAL_SCROLL_COLUMNS
        };
        self.text.scroll((0, columns));
    }

    pub fn replace_source(&mut self, source: &str) -> bool {
        if self.source() == source {
            return false;
        }

        self.text.select_all();
        self.text.insert_str(source);
        true
    }

    pub fn disk_state(&self) -> Result<DiskState> {
        match fs::read_to_string(&self.path) {
            Ok(source) if self.saved_source.as_deref() == Some(source.as_str()) => {
                Ok(DiskState::Unchanged)
            }
            Ok(_) => Ok(DiskState::Changed),
            Err(error) if error.kind() == ErrorKind::NotFound => {
                Ok(if self.saved_source.is_some() {
                    DiskState::Missing
                } else {
                    DiskState::Unchanged
                })
            }
            Err(error) => Err(error).context("Could not read the source on disk"),
        }
    }

    pub fn reload(&mut self) -> Result<()> {
        let source = fs::read_to_string(&self.path)
            .context("Could not reload source; your editor text is preserved")?;
        self.replace_source(&source);
        self.saved_source = Some(source);
        Ok(())
    }

    pub fn recover_missing(&mut self) -> Result<()> {
        use std::io::Write;

        let source = self.source();
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&self.path)
            .context("Source already exists or cannot be restored; nothing was overwritten")?;
        file.write_all(source.as_bytes())?;
        self.saved_source = Some(source);
        Ok(())
    }

    pub fn save(&mut self) -> Result<()> {
        ensure!(
            self.disk_state()? == DiskState::Unchanged,
            "Source changed or disappeared on disk. Use Reload or Restore source in Actions"
        );
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
            let row = usize::from(scroll_row) + usize::from(y);
            for x in 0..content.width {
                let display_column = usize::from(scroll_column) + usize::from(x);
                let Some(column) = display_column.checked_sub(usize::from(number_width)) else {
                    continue;
                };
                let position = self.text.screen_to_data(row, column);
                if focused && position == cursor {
                    continue;
                }

                if let Some(color) = self
                    .highlighting
                    .colors
                    .get(position.0)
                    .and_then(|colors| colors.get(position.1))
                {
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
        editor.handle_mouse(
            Rect::new(0, 0, 40, 4),
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: text_start + u16::from(TAB_WIDTH),
                row: 1,
                modifiers: KeyModifiers::NONE,
            },
        );
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
        editor.apply_preferences(EditorPreferences { soft_wrap: false });
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

#[cfg(test)]
mod external_change_tests {
    use super::*;

    #[test]
    fn external_changes_and_deletions_never_overwrite_unsaved_work() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("main.tex");
        fs::write(&path, "Original")?;
        let mut editor = Editor::open(path.clone())?;
        editor.text.insert_str("Unsaved ");
        fs::write(&path, "External")?;

        assert_eq!(editor.disk_state()?, DiskState::Changed);
        assert!(editor.save().is_err());
        assert_eq!(fs::read_to_string(&path)?, "External");
        assert!(editor.source().contains("Unsaved"));

        editor.reload()?;
        assert_eq!(editor.source(), "External");
        assert!(!editor.is_modified());

        fs::remove_file(&path)?;
        assert_eq!(editor.disk_state()?, DiskState::Missing);
        assert!(editor.save().is_err());
        editor.recover_missing()?;
        assert_eq!(fs::read_to_string(&path)?, "External");
        assert!(editor.recover_missing().is_err());
        Ok(())
    }
}

#[cfg(test)]
mod viewport_tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    fn mouse(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    #[test]
    fn wrapped_highlighting_and_unicode_drag_follow_the_rendered_text_after_resize() -> Result<()> {
        let source = "\\section{Résumé} 李\t% Notes on experience\n";
        let mut editor = Editor::placeholder(PathBuf::from("unused.tex"))?;
        editor.replace_source(source);

        for width in [26, 22, 24] {
            let mut terminal = Terminal::new(TestBackend::new(width, 14))?;
            editor.text.move_cursor(CursorMove::Jump(0, 0));
            let area = Rect::new(0, 0, width, 14);
            terminal.draw(|frame| editor.draw(frame, area, true))?;
            let buffer = terminal.backend().buffer();
            let find = |symbol: &str| {
                buffer
                    .content()
                    .iter()
                    .position(|cell| cell.symbol() == symbol)
                    .map(|index| (index as u16 % width, index as u16 / width))
                    .expect("Expected visible source character")
            };
            let (wide_x, wide_y) = find("李");
            let (comment_x, comment_y) = find("%");
            let (note_x, note_y) = find("N");
            assert!(comment_y > 1, "Comment should be on a continuation row");
            assert_eq!(buffer[(comment_x, comment_y)].fg, theme::MUTED);
            assert_eq!(buffer[(note_x, note_y)].fg, theme::MUTED);

            editor.handle_mouse(
                area,
                mouse(MouseEventKind::Down(MouseButton::Left), wide_x + 1, wide_y),
            );
            assert_eq!(editor.text.cursor(), (0, 17));
            editor.handle_mouse(
                area,
                mouse(MouseEventKind::Up(MouseButton::Left), wide_x + 1, wide_y),
            );
            assert!(!editor.text.is_selecting());

            editor.handle_mouse(
                area,
                mouse(
                    MouseEventKind::Down(MouseButton::Left),
                    comment_x,
                    comment_y,
                ),
            );
            editor.handle_mouse(
                area,
                mouse(MouseEventKind::Drag(MouseButton::Left), note_x, note_y),
            );
            editor.handle_mouse(
                area,
                mouse(MouseEventKind::Up(MouseButton::Left), note_x, note_y),
            );
            editor.text.copy();
            assert_eq!(editor.text.yank_text(), "% ");
            assert_eq!(editor.source(), source);
            editor.text.cancel_selection();
        }

        assert!(editor.text.undo());
        assert!(editor.text.undo());
        assert_eq!(editor.source(), STARTER_DOCUMENT);
        Ok(())
    }

    #[test]
    fn horizontal_pan_and_click_use_scrolled_gutter_coordinates() -> Result<()> {
        let mut editor = Editor::placeholder(PathBuf::from("unused.tex"))?;
        editor.replace_source("\\section{Experience} % Notes about work");
        editor.apply_preferences(EditorPreferences { soft_wrap: false });
        editor.text.move_cursor(CursorMove::Top);
        editor.text.move_cursor(CursorMove::End);
        let area = Rect::new(0, 0, 22, 6);
        let mut terminal = Terminal::new(TestBackend::new(area.width, area.height))?;
        terminal.draw(|frame| editor.draw(frame, area, true))?;
        assert!(editor.text.scroll_offset().1 > 0);
        let buffer = terminal.backend().buffer();
        let index = buffer
            .content()
            .iter()
            .position(|cell| cell.symbol() == "N")
            .unwrap();
        let column = index as u16 % area.width;
        let row = index as u16 / area.width;
        assert_eq!(buffer[(column, row)].fg, theme::MUTED);
        editor.handle_mouse(
            area,
            mouse(MouseEventKind::Down(MouseButton::Left), column, row),
        );
        assert_eq!(editor.text.cursor(), (0, 23));
        editor.end_drag();

        let before = editor.text.scroll_offset().1;
        editor.handle_key(KeyEvent::new(
            KeyCode::Left,
            KeyModifiers::CONTROL | KeyModifiers::ALT,
        ));
        terminal.draw(|frame| editor.draw(frame, area, true))?;
        assert!(editor.text.scroll_offset().1 < before);
        editor.apply_preferences(EditorPreferences::default());
        terminal.draw(|frame| editor.draw(frame, area, true))?;
        assert_eq!(editor.text.scroll_offset().1, 0);
        assert_eq!(editor.source(), "\\section{Experience} % Notes about work");
        Ok(())
    }
}
