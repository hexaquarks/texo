use ratatui::{
    Frame,
    crossterm::event::{Event, KeyCode, KeyModifiers, MouseButton, MouseEventKind},
    layout::{Constraint, Layout, Margin, Position, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
};
use texo::theme;

use super::App;

const HELP_WIDTH: u16 = 90;
const HELP_HEIGHT: u16 = 34;
const KEY_COLUMN_WIDTH: usize = 28;
const MODAL_MARGIN: u16 = 1;
const HEADER_AND_FOOTER_ROWS: u16 = 4;
const SPACIOUS_HEIGHT: u16 = 12;
const WHEEL_ROWS: usize = 3;

impl App {
    fn shortcut_bindings(&self) -> Vec<(&'static str, &'static str, &'static str)> {
        let mut bindings = vec![
            ("General", "?", "Keybindings"),
            ("General", "s", "Settings"),
            ("Panes", "h/j/k/l", "Select pane"),
            ("Panes", "H/J/K/L", "Resize pane"),
            ("Panes", "c", "Cycle pane"),
            ("Panes", "u", "Expand column"),
            ("Panes", "z", "Zoom pane"),
            ("Panes", "t", "Toggle tree"),
            ("Resumes", "n", "New resume"),
            ("Resumes", "i", "Import"),
            ("Resumes", "r", "Resumes"),
            ("Resumes", "y", "History"),
            ("Resumes", "v", "Refresh"),
            ("Document", "b", "Build PDF"),
            ("Document", "w", "Save"),
            ("Document", "e", "Export PDF"),
        ];

        if self.session.proposal_review.is_some() {
            bindings.extend([
                ("Review", "a", "Accept"),
                ("Review", "m", "Modify"),
                ("Review", "d", "Discard"),
            ]);
        }
        if self.session.diagnostics.is_some() {
            bindings.extend([
                ("Errors", "G", "Go to error"),
                ("Errors", "F", "Ask AI"),
                ("Errors", "R", "Download & retry"),
                ("Errors", "E", "Error details"),
            ]);
        }
        if self.session.build_diagnostics.is_some() {
            bindings.extend([
                ("Errors", "D", "Toggle errors"),
                ("Errors", "[ / ]", "Previous / next error"),
            ]);
        }

        bindings
    }

    fn shortcut_lines(&self, width: u16) -> Vec<Line<'static>> {
        let query = self.shortcut_query.to_lowercase();
        let key_width = KEY_COLUMN_WIDTH.min(usize::from(width / 2));
        let mut lines = Vec::new();
        let mut previous_group = "";

        for (group, key, label) in self.shortcut_bindings() {
            let shortcut = format!("prefix+{key}");
            let searchable = format!("{group} {shortcut} {label}").to_lowercase();
            if !searchable.contains(&query) {
                continue;
            }

            if group != previous_group {
                if !lines.is_empty() {
                    lines.push(Line::default());
                }
                lines.push(Line::styled(group, theme::dialog_title()));
                previous_group = group;
            }
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{shortcut:<key_width$}"),
                    Style::default().fg(theme::LAVENDER),
                ),
                Span::raw(label),
            ]));
        }

        if lines.is_empty() {
            lines.push(Line::styled("No matching shortcuts", theme::muted()));
        }
        lines
    }

    pub(super) fn shortcut_modal_area(&self, area: Rect) -> Rect {
        let available = area.inner(Margin::new(MODAL_MARGIN, MODAL_MARGIN));
        let width = available.width.min(HELP_WIDTH);
        let height = available.height.min(HELP_HEIGHT);
        Rect::new(
            area.x + (area.width - width) / 2,
            area.y + (area.height - height) / 2,
            width,
            height,
        )
    }

    fn shortcut_layout(&self, area: Rect) -> [Rect; 7] {
        let inner = area.inner(Margin::new(2, 1));
        let header_height = u16::from(inner.height > HEADER_AND_FOOTER_ROWS);
        let spacing = u16::from(area.height >= SPACIOUS_HEIGHT);
        Layout::vertical([
            Constraint::Length(header_height),
            Constraint::Length(header_height),
            Constraint::Length(header_height),
            Constraint::Length(spacing),
            Constraint::Min(0),
            Constraint::Length(spacing),
            Constraint::Length(header_height),
        ])
        .areas(inner)
    }

    fn shortcut_scroll_limit(&self, area: Rect) -> usize {
        let modal = self.shortcut_modal_area(area);
        let [_, _, _, _, content, _, _] = self.shortcut_layout(modal);
        self.shortcut_lines(content.width)
            .len()
            .saturating_sub(usize::from(content.height))
    }

    pub(super) fn handle_shortcut_help(&mut self, event: Event, area: Rect) {
        let modal = self.shortcut_modal_area(area);
        let [_, _, _, _, content, _, _] = self.shortcut_layout(modal);
        let page = usize::from(content.height.max(1));
        let limit = self.shortcut_scroll_limit(area);
        self.shortcut_scroll = self.shortcut_scroll.min(limit);

        if let Event::Key(key) = event
            && self.shortcut_searching
        {
            match key.code {
                KeyCode::Esc => {
                    self.shortcut_searching = false;
                    self.shortcut_query.clear();
                }
                KeyCode::Enter => self.shortcut_searching = false,
                KeyCode::Backspace => {
                    self.shortcut_query.pop();
                }
                KeyCode::Char(character)
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    self.shortcut_query.push(character);
                }
                _ => {}
            }
            self.shortcut_scroll = 0;
            return;
        }

        match event {
            Event::Key(key) => match key.code {
                KeyCode::Esc | KeyCode::Enter | KeyCode::Char('?') => self.shortcuts_open = false,
                KeyCode::Char('/') => self.shortcut_searching = true,
                KeyCode::Up | KeyCode::Char('k') => {
                    self.shortcut_scroll = self.shortcut_scroll.saturating_sub(1);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.shortcut_scroll = self.shortcut_scroll.saturating_add(1).min(limit);
                }
                KeyCode::PageUp => self.shortcut_scroll = self.shortcut_scroll.saturating_sub(page),
                KeyCode::PageDown => {
                    self.shortcut_scroll = self.shortcut_scroll.saturating_add(page).min(limit);
                }
                KeyCode::Home => self.shortcut_scroll = 0,
                KeyCode::End => self.shortcut_scroll = limit,
                _ => {}
            },
            Event::Mouse(mouse) => match mouse.kind {
                MouseEventKind::ScrollUp => {
                    self.shortcut_scroll = self.shortcut_scroll.saturating_sub(WHEEL_ROWS);
                }
                MouseEventKind::ScrollDown => {
                    self.shortcut_scroll =
                        self.shortcut_scroll.saturating_add(WHEEL_ROWS).min(limit);
                }
                MouseEventKind::Down(MouseButton::Left) => {
                    let position = Position::new(mouse.column, mouse.row);
                    if !modal.contains(position) {
                        self.shortcuts_open = false;
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }

    pub(super) fn draw_shortcut_help(&self, frame: &mut Frame) {
        let area = self.shortcut_modal_area(frame.area());
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(theme::dialog_title())
            .style(theme::dialog());
        let [title, prefix, search, _, content, _, footer] = self.shortcut_layout(area);

        frame.render_widget(Clear, area);
        frame.render_widget(block, area);
        frame.render_widget(
            Paragraph::new("Keybindings").style(theme::dialog_title()),
            title,
        );
        frame.render_widget(
            Paragraph::new(format!("Prefix: {}", self.prefix.label())).style(theme::muted()),
            prefix,
        );
        let search_text = if self.shortcut_searching || !self.shortcut_query.is_empty() {
            format!("/ {}", self.shortcut_query)
        } else {
            "/ Search commands or shortcuts".into()
        };
        frame.render_widget(Paragraph::new(search_text).style(theme::muted()), search);

        let limit = self.shortcut_scroll_limit(frame.area());
        let scroll = self.shortcut_scroll.min(limit);
        frame.render_widget(
            Paragraph::new(self.shortcut_lines(content.width)).scroll((scroll as u16, 0)),
            content,
        );
        let hint = if self.shortcut_searching {
            "Enter Apply · Esc Clear"
        } else {
            "/ Search · ↑/↓ Scroll · Esc Close"
        };
        frame.render_widget(Paragraph::new(hint).style(theme::muted()), footer);

        if limit > 0 {
            let mut state = ScrollbarState::new(limit + 1).position(scroll);
            frame.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::VerticalRight),
                Rect::new(area.right().saturating_sub(1), content.y, 1, content.height),
                &mut state,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Focus, test_support::application};
    use super::*;
    use ratatui::{
        Terminal,
        backend::TestBackend,
        crossterm::event::{KeyEvent, KeyModifiers},
    };

    fn key(app: &mut App, code: KeyCode, modifiers: KeyModifiers, area: Rect) {
        app.handle_event(Event::Key(KeyEvent::new(code, modifiers)), area);
    }

    #[test]
    fn footer_is_identical_with_the_prefix_armed_at_every_width() -> anyhow::Result<()> {
        for (width, height) in [(160, 45), (80, 30), (40, 12), (1, 1)] {
            let (_directory, mut app) = application()?;
            let area = Rect::new(0, 0, width, height);
            let mut terminal = Terminal::new(TestBackend::new(width, height))?;
            terminal.draw(|frame| app.draw(frame))?;
            let footer = |terminal: &Terminal<TestBackend>| {
                (0..width)
                    .map(|x| terminal.backend().buffer()[(x, height - 1)].clone())
                    .collect::<Vec<_>>()
            };
            let before = footer(&terminal);

            key(&mut app, KeyCode::Char('x'), KeyModifiers::CONTROL, area);
            assert!(app.prefix.armed);
            terminal.draw(|frame| app.draw(frame))?;
            assert_eq!(footer(&terminal), before);
            if width >= 40 {
                let text: String = before.iter().map(|cell| cell.symbol()).collect();
                assert!(text.contains("Ctrl-x ?"));
            }
        }
        Ok(())
    }

    #[test]
    fn help_uses_configured_prefix_and_preserves_editor_chat_and_expansion() -> anyhow::Result<()> {
        let (_directory, mut app) = application()?;
        let area = Rect::new(0, 0, 120, 35);
        app.prefix = crate::keybindings::Prefix::new("alt+a")?;
        app.session.assistant.input.insert_str("Unsent draft");
        app.workspace_expansion = Some(Focus::Editor);
        let source = app.session.editor.source();
        key(&mut app, KeyCode::Char('a'), KeyModifiers::ALT, area);
        key(&mut app, KeyCode::Char('?'), KeyModifiers::SHIFT, area);
        assert!(app.shortcuts_open && app.overlay_open());
        assert!(!app.prefix.armed && !app.editor_has_focus());

        let mut terminal = Terminal::new(TestBackend::new(area.width, area.height))?;
        terminal.draw(|frame| app.draw(frame))?;
        let screen: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(screen.contains("Prefix: Alt-a"));
        assert!(screen.contains("Zoom pane"));
        assert!(!screen.contains("Download & retry"));

        key(&mut app, KeyCode::Char('w'), KeyModifiers::NONE, area);
        app.handle_event(Event::Paste("Do not insert".into()), area);
        assert_eq!(app.session.editor.source(), source);
        assert_eq!(app.session.assistant.input.lines(), &["Unsent draft"]);
        assert!(app.workspace_expansion == Some(Focus::Editor));

        key(&mut app, KeyCode::Esc, KeyModifiers::NONE, area);
        assert!(!app.shortcuts_open && !app.overlay_open());
        assert!(app.focus == Focus::Editor && app.editor_has_focus());
        Ok(())
    }

    #[test]
    fn search_filters_commands_and_keys_without_editing_the_document() -> anyhow::Result<()> {
        let (_directory, mut app) = application()?;
        let area = Rect::new(0, 0, 120, 35);
        let source = app.session.editor.source();
        app.handle_prefix_action(KeyCode::Char('?'), area);
        key(&mut app, KeyCode::Char('/'), KeyModifiers::NONE, area);
        for character in "zoom".chars() {
            key(&mut app, KeyCode::Char(character), KeyModifiers::NONE, area);
        }

        let text = |app: &App| {
            app.shortcut_lines(80)
                .iter()
                .map(Line::to_string)
                .collect::<Vec<_>>()
                .join("\n")
        };
        assert!(text(&app).contains("Zoom pane"));
        assert!(!text(&app).contains("Settings"));
        key(&mut app, KeyCode::Char('!'), KeyModifiers::NONE, area);
        assert!(text(&app).contains("No matching shortcuts"));
        key(&mut app, KeyCode::Backspace, KeyModifiers::NONE, area);
        assert!(text(&app).contains("Zoom pane"));
        assert_eq!(app.session.editor.source(), source);

        key(&mut app, KeyCode::Esc, KeyModifiers::NONE, area);
        assert!(app.shortcuts_open);
        assert!(app.shortcut_query.is_empty());
        app.shortcut_query = "prefix+w".into();
        assert!(text(&app).contains("Save"));
        assert!(!text(&app).contains("Actions"));
        key(&mut app, KeyCode::Esc, KeyModifiers::NONE, area);
        app.handle_prefix_action(KeyCode::Char('?'), area);
        assert!(app.shortcut_query.is_empty());
        Ok(())
    }

    #[test]
    fn help_scrolls_in_small_windows_and_closes_on_outside_click() -> anyhow::Result<()> {
        let (_directory, mut app) = application()?;
        let area = Rect::new(0, 0, 40, 10);
        app.handle_prefix_action(KeyCode::Char('?'), area);
        key(&mut app, KeyCode::End, KeyModifiers::NONE, area);
        assert!(app.shortcut_scroll > 0);
        assert_eq!(app.shortcut_scroll, app.shortcut_scroll_limit(area));

        let mut terminal = Terminal::new(TestBackend::new(area.width, area.height))?;
        terminal.draw(|frame| app.draw(frame))?;
        let screen: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(screen.contains("Export PDF"));

        let mut tiny = Terminal::new(TestBackend::new(1, 1))?;
        tiny.draw(|frame| app.draw(frame))?;
        app.handle_event(
            Event::Mouse(ratatui::crossterm::event::MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 0,
                row: 0,
                modifiers: KeyModifiers::NONE,
            }),
            area,
        );
        assert!(!app.shortcuts_open);
        assert!(app.focus == Focus::Editor);
        Ok(())
    }
}
