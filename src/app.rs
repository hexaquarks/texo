use std::{path::PathBuf, time::Duration};

use anyhow::Context;

use ratatui::{
    DefaultTerminal, Frame,
    crossterm::event::{
        self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
    },
    layout::{Constraint, Layout, Position, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};
use texo::{preview::Preview, theme};

use crate::{editor::Editor, explorer::Explorer, render_job::RenderJob};

const EVENT_POLL_INTERVAL: Duration = Duration::from_millis(100);
const MIN_TERMINAL_WIDTH: u16 = 100;
const MIN_TERMINAL_HEIGHT: u16 = 16;
const TOOLBAR_HEIGHT: u16 = 2;
const FOOTER_HEIGHT: u16 = 2;
const RENDER_BUTTON_WIDTH: u16 = 20;
const EXPLORER_WIDTH: u16 = 30;
const DIAGNOSTICS_HEIGHT: u16 = 8;
const DIAGNOSTIC_SCROLL_LINES: u16 = 4;

#[derive(Clone, Copy, PartialEq)]
enum Focus {
    Explorer,
    Editor,
    Render,
}

struct App {
    editor: Editor,
    explorer: Explorer,
    explorer_visible: bool,
    preview: Preview,
    job: Option<RenderJob>,
    rendered_source: Option<String>,
    status: String,
    diagnostics: Option<String>,
    diagnostic_scroll: u16,
    focus: Focus,
    confirm_quit: bool,
}

pub fn run(terminal: &mut DefaultTerminal, editor: Editor) -> anyhow::Result<()> {
    let mut app = App::new(editor, Preview::detect())?;

    loop {
        app.receive_render();
        terminal.draw(|frame| app.draw(frame))?;

        if event::poll(EVENT_POLL_INTERVAL)? {
            let area = terminal.get_frame().area();
            let explorer_was_visible = app.explorer_visible;
            if app.handle_event(event::read()?, area) {
                return Ok(());
            }
            if explorer_was_visible != app.explorer_visible {
                // Repaint cells formerly occupied by the terminal's PDF image placement.
                terminal.clear()?;
            }
        }
    }
}

impl App {
    fn new(editor: Editor, preview: Preview) -> anyhow::Result<Self> {
        let root = editor
            .path
            .parent()
            .context("The document needs a workspace directory")?
            .to_path_buf();
        let explorer = Explorer::new(root, &editor.path)?;
        Ok(Self {
            editor,
            explorer,
            explorer_visible: true,
            preview,
            job: None,
            rendered_source: None,
            status: "Edit your document, then select Render or press Ctrl+R.".into(),
            diagnostics: None,
            diagnostic_scroll: 0,
            focus: Focus::Editor,
            confirm_quit: false,
        })
    }

    fn start_render(&mut self) {
        if self.job.is_some() {
            return;
        }

        match RenderJob::start(self.editor.source(), self.editor.path.clone()) {
            Ok(job) => {
                self.job = Some(job);
                self.diagnostics = None;
                self.status = "Rendering… You can continue editing.".into();
            }
            Err(error) => self.report_error(error),
        }
    }

    fn receive_render(&mut self) {
        let Some(result) = self.job.as_ref().and_then(RenderJob::poll) else {
            return;
        };
        self.job = None;

        match result {
            Ok(document) => match self.preview.set_pages(document.pages) {
                Ok(()) => {
                    self.rendered_source = Some(document.source);
                    self.status = format!("Rendered {}", document.pdf_path.display());
                    self.diagnostics = None;
                    self.refresh_explorer();
                }
                Err(error) => self.report_error(error),
            },
            Err(error) => self.report_error(error),
        }
    }

    fn report_error(&mut self, error: anyhow::Error) {
        self.status = "Operation failed. See diagnostics below; PgUp/PgDn to scroll.".into();
        self.diagnostics = Some(format!("{error:#}"));
        self.diagnostic_scroll = 0;
    }

    fn handle_event(&mut self, event: Event, area: Rect) -> bool {
        match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => return self.handle_key(key),
            Event::Paste(text) if self.focus == Focus::Editor => {
                self.editor.text.insert_str(text);
                self.confirm_quit = false;
            }
            Event::Mouse(mouse) => {
                let layout =
                    WorkspaceLayout::new(area, self.diagnostics.is_some(), self.explorer_visible);
                let position = Position::new(mouse.column, mouse.row);
                if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
                    self.confirm_quit = false;
                    if layout.render_button.contains(position) {
                        self.focus = Focus::Render;
                        self.start_render();
                    } else if layout.explorer.contains(position) {
                        self.focus = Focus::Explorer;
                        if self.explorer.select_at(layout.explorer, mouse.row) {
                            self.activate_explorer();
                        }
                    } else if layout.editor.contains(position) {
                        self.focus = Focus::Editor;
                    }
                } else if layout.editor.contains(position) {
                    self.editor.text.input(mouse);
                } else if layout.explorer.contains(position) {
                    match mouse.kind {
                        MouseEventKind::ScrollDown => self.explorer.move_selection(true),
                        MouseEventKind::ScrollUp => self.explorer.move_selection(false),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
        false
    }

    fn handle_key(&mut self, key: KeyEvent) -> bool {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('q') {
            if !self.editor.is_modified() || self.confirm_quit {
                return true;
            }
            self.confirm_quit = true;
            self.status = "Unsaved changes. Ctrl+Q again to discard, or Ctrl+S to save.".into();
            return false;
        }
        self.confirm_quit = false;

        if key.modifiers.contains(KeyModifiers::CONTROL) {
            self.handle_control_key(key);
        } else if key.modifiers.contains(KeyModifiers::ALT) {
            self.handle_alt_key(key);
        } else {
            self.handle_navigation_key(key);
        }
        false
    }

    fn handle_control_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('r') => self.start_render(),
            KeyCode::Char('b') => {
                self.explorer_visible = !self.explorer_visible;
                if !self.explorer_visible && self.focus == Focus::Explorer {
                    self.focus = Focus::Editor;
                }
            }
            KeyCode::Char('s') => match self.editor.save() {
                Ok(()) => {
                    self.status = "Saved TeX document.".into();
                    self.refresh_explorer();
                    self.explorer.select_path(&self.editor.path);
                }
                Err(error) => self.report_error(error),
            },
            KeyCode::Char('z') => {
                self.editor.text.undo();
            }
            KeyCode::Char('y') => {
                self.editor.text.redo();
            }
            _ if self.focus == Focus::Editor => {
                self.editor.text.input(key);
            }
            _ => {}
        }
    }

    fn handle_alt_key(&mut self, key: KeyEvent) {
        let result = match key.code {
            KeyCode::Left => self.preview.previous_page(),
            KeyCode::Right => self.preview.next_page(),
            _ => {
                if self.focus == Focus::Editor {
                    self.editor.text.input(key);
                }
                Ok(())
            }
        };
        if let Err(error) = result {
            self.report_error(error);
        }
    }

    fn handle_navigation_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::F(5) => self.refresh_explorer(),
            KeyCode::F(6) => {
                self.focus = match self.focus {
                    Focus::Editor => Focus::Render,
                    Focus::Render if self.explorer_visible => Focus::Explorer,
                    Focus::Render | Focus::Explorer => Focus::Editor,
                };
            }
            KeyCode::Esc if self.diagnostics.is_some() => self.diagnostics = None,
            KeyCode::PageUp if self.diagnostics.is_some() => {
                self.diagnostic_scroll = self
                    .diagnostic_scroll
                    .saturating_sub(DIAGNOSTIC_SCROLL_LINES);
            }
            KeyCode::PageDown if self.diagnostics.is_some() => {
                self.diagnostic_scroll = self
                    .diagnostic_scroll
                    .saturating_add(DIAGNOSTIC_SCROLL_LINES);
            }
            KeyCode::Enter | KeyCode::Char(' ') if self.focus == Focus::Render => {
                self.start_render()
            }
            _ if self.focus == Focus::Explorer => self.handle_explorer_key(key),
            _ if self.focus == Focus::Editor => {
                self.editor.text.input(key);
            }
            _ => {}
        }
    }

    fn handle_explorer_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.explorer.move_selection(false),
            KeyCode::Down | KeyCode::Char('j') => self.explorer.move_selection(true),
            KeyCode::Enter => self.activate_explorer(),
            KeyCode::Right => {
                if let Err(error) = self.explorer.expand_selected() {
                    self.report_error(error);
                }
            }
            KeyCode::Left => {
                if let Err(error) = self.explorer.collapse_selected() {
                    self.report_error(error);
                }
            }
            _ => {}
        }
    }

    fn activate_explorer(&mut self) {
        match self.explorer.activate() {
            Ok(Some(path)) => self.open_document(path),
            Ok(None) => {}
            Err(error) => self.report_error(error),
        }
    }

    fn refresh_explorer(&mut self) {
        if let Err(error) = self.explorer.refresh() {
            self.report_error(error);
        }
    }

    fn open_document(&mut self, path: PathBuf) {
        if path == self.editor.path {
            self.focus = Focus::Editor;
            return;
        }
        if self.editor.is_modified() {
            self.status = "Save your changes with Ctrl+S before opening another document.".into();
            return;
        }
        if self.job.is_some() {
            self.status =
                "Wait for the current render to finish before switching documents.".into();
            return;
        }
        if path.extension().is_none_or(|extension| extension != "tex") {
            self.status =
                "Select a .tex document to edit. Other files are shown for context.".into();
            return;
        }
        match Editor::open(path) {
            Ok(editor) => {
                self.editor = editor;
                self.preview.clear();
                self.rendered_source = None;
                self.diagnostics = None;
                self.focus = Focus::Editor;
                self.status = "Document opened. Ctrl+R to render.".into();
            }
            Err(error) => self.report_error(error),
        }
    }

    fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();
        frame.render_widget(Block::default().style(theme::text()), area);

        if area.width < MIN_TERMINAL_WIDTH || area.height < MIN_TERMINAL_HEIGHT {
            frame.render_widget(
                Paragraph::new(format!(
                    "Resize to at least {MIN_TERMINAL_WIDTH}×{MIN_TERMINAL_HEIGHT}. Ctrl+Q quits."
                ))
                .wrap(Wrap { trim: false }),
                area,
            );
            return;
        }

        let layout = WorkspaceLayout::new(area, self.diagnostics.is_some(), self.explorer_visible);
        self.draw_toolbar(frame, &layout);
        if self.explorer_visible {
            self.explorer.draw(
                frame,
                layout.explorer,
                &self.editor.path,
                self.focus == Focus::Explorer,
            );
        }
        self.editor
            .draw(frame, layout.editor, self.focus == Focus::Editor);
        self.preview.draw(frame, layout.preview);
        self.draw_footer(frame, &layout);
    }

    fn draw_toolbar(&self, frame: &mut Frame, layout: &WorkspaceLayout) {
        let stale = self
            .rendered_source
            .as_ref()
            .is_some_and(|source| *source != self.editor.source());
        let preview_status = if stale {
            "  ·  Preview out of date"
        } else {
            ""
        };
        let heading = Paragraph::new(format!(
            "  {}  —  texo{preview_status}",
            self.explorer.name()
        ))
        .style(theme::muted())
        .block(
            Block::default()
                .borders(Borders::BOTTOM)
                .border_style(Style::default().fg(theme::DIVIDER)),
        );
        frame.render_widget(heading, layout.heading);

        let label = if self.job.is_some() {
            "Rendering…"
        } else {
            "Render  Ctrl+R"
        };
        let foreground = if self.job.is_some() {
            theme::MUTED
        } else {
            theme::TEXT
        };
        let background = if self.focus == Focus::Render {
            theme::SELECTION
        } else {
            theme::BUTTON
        };
        frame.render_widget(
            Paragraph::new(label)
                .centered()
                .style(Style::default().fg(foreground).bg(background)),
            layout.render_button,
        );
    }

    fn draw_footer(&self, frame: &mut Frame, layout: &WorkspaceLayout) {
        if let Some(diagnostics) = &self.diagnostics {
            let log = Paragraph::new(diagnostics.as_str())
                .block(Block::default().title(" Build diagnostics · PgUp/PgDn scroll · Esc close "))
                .style(Style::default().fg(theme::ERROR).bg(theme::ELEVATED))
                .wrap(Wrap { trim: false })
                .scroll((self.diagnostic_scroll, 0));
            frame.render_widget(log, layout.diagnostics);
        }

        let footer = vec![
            Line::styled(self.status.as_str(), theme::muted()),
            Line::from(vec![
                Span::styled("Ctrl+B", Style::default().fg(theme::ACCENT)),
                Span::styled(" explorer     ", theme::muted()),
                Span::styled("Ctrl+R", Style::default().fg(theme::ACCENT)),
                Span::styled(" render     ", theme::muted()),
                Span::styled("F6", Style::default().fg(theme::ACCENT)),
                Span::styled(" focus     ", theme::muted()),
                Span::styled("Ctrl+Q", Style::default().fg(theme::ACCENT)),
                Span::styled(" quit", theme::muted()),
            ]),
        ];
        frame.render_widget(Paragraph::new(footer), layout.footer);
    }
}

struct WorkspaceLayout {
    explorer: Rect,
    heading: Rect,
    render_button: Rect,
    editor: Rect,
    preview: Rect,
    diagnostics: Rect,
    footer: Rect,
}

impl WorkspaceLayout {
    fn new(area: Rect, show_diagnostics: bool, show_explorer: bool) -> Self {
        let diagnostics_height = if show_diagnostics {
            DIAGNOSTICS_HEIGHT
        } else {
            0
        };
        let [toolbar, body, diagnostics, footer] = Layout::vertical([
            Constraint::Length(TOOLBAR_HEIGHT),
            Constraint::Min(1),
            Constraint::Length(diagnostics_height),
            Constraint::Length(FOOTER_HEIGHT),
        ])
        .areas(area);
        let [heading, render_button] =
            Layout::horizontal([Constraint::Min(1), Constraint::Length(RENDER_BUTTON_WIDTH)])
                .areas(toolbar);
        let sidebar_width = if show_explorer { EXPLORER_WIDTH } else { 0 };
        let [explorer, editor, preview] = Layout::horizontal([
            Constraint::Length(sidebar_width),
            Constraint::Fill(1),
            Constraint::Fill(1),
        ])
        .areas(body);

        Self {
            explorer,
            heading,
            render_button,
            editor,
            preview,
            diagnostics,
            footer,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    fn application() -> anyhow::Result<(tempfile::TempDir, App)> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("resume.tex");
        std::fs::write(&path, "Original document")?;
        let editor = Editor::open(path)?;
        Ok((directory, App::new(editor, Preview::default())?))
    }

    #[test]
    fn typing_q_edits_and_modified_documents_require_quit_confirmation() -> anyhow::Result<()> {
        let (_directory, mut app) = application()?;
        assert!(!app.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)));
        assert!(app.editor.source().starts_with('q'));
        let quit = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL);
        assert!(!app.handle_key(quit));
        assert!(app.handle_key(quit));
        Ok(())
    }

    #[test]
    fn pane_focus_prevents_button_keystrokes_from_editing_text() -> anyhow::Result<()> {
        let (_directory, mut app) = application()?;
        let original = app.editor.source();
        app.handle_key(KeyEvent::new(KeyCode::F(6), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
        assert_eq!(app.editor.source(), original);
        app.handle_key(KeyEvent::new(KeyCode::F(6), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::F(6), KeyModifiers::NONE));
        app.handle_event(
            Event::Paste("Pasted\ntext".into()),
            Rect::new(0, 0, 100, 30),
        );
        assert!(app.editor.source().starts_with("Pasted\ntext"));
        Ok(())
    }

    #[test]
    fn switching_files_requires_saving_and_clears_the_previous_preview() -> anyhow::Result<()> {
        let (directory, mut app) = application()?;
        let original_path = app.editor.path.clone();
        let second = directory.path().join("second.tex");
        std::fs::write(&second, "Second document")?;
        app.editor.text.insert_str("Unsaved edit ");
        app.open_document(second.clone());
        assert_eq!(app.editor.path, original_path);
        assert!(app.editor.source().contains("Unsaved edit"));

        app.editor.save()?;
        app.rendered_source = Some(app.editor.source());
        app.open_document(second.clone());
        assert_eq!(app.editor.path, second);
        assert_eq!(app.editor.source(), "Second document");
        assert!(app.rendered_source.is_none());
        Ok(())
    }

    #[test]
    fn layout_renders_both_panes_and_handles_small_terminals() -> anyhow::Result<()> {
        let (_directory, mut app) = application()?;
        let mut terminal = Terminal::new(TestBackend::new(100, 30))?;
        terminal.draw(|frame| app.draw(frame))?;
        let buffer = terminal.backend().buffer();
        let text: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
        assert!(text.contains("Render  Ctrl+R"));
        assert!(text.contains("PDF preview"));
        assert!(text.contains("Original document"));

        let mut small_terminal = Terminal::new(TestBackend::new(30, 8))?;
        small_terminal.draw(|frame| app.draw(frame))?;
        let text: String = small_terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains("Resize to at least"));
        Ok(())
    }
}
