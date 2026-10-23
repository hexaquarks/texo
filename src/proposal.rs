use std::time::Duration;

use ratatui::{
    Frame,
    layout::{Margin, Rect},
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
};
use similar::{ChangeTag, TextDiff};
use texo::theme;

use crate::{assistant::ProposedEdit, editor::source_colors};

const DIFF_TIMEOUT: Duration = Duration::from_millis(100);
const SCROLL_LINES: u16 = 3;
const MIN_LINE_NUMBER_WIDTH: usize = 4;
const SCROLL_COLUMNS: u16 = 8;

pub struct ProposalReview {
    pub edit: ProposedEdit,
    lines: Vec<Line<'static>>,
    vertical_offset: u16,
    horizontal_offset: u16,
}

impl ProposalReview {
    pub fn new(proposal: &ProposedEdit) -> Self {
        let diff = TextDiff::configure()
            .timeout(DIFF_TIMEOUT)
            .diff_lines(&proposal.source_before, &proposal.source_after);

        let number_width = proposal
            .source_after
            .lines()
            .count()
            .to_string()
            .len()
            .max(MIN_LINE_NUMBER_WIDTH);

        let after_colors = source_colors(&proposal.source_after);

        let lines = diff
            .iter_all_changes()
            .filter_map(|change| {
                let index = change.new_index()?;
                let number = index + 1;
                let marker = if change.tag() == ChangeTag::Insert {
                    "▎"
                } else {
                    " "
                };
                let text = change
                    .value()
                    .trim_end_matches('\n')
                    .trim_end_matches('\r')
                    .to_owned();

                let colors = after_colors.get(index);
                let mut spans = vec![
                    Span::styled(format!("{number:>number_width$} "), theme::muted()),
                    Span::styled(format!("{marker} "), Style::default().fg(theme::ACCENT)),
                ];

                for (index, character) in text.chars().enumerate() {
                    let color = colors
                        .and_then(|line| line.get(index))
                        .copied()
                        .unwrap_or(theme::TEXT);
                    let style = Style::default().fg(color);

                    if let Some(last) = spans.last_mut()
                        && last.style == style
                    {
                        last.content.to_mut().push(character);
                    } else {
                        spans.push(Span::styled(character.to_string(), style));
                    }
                }

                Some(Line::from(spans).style(theme::text()))
            })
            .collect();

        Self {
            edit: proposal.clone(),
            lines,
            vertical_offset: 0,
            horizontal_offset: 0,
        }
    }

    pub fn scroll(&mut self, up: bool) {
        let maximum = u16::try_from(self.lines.len().saturating_sub(1)).unwrap_or(u16::MAX);
        self.vertical_offset = if up {
            self.vertical_offset.saturating_sub(SCROLL_LINES)
        } else {
            self.vertical_offset
                .saturating_add(SCROLL_LINES)
                .min(maximum)
        };
    }

    pub fn scroll_horizontal(&mut self, left: bool) {
        let maximum = self.lines.iter().map(Line::width).max().unwrap_or(0);
        let maximum = u16::try_from(maximum).unwrap_or(u16::MAX);

        self.horizontal_offset = if left {
            self.horizontal_offset.saturating_sub(SCROLL_COLUMNS)
        } else {
            self.horizontal_offset
                .saturating_add(SCROLL_COLUMNS)
                .min(maximum)
        };
    }

    pub fn draw(&self, frame: &mut Frame, area: Rect) {
        let content = area.inner(Margin::new(theme::PANEL_PADDING, 1));

        for (row, line) in self
            .lines
            .iter()
            .skip(usize::from(self.vertical_offset))
            .take(usize::from(content.height))
            .enumerate()
        {
            let row_area = Rect {
                y: content.y + row as u16,
                height: 1,
                ..content
            };
            frame.render_widget(
                Paragraph::new(line.clone())
                    .style(line.style)
                    .scroll((0, self.horizontal_offset)),
                row_area,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn proposed_source_shows_only_current_lines_with_subtle_change_markers() -> anyhow::Result<()> {
        let edit = ProposedEdit {
            source_before: "Same\nOld bullet\n".into(),
            source_after: "Same\nNew bullet\n".into(),
        };
        let review = ProposalReview::new(&edit);
        let mut terminal = Terminal::new(TestBackend::new(80, 12))?;

        terminal.draw(|frame| review.draw(frame, frame.area()))?;
        let buffer = terminal.backend().buffer();
        let text: String = buffer.content.iter().map(|cell| cell.symbol()).collect();

        assert!(!text.contains("Old bullet"));
        assert!(text.contains("New bullet"));
        assert_eq!(review.lines.len(), 2);
        assert!(text.contains("▎"));
        for row in 1..=2 {
            for column in theme::PANEL_PADDING..80 - theme::PANEL_PADDING {
                assert_eq!(buffer[(column, row)].bg, theme::BACKGROUND);
            }
        }

        assert_eq!(review.edit.source_before, edit.source_before);
        Ok(())
    }

    #[test]
    fn empty_unicode_and_tiny_reviews_do_not_panic() -> anyhow::Result<()> {
        for (before, after) in [("", "é界"), ("Removed", ""), ("Same", "Same")] {
            let mut review = ProposalReview::new(&ProposedEdit {
                source_before: before.into(),
                source_after: after.into(),
            });
            review.scroll(false);
            review.scroll_horizontal(false);

            let mut terminal = Terminal::new(TestBackend::new(2, 2))?;
            terminal.draw(|frame| review.draw(frame, frame.area()))?;
        }
        Ok(())
    }
}
