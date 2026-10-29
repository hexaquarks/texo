use ratatui::{
    Frame,
    crossterm::event::{Event, KeyCode, MouseButton, MouseEventKind},
    layout::{Margin, Position, Rect},
    text::Line,
    widgets::{Block, Borders, Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
};
use texo::theme;

use super::App;

const DETAILS_WIDTH: u16 = 110;
const DETAILS_HEIGHT: u16 = 30;
const MODAL_MARGIN: u16 = 2;
const WHEEL_SCROLL_LINES: u16 = 3;

impl App {
    pub(super) fn diagnostic_modal_area(area: Rect) -> Rect {
        let available = area.inner(Margin::new(MODAL_MARGIN, MODAL_MARGIN));
        let width = available.width.min(DETAILS_WIDTH);
        let height = available.height.min(DETAILS_HEIGHT);
        Rect::new(
            area.x + (area.width - width) / 2,
            area.y + (area.height - height) / 2,
            width,
            height,
        )
    }

    fn diagnostic_text(&self) -> String {
        let log = self.session.diagnostics.as_deref().unwrap_or_default();
        if let Some(diagnostics) = &self.session.build_diagnostics {
            format!("{}\n\n{log}", diagnostics.selected().help)
        } else {
            log.into()
        }
    }

    fn diagnostic_lines(&self, width: u16) -> Vec<Line<'static>> {
        self.diagnostic_text()
            .split('\n')
            .flat_map(|line| {
                textwrap::wrap(line, usize::from(width.max(1)))
                    .into_iter()
                    .map(|line| Line::from(line.into_owned()))
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    fn diagnostic_scroll_limit(&self, area: Rect) -> u16 {
        let content = Self::diagnostic_modal_area(area).inner(Margin::new(1, 1));
        let lines = self.diagnostic_lines(content.width).len();
        lines
            .saturating_sub(usize::from(content.height))
            .min(usize::from(u16::MAX)) as u16
    }

    pub(super) fn handle_diagnostic_details(&mut self, event: Event, area: Rect) {
        let modal = Self::diagnostic_modal_area(area);
        let page = modal.height.saturating_sub(2).max(1);
        let limit = self.diagnostic_scroll_limit(area);
        let scroll = &mut self.session.diagnostic_scroll;
        *scroll = (*scroll).min(limit);
        let mut close = false;

        match event {
            Event::Key(key) => match key.code {
                KeyCode::Esc | KeyCode::Enter => close = true,
                KeyCode::Up | KeyCode::Char('k') => *scroll = scroll.saturating_sub(1),
                KeyCode::Down | KeyCode::Char('j') => *scroll = scroll.saturating_add(1).min(limit),
                KeyCode::PageUp => *scroll = scroll.saturating_sub(page),
                KeyCode::PageDown | KeyCode::Char(' ') => {
                    *scroll = scroll.saturating_add(page).min(limit)
                }
                KeyCode::Home => *scroll = 0,
                KeyCode::End => *scroll = limit,
                _ => {}
            },
            Event::Mouse(mouse) => match mouse.kind {
                MouseEventKind::ScrollUp => *scroll = scroll.saturating_sub(WHEEL_SCROLL_LINES),
                MouseEventKind::ScrollDown => {
                    *scroll = scroll.saturating_add(WHEEL_SCROLL_LINES).min(limit)
                }
                MouseEventKind::Down(MouseButton::Left) => {
                    let position = Position::new(mouse.column, mouse.row);
                    close =
                        !modal.contains(position) || mouse.row == modal.bottom().saturating_sub(1);
                }
                _ => {}
            },
            _ => {}
        }

        if close {
            self.session.details_open = false;
            if self.session.build_diagnostics.is_none() {
                self.session.summary_hidden = true;
            }
        }
    }

    pub(super) fn draw_diagnostic_details(&self, frame: &mut Frame) {
        let area = Self::diagnostic_modal_area(frame.area());
        let limit = self.diagnostic_scroll_limit(frame.area());
        let scroll = self.session.diagnostic_scroll.min(limit);
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Error details ")
            .title_bottom(" Esc Close · ↑/↓ or wheel Scroll · PgUp/PgDn ")
            .border_style(theme::dialog_title());
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new(self.diagnostic_lines(area.width.saturating_sub(2)))
                .block(block)
                .style(theme::dialog())
                .scroll((scroll, 0)),
            area,
        );

        if limit > 0 {
            let mut scrollbar =
                ScrollbarState::new(usize::from(limit) + 1).position(usize::from(scroll));
            frame.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::VerticalRight),
                area.inner(Margin::new(0, 1)),
                &mut scrollbar,
            );
        }
    }
}
