use ratatui::{
    Frame,
    crossterm::event::{Event, KeyCode, MouseButton, MouseEventKind},
    layout::{Margin, Position, Rect},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
};
use texo::theme;

use super::App;

const HELP_WIDTH: u16 = 44;
const KEY_COLUMN_WIDTH: usize = 9;
const MODAL_MARGIN: u16 = 1;
const BORDER_ROWS: u16 = 2;
const WHEEL_ROWS: usize = 3;

impl App {
    fn shortcut_bindings(&self) -> Vec<(&'static str, &'static str)> {
        let mut bindings = vec![
            ("h/j/k/l", "Select pane"),
            ("H/J/K/L", "Resize pane"),
            ("c", "Cycle pane"),
            ("u", "Expand column"),
            ("z", "Zoom pane"),
            ("t", "Toggle tree"),
            ("n", "New resume"),
            ("i", "Import"),
            ("r", "Resumes"),
            ("f", "Files"),
            ("b", "Build PDF"),
            ("w", "Save"),
            ("e", "Export PDF"),
            ("v", "Refresh"),
            ("p", "Actions"),
            ("s", "Settings"),
            ("?", "Shortcuts"),
        ];

        if self.session.proposal_review.is_some() {
            bindings.extend([("a", "Accept"), ("m", "Modify"), ("d", "Discard")]);
        }
        if self.session.diagnostics.is_some() {
            bindings.extend([
                ("G", "Go to error"),
                ("F", "Ask AI"),
                ("R", "Download & retry"),
                ("E", "Error details"),
            ]);
        }
        if self.session.build_diagnostics.is_some() {
            bindings.extend([("D", "Toggle errors"), ("[ / ]", "Previous / next error")]);
        }

        bindings
    }

    pub(super) fn shortcut_modal_area(&self, area: Rect) -> Rect {
        let available = area.inner(Margin::new(MODAL_MARGIN, MODAL_MARGIN));
        let width = available.width.min(HELP_WIDTH);
        let height = available
            .height
            .min(self.shortcut_bindings().len() as u16 + BORDER_ROWS);
        Rect::new(
            area.x + (area.width - width) / 2,
            area.y + (area.height - height) / 2,
            width,
            height,
        )
    }

    fn shortcut_scroll_limit(&self, area: Rect) -> usize {
        let content_height = self
            .shortcut_modal_area(area)
            .height
            .saturating_sub(BORDER_ROWS);
        self.shortcut_bindings()
            .len()
            .saturating_sub(usize::from(content_height))
    }

    pub(super) fn handle_shortcut_help(&mut self, event: Event, area: Rect) {
        let modal = self.shortcut_modal_area(area);
        let page = usize::from(modal.height.saturating_sub(BORDER_ROWS).max(1));
        let limit = self.shortcut_scroll_limit(area);
        self.shortcut_scroll = self.shortcut_scroll.min(limit);

        match event {
            Event::Key(key) => match key.code {
                KeyCode::Esc | KeyCode::Enter | KeyCode::Char('?') => self.shortcuts_open = false,
                KeyCode::Up | KeyCode::Char('k') => {
                    self.shortcut_scroll = self.shortcut_scroll.saturating_sub(1);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.shortcut_scroll = self.shortcut_scroll.saturating_add(1).min(limit);
                }
                KeyCode::PageUp => self.shortcut_scroll = self.shortcut_scroll.saturating_sub(page),
                KeyCode::PageDown => {
                    self.shortcut_scroll = self.shortcut_scroll.saturating_add(page).min(limit)
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
                    if !modal.contains(position) || mouse.row == modal.bottom().saturating_sub(1) {
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
        let limit = self.shortcut_scroll_limit(frame.area());
        let scroll = self.shortcut_scroll.min(limit);
        let lines: Vec<_> = self
            .shortcut_bindings()
            .into_iter()
            .map(|(key, label)| {
                Line::from(vec![
                    Span::raw(" "),
                    theme::keycap(&format!("{key:<KEY_COLUMN_WIDTH$}")),
                    Span::raw(format!("  {label}")),
                ])
            })
            .collect();
        let footer = if limit > 0 {
            " ↑/↓ Scroll · Esc Close "
        } else {
            " Esc Close "
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .title(format!(" Shortcuts · {} ", self.prefix.label()))
            .title_bottom(footer)
            .border_style(theme::dialog_title());

        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new(lines)
                .block(block)
                .style(theme::dialog())
                .scroll((scroll as u16, 0)),
            area,
        );
        if limit > 0 {
            let mut state = ScrollbarState::new(limit + 1).position(scroll);
            frame.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::VerticalRight),
                area.inner(Margin::new(0, 1)),
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
        assert!(screen.contains("Shortcuts · Alt-a"));
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
        assert!(screen.contains("Settings"));

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
