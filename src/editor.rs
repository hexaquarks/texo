use std::{fs, io::ErrorKind, path::PathBuf, sync::OnceLock};

use anyhow::{Context, Result, ensure};
use ratatui::{
    Frame,
    crossterm::{
        cursor::SetCursorStyle,
        event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind},
    },
    layout::{Constraint, Layout, Position, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Padding, Paragraph},
};
use ratatui_textarea::{CursorMove, TextArea, WrapMode};

use crate::config::{EditingMode, EditorPreferences};
use syntect::{
    easy::HighlightLines,
    highlighting::{StyleModifier, Theme, ThemeItem},
    parsing::SyntaxSet,
};
use texo::theme;

#[path = "editor_clipboard.rs"]
mod clipboard;
#[path = "editor_search.rs"]
mod search;
#[path = "editor_vim.rs"]
mod vim;

use clipboard::{ClipboardAccess, ClipboardAction, SystemClipboard};
use search::Search;
use vim::{Vim, VimInput};

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
    search: Search,
    clipboard: Box<dyn ClipboardAccess>,
    notice: String,
    vim: Option<Vim>,
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
        text.set_cursor_style(Style::default());
        text.set_selection_style(Style::default().bg(theme::SELECTION));
        text.set_search_style(Style::default().bg(theme::KEYCAP));

        Ok(Self {
            path,
            text,
            saved_source,
            highlighting: Highlighting::new()?,
            dragging: false,
            search: Search::default(),
            clipboard: Box::<SystemClipboard>::default(),
            notice: String::new(),
            vim: None,
        })
    }

    pub fn source(&self) -> String {
        self.text.lines().join("\n")
    }

    pub fn is_modified(&self) -> bool {
        self.saved_source.as_deref() != Some(self.source().as_str())
    }

    pub fn apply_preferences(&mut self, preferences: EditorPreferences) {
        match preferences.mode {
            EditingMode::Vim if self.vim.is_none() => {
                self.text.cancel_selection();
                self.vim = Some(Vim::default());
            }
            EditingMode::Standard if self.vim.is_some() => {
                self.text.cancel_selection();
                self.vim = None;
            }
            _ => {}
        }

        self.text.set_wrap_mode(if preferences.soft_wrap {
            WrapMode::WordOrGlyph
        } else {
            WrapMode::None
        });
    }

    pub fn handle_key(&mut self, mut key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && let KeyCode::Char(character) = key.code
        {
            key.code = KeyCode::Char(character.to_ascii_lowercase());
        }

        if self.search.open {
            self.search.handle_key(key, &mut self.text);
            return;
        }

        self.notice.clear();
        // Ctrl+Shift+F stays available for search; Ctrl+F pages in Vim Normal/Visual.
        let search_shortcut = key.code == KeyCode::Char('f')
            && key.modifiers == (KeyModifiers::CONTROL | KeyModifiers::SHIFT);
        if search_shortcut {
            self.cancel_pending_commands();
            self.search.start(&mut self.text);
            return;
        }
        if key.modifiers == (KeyModifiers::ALT | KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Left | KeyCode::Right)
        {
            self.cancel_pending_commands();
            self.scroll_horizontal(key.code == KeyCode::Left);
            return;
        }

        let insert_clipboard_shortcut = key.code == KeyCode::Insert
            && [
                KeyModifiers::CONTROL,
                KeyModifiers::SHIFT,
                KeyModifiers::ALT,
            ]
            .contains(&key.modifiers);
        let shifted_clipboard_shortcut = (key.code == KeyCode::Delete
            && key.modifiers == KeyModifiers::SHIFT)
            || (key.modifiers == (KeyModifiers::CONTROL | KeyModifiers::SHIFT)
                && matches!(key.code, KeyCode::Char('c' | 'x' | 'v')));
        let clipboard_shortcut = insert_clipboard_shortcut || shifted_clipboard_shortcut;
        if clipboard_shortcut {
            self.cancel_pending_commands();
            if let Some(vim) = &mut self.vim
                && vim.mode == vim::Mode::Visual
            {
                vim.mode = vim::Mode::Normal;
            }
            self.handle_editor_command(key);
            return;
        }

        if let Some(vim) = &mut self.vim {
            match vim.handle_key(key, &mut self.text) {
                VimInput::Consumed => return,
                VimInput::Clipboard(action) => {
                    self.clipboard_action(action);
                    return;
                }
                VimInput::Insert => {}
            }
        }

        if self.handle_editor_command(key) {
            return;
        }

        self.handle_standard_key(key);
    }

    fn handle_editor_command(&mut self, key: KeyEvent) -> bool {
        if key.code == KeyCode::Insert && key.modifiers == KeyModifiers::ALT {
            self.text.paste();
            return true;
        }

        if key.modifiers.contains(KeyModifiers::CONTROL)
            && !key.modifiers.contains(KeyModifiers::ALT)
        {
            match key.code {
                KeyCode::Char('f') => {
                    self.search.start(&mut self.text);
                    return true;
                }
                KeyCode::Char('g') => {
                    self.search.find(
                        &mut self.text,
                        key.modifiers.contains(KeyModifiers::SHIFT),
                        false,
                    );
                    return true;
                }
                KeyCode::Char('c') | KeyCode::Insert => {
                    self.clipboard_action(ClipboardAction::Copy);
                    return true;
                }
                KeyCode::Char('x') => {
                    self.clipboard_action(ClipboardAction::Cut);
                    return true;
                }
                KeyCode::Char('v') => {
                    self.clipboard_action(ClipboardAction::Paste);
                    return true;
                }
                _ => {}
            }
        } else if key.modifiers == KeyModifiers::SHIFT {
            match key.code {
                KeyCode::Delete => {
                    self.clipboard_action(ClipboardAction::Cut);
                    return true;
                }
                KeyCode::Insert => {
                    self.clipboard_action(ClipboardAction::Paste);
                    return true;
                }
                _ => {}
            }
        }

        false
    }

    fn handle_standard_key(&mut self, mut key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) {
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

    pub fn paste_text(&mut self, text: &str) {
        if self.search.open {
            self.search.paste(text, &mut self.text);
        } else {
            // Bracketed paste is literal text in every mode, never a sequence of commands.
            self.cancel_pending_commands();
            self.text.insert_str(text);
        }
    }

    pub fn cancel_pending_commands(&mut self) {
        if let Some(vim) = &mut self.vim {
            vim.cancel_pending(&mut self.text);
        }
    }

    fn clipboard_action(&mut self, action: ClipboardAction) {
        match action {
            ClipboardAction::Copy | ClipboardAction::Cut => {
                if !self
                    .text
                    .selection_range()
                    .is_some_and(|(from, to)| from != to)
                {
                    return;
                }

                if matches!(action, ClipboardAction::Cut) {
                    self.text.cut();
                } else {
                    self.text.copy();
                }

                self.notice = if self.clipboard.write(self.text.yank_text()).is_err() {
                    "System copy failed; internal copy retained (Alt+Insert to paste).".into()
                } else {
                    "Copied to clipboard".into()
                };
            }
            ClipboardAction::Paste => {
                let result = self.clipboard.read();
                match result {
                    Ok(text) => self.text.set_yank_text(text),
                    Err(_) => {
                        self.notice = if self.text.yank_text().is_empty() {
                            "System paste failed; internal copy is empty.".into()
                        } else {
                            "System paste failed; pasted internal copy.".into()
                        };
                    }
                }
                self.text.paste();
            }
        }
    }

    fn draw_status(&mut self, frame: &mut Frame, area: Rect, focused: bool) {
        if area.height < 3 {
            return;
        }

        let footer = Rect::new(
            area.x + theme::PANEL_PADDING,
            area.bottom() - 1,
            area.width.saturating_sub(theme::PANEL_PADDING * 2),
            1,
        );
        if self.search.open {
            self.search.draw(frame, footer, focused);
            return;
        }

        let cursor = self.text.cursor();
        let modified = if self.is_modified() {
            " · unsaved"
        } else {
            ""
        };
        let position = format!("{}:{}{}", cursor.0 + 1, cursor.1 + 1, modified);
        let status = match &self.vim {
            Some(vim) => Line::from(vec![
                Span::styled("VIM", Style::default().fg(theme::GREEN)),
                Span::raw(format!(" {} {position}", vim.label())),
            ]),
            None => Line::from(position),
        };
        let [position, controls] = Layout::horizontal([
            Constraint::Length(status.width() as u16 + 2),
            Constraint::Min(0),
        ])
        .areas(footer);
        frame.render_widget(Paragraph::new(status).style(theme::muted()), position);

        if !self.notice.is_empty() {
            frame.render_widget(
                Paragraph::new(self.notice.as_str()).style(theme::muted()),
                controls,
            );
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
                self.cancel_pending_commands();
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
        let was_dragging = std::mem::take(&mut self.dragging);
        if self
            .text
            .selection_range()
            .is_some_and(|(from, to)| from == to)
        {
            self.text.cancel_selection();
        }
        if was_dragging && let Some(vim) = &mut self.vim {
            vim.finish_mouse_selection(&mut self.text);
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

        self.cancel_pending_commands();
        self.end_drag();
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

    pub fn terminal_cursor_style(&self) -> SetCursorStyle {
        if self.search.open
            || self
                .vim
                .as_ref()
                .is_none_or(|vim| vim.mode == vim::Mode::Insert)
        {
            SetCursorStyle::BlinkingBar
        } else {
            SetCursorStyle::SteadyBlock
        }
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
        frame.render_widget(&self.text, area);
        if focused && !self.search.open {
            crate::terminal::show_text_cursor(frame, &self.text, area);
        }

        self.highlighting.update(self.text.lines());
        self.draw_highlighting(frame, content);
        self.draw_status(frame, area, focused);
    }

    fn draw_highlighting(&self, frame: &mut Frame, content: Rect) {
        let (scroll_row, scroll_column) = self.text.scroll_offset();
        let number_width = self.text.line_number_width();

        // Let the textarea own glyphs, scrolling, tabs, and selection backgrounds.
        // Only recolor the text cells, using its screen mapping for wide Unicode characters.
        for y in 0..content.height {
            let row = usize::from(scroll_row) + usize::from(y);
            for x in 0..content.width {
                let display_column = usize::from(scroll_column) + usize::from(x);
                let Some(column) = display_column.checked_sub(usize::from(number_width)) else {
                    continue;
                };
                let position = self.text.screen_to_data(row, column);
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
        editor.apply_preferences(EditorPreferences {
            soft_wrap: false,
            ..EditorPreferences::default()
        });
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
        editor.apply_preferences(EditorPreferences {
            soft_wrap: false,
            ..EditorPreferences::default()
        });
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

#[cfg(test)]
mod editing_polish_tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    struct UnavailableClipboard;

    impl ClipboardAccess for UnavailableClipboard {
        fn read(&mut self) -> Result<String> {
            anyhow::bail!("No clipboard in this test")
        }

        fn write(&mut self, _text: String) -> Result<()> {
            anyhow::bail!("No clipboard in this test")
        }
    }

    #[test]
    fn unavailable_clipboard_keeps_native_cut_paste_and_history() -> Result<()> {
        let mut editor = Editor::placeholder(PathBuf::from("unused.tex"))?;
        editor.replace_source("Résumé 李\n\n");
        editor.clipboard = Box::new(UnavailableClipboard);
        editor.text.select_all();
        editor.handle_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::SHIFT));
        assert_eq!(editor.source(), "");
        assert!(editor.notice.contains("System copy failed"));
        assert_eq!(editor.text.yank_text(), "Résumé 李\n\n");

        editor.handle_key(KeyEvent::new(KeyCode::Insert, KeyModifiers::SHIFT));
        assert_eq!(editor.source(), "Résumé 李\n\n");
        assert!(editor.notice.contains("pasted internal copy"));
        editor.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL));
        assert_eq!(editor.source(), "");
        editor.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL));
        assert_eq!(editor.source(), "Résumé 李\n\n");
        Ok(())
    }

    #[test]
    fn search_matches_before_wrapped_continuations_survive_redraw_and_resize() -> Result<()> {
        let source = format!("% Résumé 李 {}\n", "experience ".repeat(20));
        let mut editor = Editor::placeholder(PathBuf::from("unused.tex"))?;
        editor.replace_source(&source);
        editor.text.move_cursor(CursorMove::Jump(0, 0));
        editor.handle_key(KeyEvent::new(
            KeyCode::Char('f'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ));
        editor.paste_text("Résumé");

        for width in [40, 24, 60, 16] {
            let mut terminal = Terminal::new(TestBackend::new(width, 8))?;
            terminal.draw(|frame| editor.draw(frame, frame.area(), true))?;
            let buffer = terminal.backend().buffer();
            let highlighted = (1..7).any(|row| {
                (0..width).any(|column| {
                    let cell = &buffer[(column, row)];
                    cell.bg == theme::KEYCAP && cell.fg == theme::MUTED
                })
            });
            assert!(
                highlighted,
                "The leading match should remain highlighted at width {width}"
            );
            assert_eq!(editor.source(), source);
        }

        editor.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(editor.text.search_pattern().is_none());
        Ok(())
    }

    #[test]
    fn literal_search_wraps_matches_highlights_continuations_and_cancels_without_edits()
    -> Result<()> {
        let source = "\\section{Résumé} % [x] and 李 [x]\n";
        let mut editor = Editor::placeholder(PathBuf::from("unused.tex"))?;
        editor.replace_source(source);
        editor.text.move_cursor(CursorMove::Jump(0, 0));
        editor.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL));
        editor.paste_text("[x]");
        assert_eq!(editor.text.cursor(), (0, 19));
        editor.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(editor.text.cursor(), (0, 29));
        editor.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(editor.text.cursor(), (0, 19));
        editor.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT));
        assert_eq!(editor.text.cursor(), (0, 29));

        let mut terminal = Terminal::new(TestBackend::new(24, 10))?;
        terminal.draw(|frame| editor.draw(frame, frame.area(), true))?;
        let buffer = terminal.backend().buffer();
        let matches: Vec<_> = buffer
            .content()
            .iter()
            .enumerate()
            .filter(|(index, cell)| {
                index / 24 > 0
                    && index / 24 < 9
                    && (cell.bg == theme::KEYCAP || cell.bg == theme::ACCENT)
            })
            .collect();
        // The component leaves its source cursor cell unshaded; the terminal caret is in the query.
        assert_eq!(matches.len(), 5);
        assert!(terminal.backend().cursor_visible());
        assert_eq!(terminal.backend().cursor_position().y, 9);
        assert!(
            matches
                .iter()
                .filter(|(_, cell)| cell.bg == theme::KEYCAP)
                .all(|(_, cell)| cell.fg == theme::MUTED)
        );
        assert_eq!(editor.source(), source);

        editor.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(!editor.search.open);
        assert!(editor.text.search_pattern().is_none());
        assert!(editor.text.undo());
        assert!(editor.text.undo());
        assert_eq!(editor.source(), STARTER_DOCUMENT);
        Ok(())
    }
}

#[cfg(test)]
mod vim_navigation_tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn vim_is_optional_and_uses_the_existing_unicode_buffer_and_history() -> Result<()> {
        let mut editor = Editor::placeholder(PathBuf::from("unused.tex"))?;
        editor.replace_source("李 résumé words\nSecond line\n");
        assert!(editor.vim.is_none());
        editor.text.move_cursor(CursorMove::Jump(0, 0));
        editor.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
        editor.apply_preferences(EditorPreferences {
            mode: EditingMode::Vim,
            ..EditorPreferences::default()
        });
        let source = editor.source();
        for character in ['q', 'z', ':'] {
            editor.handle_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
        }
        assert_eq!(editor.source(), source);

        editor.handle_key(KeyEvent::new(KeyCode::Char('A'), KeyModifiers::SHIFT));
        assert!(editor.vim.as_ref().unwrap().mode == vim::Mode::Insert);
        editor.paste_text(" new");
        editor.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        editor.handle_key(KeyEvent::new(KeyCode::Char('0'), KeyModifiers::NONE));
        editor.handle_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE));
        editor.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE));
        assert!(editor.text.is_selecting());
        assert!(editor.vim.as_ref().unwrap().mode == vim::Mode::Visual);

        let mut terminal = Terminal::new(TestBackend::new(60, 10))?;
        terminal.draw(|frame| editor.draw(frame, frame.area(), true))?;
        let visible: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(visible.contains("VISUAL"));
        editor.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(!editor.text.is_selecting());
        editor.apply_preferences(EditorPreferences::default());
        editor.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL));
        assert_eq!(editor.source(), source);
        Ok(())
    }
}

#[cfg(test)]
mod vim_editing_tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    struct UnavailableClipboard;

    impl ClipboardAccess for UnavailableClipboard {
        fn read(&mut self) -> Result<String> {
            anyhow::bail!("Clipboard unavailable in this test")
        }

        fn write(&mut self, _text: String) -> Result<()> {
            anyhow::bail!("Clipboard unavailable in this test")
        }
    }

    fn vim_editor(path: PathBuf) -> Result<Editor> {
        let mut editor = Editor::open(path)?;
        editor.apply_preferences(EditorPreferences {
            mode: EditingMode::Vim,
            ..EditorPreferences::default()
        });
        editor.clipboard = Box::new(UnavailableClipboard);
        Ok(editor)
    }

    fn commands(editor: &mut Editor, keys: &str) {
        for character in keys.chars() {
            editor.handle_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
        }
    }

    #[test]
    fn wrapped_line_operators_share_standard_history_and_preserve_saved_newlines() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("resume.tex");
        let first_line = "\\section{Résumé 李 and a very long description of experience}";
        let source = format!("{first_line}\nSecond\n\n");
        fs::write(&path, &source)?;
        let mut editor = vim_editor(path.clone())?;
        let mut terminal = Terminal::new(TestBackend::new(24, 12))?;
        terminal.draw(|frame| editor.draw(frame, frame.area(), true))?;

        editor.text.move_cursor(CursorMove::Jump(0, 12));
        commands(&mut editor, "dd");
        assert_eq!(editor.source(), "Second\n\n");
        assert_eq!(editor.text.yank_text(), format!("{first_line}\n"));
        assert!(editor.notice.contains("System copy failed"));
        commands(&mut editor, "u");
        assert_eq!(editor.source(), source);
        editor.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));
        assert_eq!(editor.source(), "Second\n\n");
        editor.apply_preferences(EditorPreferences::default());
        editor.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL));
        assert_eq!(editor.source(), source);

        editor.apply_preferences(EditorPreferences {
            mode: EditingMode::Vim,
            ..EditorPreferences::default()
        });
        editor.text.move_cursor(CursorMove::Jump(0, 0));
        commands(&mut editor, "yyp");
        assert_eq!(editor.source(), format!("{first_line}\n{source}"));
        commands(&mut editor, "u");
        editor.text.move_cursor(CursorMove::Jump(0, 0));
        commands(&mut editor, "cc");
        assert!(editor.vim.as_ref().unwrap().mode == vim::Mode::Insert);
        editor.paste_text("Replacement\n");
        editor.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        editor.save()?;
        assert_eq!(fs::read_to_string(path)?, "Replacement\nSecond\n\n");
        Ok(())
    }

    #[test]
    fn unicode_word_changes_and_reversed_visual_yanks_use_native_character_ranges() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("resume.tex");
        fs::write(&path, "李 résumé words\n")?;
        let mut editor = vim_editor(path)?;
        editor.text.move_cursor(CursorMove::Jump(0, 2));
        commands(&mut editor, "cw");
        editor.paste_text("CV ");
        editor.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(editor.source(), "李 CV words\n");

        editor.text.move_cursor(CursorMove::Jump(0, 3));
        commands(&mut editor, "vhy");
        assert_eq!(editor.text.yank_text(), "CV");
        assert!(!editor.text.is_selecting());
        editor.text.move_cursor(CursorMove::End);
        commands(&mut editor, "p");
        assert_eq!(editor.source(), "李 CV wordsCV\n");
        commands(&mut editor, "u");
        editor.text.move_cursor(CursorMove::Jump(0, 2));
        commands(&mut editor, "de");
        assert_eq!(editor.source(), "李  words\n");
        Ok(())
    }

    #[test]
    fn invalid_and_cancelled_operators_never_turn_commands_into_source_text() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("resume.tex");
        fs::write(&path, "Résumé 李\n")?;
        let mut editor = vim_editor(path)?;
        let source = editor.source();
        commands(&mut editor, "dgi");
        assert_eq!(editor.source(), source);
        assert!(editor.vim.as_ref().unwrap().mode == vim::Mode::Normal);
        assert!(!editor.text.is_selecting());
        commands(&mut editor, "d");
        editor.cancel_pending_commands();
        commands(&mut editor, "w");
        assert_eq!(editor.source(), source);
        commands(&mut editor, "c");
        editor.paste_text("?\n");
        assert!(editor.source().contains("?\n"));
        assert!(editor.vim.as_ref().unwrap().mode == vim::Mode::Normal);
        Ok(())
    }
}

#[cfg(test)]
mod caret_tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    fn footer(terminal: &Terminal<TestBackend>) -> String {
        let buffer = terminal.backend().buffer();
        let row = buffer.area.bottom() - 1;
        (0..buffer.area.width)
            .map(|column| buffer[(column, row)].symbol())
            .collect()
    }

    #[test]
    fn status_and_terminal_caret_follow_standard_and_vim_modes() -> Result<()> {
        let mut editor = Editor::placeholder(PathBuf::from("unused.tex"))?;
        editor.replace_source("李\t\\section{Résumé}");
        editor.text.move_cursor(CursorMove::Jump(0, 3));
        let mut terminal = Terminal::new(TestBackend::new(60, 8))?;
        terminal.draw(|frame| editor.draw(frame, frame.area(), true))?;
        assert!(!footer(&terminal).contains("STANDARD"));
        assert!(!footer(&terminal).contains("VIM"));
        assert!(footer(&terminal).contains("1:4"));
        assert_eq!(editor.terminal_cursor_style(), SetCursorStyle::BlinkingBar);
        assert!(terminal.backend().cursor_visible());
        let position = terminal.backend().cursor_position();
        assert_eq!(
            position,
            Position::new(
                theme::PANEL_PADDING + editor.text.line_number_width() + u16::from(TAB_WIDTH) + 1,
                1
            )
        );
        assert_eq!(
            terminal.backend().buffer()[position].fg,
            theme::SYNTAX_COMMAND
        );
        assert_eq!(terminal.backend().buffer()[position].bg, theme::EDITOR);

        editor.apply_preferences(EditorPreferences {
            mode: EditingMode::Vim,
            ..EditorPreferences::default()
        });
        terminal.draw(|frame| editor.draw(frame, frame.area(), true))?;
        assert!(footer(&terminal).contains("VIM NORMAL"));
        assert_eq!(editor.terminal_cursor_style(), SetCursorStyle::SteadyBlock);
        editor.handle_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE));
        terminal.draw(|frame| editor.draw(frame, frame.area(), true))?;
        assert!(footer(&terminal).contains("VIM INSERT"));
        assert_eq!(editor.terminal_cursor_style(), SetCursorStyle::BlinkingBar);
        editor.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        editor.handle_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE));
        terminal.draw(|frame| editor.draw(frame, frame.area(), true))?;
        assert!(footer(&terminal).contains("VIM VISUAL"));
        assert_eq!(editor.terminal_cursor_style(), SetCursorStyle::SteadyBlock);
        terminal.draw(|frame| editor.draw(frame, frame.area(), false))?;
        assert!(!terminal.backend().cursor_visible());
        Ok(())
    }

    #[test]
    fn caret_stays_on_rendered_unicode_after_wrapping_scrolling_and_resize() -> Result<()> {
        let mut editor = Editor::placeholder(PathBuf::from("unused.tex"))?;
        let source = format!("李\t{}§XY", "experience ".repeat(12));
        editor.replace_source(&source);
        let target = u16::try_from(
            source
                .chars()
                .position(|character| character == '§')
                .unwrap(),
        )?;
        editor.text.move_cursor(CursorMove::Jump(0, target));

        for (width, soft_wrap) in [(24, true), (40, true), (18, true), (24, false)] {
            editor.apply_preferences(EditorPreferences {
                soft_wrap,
                ..EditorPreferences::default()
            });
            let mut terminal = Terminal::new(TestBackend::new(width, 6))?;
            terminal.draw(|frame| editor.draw(frame, frame.area(), true))?;
            let position = terminal.backend().cursor_position();
            assert!(terminal.backend().cursor_visible());
            assert_eq!(terminal.backend().buffer()[position].symbol(), "§");
            if soft_wrap {
                assert!(editor.text.scroll_offset().0 > 0);
            } else {
                assert!(editor.text.scroll_offset().1 > 0);
            }
            assert_eq!(editor.source(), source);
        }
        Ok(())
    }
}
