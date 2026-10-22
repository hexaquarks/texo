use ratatui::{
    Frame,
    layout::{Constraint, Layout, Margin, Rect},
    style::Style,
    text::{Line, Text},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
};
use texo::theme;

use crate::assistant::ProposedEdit;

const REVIEW_MARGIN: u16 = 2;
const INSTRUCTIONS_HEIGHT: u16 = 2;
const SCROLL_LINES: u16 = 6;
const SCROLL_COLUMNS: u16 = 12;

pub struct ProposalReview {
    before: String,
    after: String,
    vertical_offset: u16,
    horizontal_offset: u16,
    maximum_vertical_offset: u16,
    maximum_horizontal_offset: u16,
}

impl ProposalReview {
    pub fn new(proposal: &ProposedEdit) -> Self {
        Self {
            before: proposal.source_before.clone(),
            after: proposal.source_after.clone(),
            vertical_offset: 0,
            horizontal_offset: 0,
            maximum_vertical_offset: 0,
            maximum_horizontal_offset: 0,
        }
    }

    pub fn scroll(&mut self, up: bool) {
        self.vertical_offset = if up {
            self.vertical_offset.saturating_sub(SCROLL_LINES)
        } else {
            self.vertical_offset
                .saturating_add(SCROLL_LINES)
                .min(self.maximum_vertical_offset)
        };
    }

    pub fn scroll_horizontal(&mut self, left: bool) {
        self.horizontal_offset = if left {
            self.horizontal_offset.saturating_sub(SCROLL_COLUMNS)
        } else {
            self.horizontal_offset
                .saturating_add(SCROLL_COLUMNS)
                .min(self.maximum_horizontal_offset)
        };
    }

    pub fn draw(&mut self, frame: &mut Frame) {
        let area = frame
            .area()
            .inner(Margin::new(REVIEW_MARGIN, REVIEW_MARGIN));
        frame.render_widget(Clear, area);
        let panel = Block::default()
            .title(" Review suggested edit ")
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme::ACCENT))
            .style(theme::text());
        let content = panel.inner(area);
        frame.render_widget(panel, area);

        let [documents, instructions] =
            Layout::vertical([Constraint::Min(0), Constraint::Length(INSTRUCTIONS_HEIGHT)])
                .areas(content);
        let [before, after] =
            Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
                .areas(documents);
        let before_content = source_area(before, &self.before);
        let after_content = source_area(after, &self.after);
        self.update_scroll_limits(before_content, after_content);

        self.draw_source(frame, before, " Before ", &self.before);
        self.draw_source(frame, after, " Suggested ", &self.after);
        frame.render_widget(
            Paragraph::new(" Enter apply · Esc back · Ctrl+Z twice to undo replacement\n PgUp/PgDn scroll · ←/→ long lines")
                .style(Style::default().fg(theme::ACCENT)),
            instructions,
        );
    }

    fn update_scroll_limits(&mut self, before_area: Rect, after_area: Rect) {
        let before = Text::from(self.before.as_str());
        let after = Text::from(self.after.as_str());
        let line_count = before.height().max(after.height());
        let visible_lines = usize::from(before_area.height.min(after_area.height));
        let before_overflow = before
            .width()
            .saturating_sub(usize::from(before_area.width));
        let after_overflow = after.width().saturating_sub(usize::from(after_area.width));

        self.maximum_vertical_offset =
            u16::try_from(line_count.saturating_sub(visible_lines)).unwrap_or(u16::MAX);
        self.maximum_horizontal_offset =
            u16::try_from(before_overflow.max(after_overflow)).unwrap_or(u16::MAX);
        self.vertical_offset = self.vertical_offset.min(self.maximum_vertical_offset);
        self.horizontal_offset = self.horizontal_offset.min(self.maximum_horizontal_offset);
    }

    fn draw_source(&self, frame: &mut Frame, area: Rect, title: &str, source: &str) {
        let block = source_block(title);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let gutter_width = line_number_width(source).min(inner.width);
        let [gutter, text] =
            Layout::horizontal([Constraint::Length(gutter_width), Constraint::Min(0)]).areas(inner);
        let numbers: Vec<Line> = source
            .split('\n')
            .enumerate()
            .map(|(index, _)| Line::from(format!("{} ", index + 1)).right_aligned())
            .collect();
        frame.render_widget(
            Paragraph::new(numbers)
                .style(theme::muted())
                .scroll((self.vertical_offset, 0)),
            gutter,
        );
        frame.render_widget(
            Paragraph::new(source)
                .style(Style::default().fg(theme::TEXT).bg(theme::EDITOR))
                .scroll((self.vertical_offset, self.horizontal_offset)),
            text,
        );
    }
}

fn source_block(title: &str) -> Block<'_> {
    Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::DIVIDER))
}

fn line_number_width(source: &str) -> u16 {
    let digits = source.split('\n').count().to_string().len();
    u16::try_from(digits + 1).unwrap_or(u16::MAX)
}

fn source_area(area: Rect, source: &str) -> Rect {
    let inner = source_block("").inner(area);
    let gutter_width = line_number_width(source).min(inner.width);
    Rect {
        x: inner.x + gutter_width,
        width: inner.width - gutter_width,
        ..inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    fn proposal() -> ProposedEdit {
        ProposedEdit {
            source_before: "Before content\n".repeat(80),
            source_after: format!("{} END", "Long suggested content ".repeat(20)),
        }
    }

    #[test]
    fn renders_both_versions_and_keeps_review_controls_visible() -> anyhow::Result<()> {
        let mut review = ProposalReview::new(&proposal());
        let mut terminal = Terminal::new(TestBackend::new(120, 30))?;
        terminal.draw(|frame| review.draw(frame))?;
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains("Before content"));
        assert!(text.contains("Long suggested content"));
        assert!(text.contains("Enter apply"));
        assert!(text.contains("Esc back"));
        Ok(())
    }

    #[test]
    fn long_lines_scroll_and_small_areas_do_not_panic() -> anyhow::Result<()> {
        let mut review = ProposalReview::new(&proposal());
        let mut terminal = Terminal::new(TestBackend::new(80, 20))?;
        terminal.draw(|frame| review.draw(frame))?;
        for _ in 0..100 {
            review.scroll(false);
            review.scroll_horizontal(false);
        }
        assert_eq!(review.vertical_offset, review.maximum_vertical_offset);
        assert_eq!(review.horizontal_offset, review.maximum_horizontal_offset);
        assert!(review.horizontal_offset > 0);
        review.vertical_offset = 0;
        terminal.draw(|frame| review.draw(frame))?;
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains(" END"));

        let mut small = Terminal::new(TestBackend::new(5, 3))?;
        small.draw(|frame| review.draw(frame))?;
        Ok(())
    }
}
