use std::{io, ops::Range};

use ratatui::{
    Frame,
    buffer::{Buffer, CellWidth},
    crossterm::{
        clipboard::CopyToClipboard,
        event::{MouseButton, MouseEvent, MouseEventKind},
        execute,
    },
    layout::{Position, Rect},
    style::Modifier,
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
    copy_notice: Option<&'static str>,
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
        self.copy_notice = None;
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
        self.copy_notice = None;
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

    pub(super) fn copy(&mut self, messages: &[Message]) -> bool {
        let Some(text) = self.selected_text(messages) else {
            return false;
        };

        self.copy_notice = Some(copy_text(&mut self.clipboard, io::stdout(), text));
        true
    }

    pub(super) fn copy_notice(&self) -> Option<&'static str> {
        self.copy_notice
    }

    pub(super) fn draw(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        rows: Vec<TranscriptRow>,
        covered_rows: u16,
        messages: &[Message],
        scroll_from_bottom: &mut usize,
    ) {
        let maximum_scroll = rows.len().saturating_sub(usize::from(area.height));
        *scroll_from_bottom = (*scroll_from_bottom).min(maximum_scroll);
        let offset = maximum_scroll - *scroll_from_bottom;
        let selection = self.selection.as_ref().and_then(Selection::range);

        let mut visible_rows: Vec<_> = rows
            .into_iter()
            .skip(offset)
            .take(usize::from(area.height))
            .collect();
        let buffer = frame.buffer_mut();
        buffer.set_style(area, theme::text());

        for (y, row) in (area.top()..area.bottom()).zip(&visible_rows) {
            let row_area = Rect::new(area.x, y, area.width, 1);
            row.render(buffer, row_area, &messages[row.message], selection.as_ref());
        }

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
            let width = grapheme_width(grapheme);
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

    fn render(
        &self,
        buffer: &mut Buffer,
        area: Rect,
        message: &Message,
        selection: Option<&Range<TextPosition>>,
    ) {
        if matches!(self.kind, RowKind::Separator) {
            return;
        }

        let style = if message.error {
            theme::text().fg(theme::ERROR)
        } else if message.role == "user" {
            theme::text().fg(theme::ACCENT).bg(theme::ELEVATED)
        } else {
            theme::text()
        };
        buffer.set_style(area, style);

        let RowKind::Text(marker) = self.kind else {
            return;
        };
        let mut x = area.x;
        let mut write = |text, style| {
            let width = usize::from(area.right().saturating_sub(x));
            (x, _) = buffer.set_stringn(x, area.y, text, width, style);
        };

        // Advance by the buffer's actual cell count so adding a style boundary
        // cannot move the text that follows it.
        write(marker, style);
        let content = &message.content;
        let bytes = self.bytes.clone();

        if let Some(selected) = self.selected_bytes(selection) {
            write(&content[bytes.start..selected.start], style);
            write(
                &content[selected.clone()],
                style.add_modifier(Modifier::REVERSED),
            );
            write(&content[selected.end..bytes.end], style);
        } else {
            write(&content[bytes], style);
        }
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

            for bytes in paragraph_rows(paragraph, &wrap_options) {
                rows.push(TranscriptRow {
                    message: index,
                    bytes: line_start + bytes.start..line_start + bytes.end,
                    kind: RowKind::Text(marker),
                });
                marker = CONTINUATION;
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

fn paragraph_rows(paragraph: &str, options: &textwrap::Options<'_>) -> Vec<Range<usize>> {
    let mut rows = Vec::new();
    let mut source_graphemes = paragraph.grapheme_indices(true).peekable();
    let mut next_byte = 0;

    for wrapped in textwrap::wrap(paragraph, options) {
        // Without inserted hyphens or indentation, wrapped lines are source slices.
        let skipped = paragraph[next_byte..]
            .find(wrapped.as_ref())
            .expect("wrapped transcript text is a source slice");
        let start = next_byte + skipped;
        let end = start + wrapped.len();
        next_byte = end;

        // The word wrapper can trim part of a grapheme at a word break. Skip
        // only whole clusters, and consume each cluster once in source order.
        while source_graphemes
            .next_if(|(offset, grapheme)| offset + grapheme.len() <= start)
            .is_some()
        {}
        let graphemes =
            std::iter::from_fn(|| source_graphemes.next_if(|(offset, _)| *offset < end));
        append_grapheme_rows(&mut rows, graphemes, options.width);
    }

    if rows.is_empty() {
        rows.push(0..0);
    }
    rows
}

fn append_grapheme_rows<'a>(
    rows: &mut Vec<Range<usize>>,
    graphemes: impl Iterator<Item = (usize, &'a str)>,
    width: usize,
) {
    let mut graphemes = graphemes.peekable();
    let Some(&(start, _)) = graphemes.peek() else {
        return;
    };

    let mut current = start..start;
    let mut columns = 0;

    for (offset, grapheme) in graphemes {
        let grapheme_width = grapheme_width(grapheme);
        if columns > 0 && columns + grapheme_width > width {
            rows.push(current);
            current = offset..offset;
            columns = 0;
        }

        current.end = offset + grapheme.len();
        columns += grapheme_width;
    }

    rows.push(current);
}

fn grapheme_width(grapheme: &str) -> usize {
    // Match Buffer::set_stringn, which skips control characters.
    if grapheme.contains(char::is_control) {
        return 0;
    }

    usize::from(grapheme.cell_width())
}

fn copy_text(
    clipboard: &mut impl ClipboardAccess,
    mut terminal: impl io::Write,
    text: String,
) -> &'static str {
    if clipboard.write(text.clone()).is_ok() {
        return "Copied chat selection";
    }

    if execute!(terminal, CopyToClipboard::to_clipboard_from(text)).is_ok() {
        // OSC 52 has no acknowledgement from the terminal.
        "Sent to terminal clipboard"
    } else {
        "Could not copy chat selection"
    }
}

#[cfg(test)]
#[path = "chat_transcript_tests.rs"]
mod tests;
