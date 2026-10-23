use std::time::Duration;

use ratatui::{
    Frame,
    layout::{Margin, Rect},
    style::{Color, Style},
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
const ADDED_BACKGROUND: Color = Color::Rgb(28, 53, 38);
const REMOVED_BACKGROUND: Color = Color::Rgb(62, 30, 34);
const ADDED_TEXT: Color = Color::Rgb(166, 227, 161);

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
            .source_before
            .lines()
            .count()
            .max(proposal.source_after.lines().count())
            .to_string()
            .len()
            .max(MIN_LINE_NUMBER_WIDTH);

        let before_colors = source_colors(&proposal.source_before);
        let after_colors = source_colors(&proposal.source_after);

        let lines = diff
            .iter_all_changes()
            .map(|change| {
                let (marker, style) = match change.tag() {
                    ChangeTag::Delete => (
                        "−",
                        Style::default().fg(theme::ERROR).bg(REMOVED_BACKGROUND),
                    ),
                    ChangeTag::Insert => {
                        ("+", Style::default().fg(ADDED_TEXT).bg(ADDED_BACKGROUND))
                    }
                    ChangeTag::Equal => (" ", theme::text()),
                };
                let old = change
                    .old_index()
                    .map(|index| (index + 1).to_string())
                    .unwrap_or_default();
                let new = change
                    .new_index()
                    .map(|index| (index + 1).to_string())
                    .unwrap_or_default();
                let text = change
                    .value()
                    .trim_end_matches('\n')
                    .trim_end_matches('\r')
                    .to_owned();

                let colors = if change.tag() == ChangeTag::Delete {
                    change
                        .old_index()
                        .and_then(|index| before_colors.get(index))
                } else {
                    change.new_index().and_then(|index| after_colors.get(index))
                };
                let mut spans = vec![Span::styled(
                    format!("{old:>number_width$} {new:>number_width$} {marker} "),
                    theme::muted(),
                )];

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

                Line::from(spans).style(style)
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
    fn inline_diff_marks_insertions_and_deletions_without_changing_source() -> anyhow::Result<()> {
        let edit = ProposedEdit {
            source_before: "Same\nOld bullet\n".into(),
            source_after: "Same\nNew bullet\n".into(),
        };
        let review = ProposalReview::new(&edit);
        let mut terminal = Terminal::new(TestBackend::new(80, 12))?;

        terminal.draw(|frame| review.draw(frame, frame.area()))?;
        let buffer = terminal.backend().buffer();
        let text: String = buffer.content.iter().map(|cell| cell.symbol()).collect();

        assert!(text.contains("Old bullet"));
        assert!(text.contains("New bullet"));
        assert!(
            buffer
                .content
                .iter()
                .any(|cell| cell.bg == ADDED_BACKGROUND)
        );
        assert!(
            buffer
                .content
                .iter()
                .any(|cell| cell.bg == REMOVED_BACKGROUND)
        );
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
