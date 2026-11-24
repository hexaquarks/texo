use ratatui::{
    Frame,
    crossterm::event::{Event, KeyCode, MouseButton, MouseEventKind},
    layout::{Constraint, Layout, Margin, Position, Rect},
    text::{Line, Span},
    widgets::{Block, BorderType, Clear, Paragraph},
};
use similar::{ChangeTag, TextDiff};
use texo::theme;

use super::App;
use crate::{actions::Action, history};

const MAX_WIDTH: u16 = 76;
// Borders, legend, spacing, and three action rows surround the comparison.
const CHROME_ROWS: u16 = 8;
const MAX_HEIGHT: u16 = 26;
const DIFF_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(100);

pub(super) struct SaveConflict {
    comparison: Vec<Line<'static>>,
    top: usize,
    left: u16,
    message: String,
}

struct ConflictLayout {
    area: Rect,
    legend: Rect,
    source: Rect,
    preserve: Rect,
    reload: Rect,
    help: Rect,
}

impl ConflictLayout {
    fn new(screen: Rect, lines: usize) -> Self {
        let rows = u16::try_from(lines).unwrap_or(u16::MAX);
        let height = rows.saturating_add(CHROME_ROWS).min(MAX_HEIGHT);
        let area = screen.centered(Constraint::Length(MAX_WIDTH), Constraint::Length(height));
        let inner = area.inner(Margin::new(2, 1));
        let [legend, source, preserve, reload, help] = Layout::vertical([
            Constraint::Length(2),
            Constraint::Fill(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas(inner);

        Self {
            area,
            legend,
            source,
            preserve,
            reload,
            help,
        }
    }
}

impl SaveConflict {
    fn new(disk: &str, draft: &str) -> Self {
        let diff = TextDiff::configure()
            .timeout(DIFF_TIMEOUT)
            .diff_lines(disk, draft);
        let mut comparison = Vec::new();
        for change in diff.iter_all_changes() {
            let (marker, color) = match change.tag() {
                ChangeTag::Delete => ("− ", theme::LAVENDER),
                ChangeTag::Insert => ("+ ", theme::GREEN),
                ChangeTag::Equal => ("  ", theme::TEXT),
            };

            let text = change.value().trim_end_matches('\n');
            comparison.push(Line::styled(
                format!("{marker}{text}"),
                theme::dialog().fg(color),
            ));
            if change.missing_newline() {
                comparison.push(Line::styled(
                    format!("{marker}[No final newline]"),
                    theme::muted(),
                ));
            }
        }

        Self {
            comparison,
            top: 0,
            left: 0,
            message: String::new(),
        }
    }

    fn scroll(&mut self, key: KeyCode, area: Rect) {
        match key {
            KeyCode::Left => self.left = self.left.saturating_sub(1),
            KeyCode::Right => {
                let width = self.comparison.iter().map(Line::width).max().unwrap_or(0);
                let last = width.saturating_sub(usize::from(area.width));
                self.left = self
                    .left
                    .saturating_add(1)
                    .min(u16::try_from(last).unwrap_or(u16::MAX));
            }
            KeyCode::Up | KeyCode::Down | KeyCode::PageUp | KeyCode::PageDown => {
                let height = usize::from(area.height);
                let last = self.comparison.len().saturating_sub(height);
                let top = self.top.min(last);
                self.top = match key {
                    KeyCode::Up => top.saturating_sub(1),
                    KeyCode::PageUp => top.saturating_sub(height),
                    KeyCode::PageDown => top.saturating_add(height).min(last),
                    _ => top.saturating_add(1).min(last),
                };
            }
            _ => {}
        }
    }

    pub(super) fn draw(&self, frame: &mut Frame) {
        let layout = ConflictLayout::new(frame.area(), self.comparison.len());
        frame.render_widget(Clear, layout.area);
        frame.render_widget(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .title(" Save conflict ")
                .style(theme::dialog())
                .border_style(theme::dialog_title()),
            layout.area,
        );

        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("− On disk", theme::dialog().fg(theme::LAVENDER)),
                Span::raw("     "),
                Span::styled("+ Local draft", theme::dialog().fg(theme::GREEN)),
            ])),
            layout.legend,
        );

        let height = usize::from(layout.source.height);
        let top = self.top.min(self.comparison.len().saturating_sub(height));
        let visible: Vec<_> = self
            .comparison
            .iter()
            .skip(top)
            .take(height)
            .cloned()
            .collect();
        frame.render_widget(
            Paragraph::new(visible).scroll((0, self.left)),
            layout.source,
        );

        frame.render_widget(
            theme::shortcuts(&[("p", "Preserve draft in History")]),
            layout.preserve,
        );
        frame.render_widget(theme::shortcuts(&[("l", "Load from disk…")]), layout.reload);

        let help = if self.message.is_empty() {
            theme::shortcuts(&[("Arrows", "Scroll"), ("Esc", "Keep editing")])
        } else {
            Line::from(self.message.as_str())
        };
        frame.render_widget(help, layout.help);
    }
}

impl App {
    pub(super) fn handle_save_error(&mut self, error: anyhow::Error) {
        match error.downcast::<history::SourceChanged>() {
            Ok(changed) => {
                self.save_conflict = Some(SaveConflict::new(
                    &changed.disk_source,
                    &self.session.editor.source(),
                ));
            }
            Err(error) => self.report_error(error),
        }
    }

    pub(super) fn handle_save_conflict(&mut self, event: Event, screen: Rect) {
        let Some(conflict) = self.save_conflict.as_mut() else {
            return;
        };

        let layout = ConflictLayout::new(screen, conflict.comparison.len());
        let key = match event {
            Event::Key(key) if key.modifiers.is_empty() => key.code,
            Event::Mouse(mouse) => {
                let point = Position::new(mouse.column, mouse.row);
                match mouse.kind {
                    MouseEventKind::ScrollUp => KeyCode::Up,
                    MouseEventKind::ScrollDown => KeyCode::Down,
                    MouseEventKind::Down(MouseButton::Left) if layout.preserve.contains(point) => {
                        KeyCode::Char('p')
                    }
                    MouseEventKind::Down(MouseButton::Left) if layout.reload.contains(point) => {
                        KeyCode::Char('l')
                    }
                    _ => return,
                }
            }
            _ => return,
        };

        match key {
            KeyCode::Esc => self.save_conflict = None,
            KeyCode::Char('l') => {
                self.save_conflict = None;
                self.open_project_dialog(Action::ReloadSource);
            }
            KeyCode::Char('p') => {
                match history::preserve_draft(
                    &self.session.editor.path,
                    &self.session.editor.source(),
                ) {
                    Ok(()) => {
                        conflict.message =
                            "Draft preserved in History. Esc to keep editing.".into();
                        self.refresh_history();
                    }
                    Err(error) => conflict.message = format!("{error:#}"),
                }
            }
            key => conflict.scroll(key, layout.source),
        }
    }
}
