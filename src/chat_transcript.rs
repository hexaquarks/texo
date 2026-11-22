use std::{io, ops::Range};

use ratatui::{
    Frame,
    crossterm::{
        clipboard::CopyToClipboard,
        event::{MouseButton, MouseEvent, MouseEventKind},
        execute,
    },
    layout::{Position, Rect},
    style::Modifier,
    text::{Line, Span},
    widgets::Paragraph,
};
use texo::theme;
use unicode_segmentation::UnicodeSegmentation;

use crate::clipboard::{ClipboardAccess, SystemClipboard};

use super::Message;

const CONTINUATION: &str = "   ";
const RIGHT_PADDING: u16 = 1;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct TextPosition {
    message: usize,
    byte: usize,
}

struct Selection {
    anchor: TextPosition,
    end: TextPosition,
}

impl Selection {
    fn range(&self) -> Option<Range<TextPosition>> {
        if self.anchor == self.end {
            return None;
        }

        Some(self.anchor.min(self.end)..self.anchor.max(self.end))
    }
}

pub(super) struct TranscriptRow {
    message: usize,
    bytes: Range<usize>,
    kind: RowKind,
}

#[derive(Clone, Copy)]
enum RowKind {
    Text(&'static str),
    PromptPadding,
    Separator,
}

#[derive(Default)]
pub(super) struct Transcript {
    viewport: Rect,
    visible_rows: Vec<TranscriptRow>,
    selection: Option<Selection>,
    dragging: bool,
    clipboard: SystemClipboard,
}

impl Transcript {
    pub(super) fn hide(&mut self) {
        self.viewport = Rect::default();
        self.visible_rows.clear();
    }

    pub(super) fn reset(&mut self) {
        self.hide();
        self.clear_selection();
    }

    pub(super) fn clear_selection(&mut self) {
        self.selection = None;
        self.end_drag();
    }

    pub(super) fn end_drag(&mut self) {
        self.dragging = false;
    }

    pub(super) fn begin_drag(&mut self, messages: &[Message], position: Position, extend: bool) {
        if !self.viewport.contains(position) {
            self.clear_selection();
            return;
        }

        let Some(point) = self.hit(messages, position) else {
            self.clear_selection();
            return;
        };

        if extend && let Some(selection) = &mut self.selection {
            selection.end = point;
        } else {
            self.selection = Some(Selection {
                anchor: point,
                end: point,
            });
        }
        self.dragging = true;
    }

    pub(super) fn handle_drag(&mut self, messages: &[Message], mouse: MouseEvent) -> bool {
        if !self.dragging {
            return false;
        }

        let released = mouse.kind == MouseEventKind::Up(MouseButton::Left);
        if !released && mouse.kind != MouseEventKind::Drag(MouseButton::Left) {
            return false;
        }

        let position = Position::new(mouse.column, mouse.row);
        if let Some(point) = self.hit(messages, position)
            && let Some(selection) = &mut self.selection
        {
            selection.end = point;
        }
        if released {
            self.end_drag();
        }

        true
    }

    fn hit(&self, messages: &[Message], position: Position) -> Option<TextPosition> {
        // An active drag can leave the pane. Keep its endpoint on a visible row.
        let index = usize::from(position.y.saturating_sub(self.viewport.y))
            .min(self.visible_rows.len().saturating_sub(1));
        let row = self.visible_rows.get(index)?;
        let message = messages.get(row.message)?;
        let column = usize::from(position.x.saturating_sub(self.viewport.x));

        Some(row.position(&message.content, column))
    }

    fn selected_text(&self, messages: &[Message]) -> Option<String> {
        let range = self.selection.as_ref()?.range()?;
        let mut text = String::new();

        for index in range.start.message..=range.end.message {
            let content = &messages.get(index)?.content;
            let start = if index == range.start.message {
                range.start.byte
            } else {
                0
            };
            let end = if index == range.end.message {
                range.end.byte
            } else {
                content.len()
            };

            if index != range.start.message {
                text.push_str("\n\n");
            }
            text.push_str(content.get(start..end)?);
        }

        Some(text)
    }

    pub(super) fn copy(&mut self, messages: &[Message]) -> Option<&'static str> {
        let text = self.selected_text(messages)?;
        let status = if self.clipboard.write(text.clone()).is_ok() {
            "Copied chat selection"
        } else if execute!(io::stdout(), CopyToClipboard::to_clipboard_from(text)).is_ok() {
            // OSC 52 has no acknowledgement. Terminal and multiplexer policy decide whether to accept it.
            "Sent chat selection to terminal clipboard"
        } else {
            "Could not copy chat selection"
        };

        Some(status)
    }

    pub(super) fn draw(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        rows: Vec<TranscriptRow>,
        covered_rows: u16,
        messages: &[Message],
        scroll_from_bottom: &mut u16,
    ) {
        let maximum_scroll = rows.len().saturating_sub(usize::from(area.height));
        *scroll_from_bottom =
            (*scroll_from_bottom).min(u16::try_from(maximum_scroll).unwrap_or(u16::MAX));
        let offset = maximum_scroll - usize::from(*scroll_from_bottom);
        let selection = self.selection.as_ref().and_then(Selection::range);

        let mut visible_rows: Vec<_> = rows
            .into_iter()
            .skip(offset)
            .take(usize::from(area.height))
            .collect();
        let lines: Vec<_> = visible_rows
            .iter()
            .map(|row| row.line(&messages[row.message], area.width, selection.as_ref()))
            .collect();
        frame.render_widget(Paragraph::new(lines).style(theme::text()), area);

        visible_rows.truncate(usize::from(area.height.saturating_sub(covered_rows)));
        self.viewport = Rect {
            height: u16::try_from(visible_rows.len()).unwrap_or(u16::MAX),
            ..area
        };
        self.visible_rows = visible_rows;
    }
}

impl TranscriptRow {
    fn position(&self, content: &str, column: usize) -> TextPosition {
        let mut byte = self.bytes.start;
        let RowKind::Text(marker) = self.kind else {
            return TextPosition {
                message: self.message,
                byte,
            };
        };
        let mut cell = marker.len();

        for grapheme in content[self.bytes.clone()].graphemes(true) {
            let width = Span::raw(grapheme).width();
            if column < cell + width {
                break;
            }

            byte += grapheme.len();
            cell += width;
        }

        TextPosition {
            message: self.message,
            byte,
        }
    }

    fn line<'a>(
        &self,
        message: &'a Message,
        width: u16,
        selection: Option<&Range<TextPosition>>,
    ) -> Line<'a> {
        if matches!(self.kind, RowKind::Separator) {
            return Line::default();
        }

        let is_user = message.role == "user";
        let style = if message.error {
            theme::text().fg(theme::ERROR)
        } else if is_user {
            theme::text().fg(theme::ACCENT).bg(theme::ELEVATED)
        } else {
            theme::text()
        };

        let mut line = Line::default().style(style);
        if let RowKind::Text(marker) = self.kind {
            line.spans.push(Span::raw(marker));

            let content = &message.content;
            let bytes = self.bytes.clone();
            if let Some(selected) = self.selected_bytes(selection) {
                line.spans
                    .push(Span::raw(&content[bytes.start..selected.start]));
                line.spans.push(Span::styled(
                    &content[selected.clone()],
                    style.add_modifier(Modifier::REVERSED),
                ));
                line.spans
                    .push(Span::raw(&content[selected.end..bytes.end]));
            } else {
                line.spans.push(Span::raw(&content[bytes]));
            }
        }

        if is_user {
            let padding = usize::from(width).saturating_sub(line.width());
            line.spans.push(Span::raw(" ".repeat(padding)));
        }
        line
    }

    fn selected_bytes(&self, selection: Option<&Range<TextPosition>>) -> Option<Range<usize>> {
        let range = selection?;
        if self.message < range.start.message || self.message > range.end.message {
            return None;
        }

        let mut bytes = self.bytes.clone();
        if self.message == range.start.message {
            bytes.start = bytes.start.max(range.start.byte);
        }
        if self.message == range.end.message {
            bytes.end = bytes.end.min(range.end.byte);
        }

        (!bytes.is_empty()).then_some(bytes)
    }
}

pub(super) fn transcript_rows(
    messages: &[Message],
    width: u16,
    compact: bool,
) -> Vec<TranscriptRow> {
    let mut rows = Vec::new();
    let content_width = usize::from(width)
        .saturating_sub(CONTINUATION.len() + usize::from(RIGHT_PADDING))
        .max(1);
    let wrap_options = textwrap::Options::new(content_width).break_words(false);

    for (index, message) in messages.iter().enumerate() {
        let padded = message.role == "user" && !compact;
        if padded {
            rows.push(spacer(index, 0, RowKind::PromptPadding));
        }

        let mut marker = match (message.role, message.error) {
            ("user", _) => " > ",
            (_, true) => " ! ",
            _ => " * ",
        };
        let mut line_start = 0;

        for physical_line in message.content.split_inclusive('\n') {
            let paragraph = physical_line.lines().next().unwrap_or_default();
            let mut remaining = paragraph;

            for wrapped in textwrap::wrap(paragraph, &wrap_options) {
                // Without indentation or inserted hyphens, each wrapped line is a source slice.
                // Search forward so repeated words map to their own occurrence.
                let skipped = remaining
                    .find(wrapped.as_ref())
                    .expect("wrapped transcript text is a source slice");
                let start_byte = line_start + paragraph.len() - remaining.len() + skipped;

                for bytes in grapheme_rows(&wrapped, start_byte, content_width) {
                    rows.push(TranscriptRow {
                        message: index,
                        bytes,
                        kind: RowKind::Text(marker),
                    });
                    marker = CONTINUATION;
                }
                remaining = &remaining[skipped + wrapped.len()..];
            }

            line_start += physical_line.len();
        }

        if padded {
            rows.push(spacer(index, message.content.len(), RowKind::PromptPadding));
        }
        // Keep a blank line between turns without wasting the last visible transcript row.
        rows.push(spacer(index, message.content.len(), RowKind::Separator));
    }

    rows.pop();
    rows
}

fn spacer(message: usize, byte: usize, kind: RowKind) -> TranscriptRow {
    TranscriptRow {
        message,
        bytes: byte..byte,
        kind,
    }
}

fn grapheme_rows(text: &str, start: usize, width: usize) -> Vec<Range<usize>> {
    let mut rows = Vec::new();
    let mut row_start = start;
    let mut columns = 0;

    // Long words need a second pass so wraps never split a displayed grapheme.
    for (offset, grapheme) in text.grapheme_indices(true) {
        let grapheme_width = Span::raw(grapheme).width();
        if columns > 0 && columns + grapheme_width > width {
            rows.push(row_start..start + offset);
            row_start = start + offset;
            columns = 0;
        }

        columns += grapheme_width;
    }

    rows.push(row_start..start + text.len());
    rows
}

#[cfg(test)]
#[path = "chat_transcript_tests.rs"]
mod tests;
