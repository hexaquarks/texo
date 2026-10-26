// Adapted from ratatui-textarea 0.9.3 examples/vim.rs (MIT).
// Copyright (c) 2022 rhysd; Copyright (c) The Ratatui Developers.
// See THIRD_PARTY_LICENSES/ratatui-textarea.txt for the license.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui_textarea::{CursorMove, DataCursor, Scrolling, TextArea};

use super::clipboard::ClipboardAction;

#[path = "vim_jumps.rs"]
mod jumps;
#[path = "vim_objects.rs"]
mod objects;

use jumps::JumpList;
use objects::{ObjectRange, ObjectScope, text_object};

#[derive(Clone, Copy)]
enum Pending {
    GoToTop,
    Object(ObjectScope),
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum Mode {
    #[default]
    Normal,
    Insert,
    Visual,
    VisualLine,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Operator {
    Delete,
    Change,
    Yank,
}

impl Operator {
    fn from_key(code: KeyCode) -> Option<Self> {
        match code {
            KeyCode::Char('d') => Some(Self::Delete),
            KeyCode::Char('c') => Some(Self::Change),
            KeyCode::Char('y') => Some(Self::Yank),
            _ => None,
        }
    }

    fn clipboard_action(self) -> ClipboardAction {
        match self {
            Self::Delete | Self::Change => ClipboardAction::Cut,
            Self::Yank => ClipboardAction::Copy,
        }
    }
}

pub(super) enum VimInput {
    Consumed,
    Insert,
    Clipboard(ClipboardAction),
}

#[derive(Clone, Copy)]
struct LineSelection {
    anchor: DataCursor,
    cursor: DataCursor,
}

struct CompletedOperator {
    cursor: DataCursor,
    linewise_text: Option<String>,
}

#[derive(Default)]
pub(super) struct Vim {
    pub(super) mode: Mode,
    pending: Option<Pending>,
    jumps: JumpList,
    operator: Option<Operator>,
    visual_lines: Option<LineSelection>,
    completed_operator: Option<CompletedOperator>,
    linewise_register: bool,
}

impl Vim {
    pub(super) fn label(&self) -> &'static str {
        match self.operator {
            Some(Operator::Delete) => "NORMAL d…",
            Some(Operator::Change) => "NORMAL c…",
            Some(Operator::Yank) => "NORMAL y…",
            None => match self.mode {
                Mode::Normal if matches!(self.pending, Some(Pending::GoToTop)) => "NORMAL g…",
                Mode::Normal => "NORMAL",
                Mode::Insert => "INSERT",
                Mode::Visual if matches!(self.pending, Some(Pending::GoToTop)) => "VISUAL g…",
                Mode::Visual => "VISUAL",
                Mode::VisualLine => "VISUAL LINE",
            },
        }
    }

    pub(super) fn finish_mouse_selection(&mut self, text: &mut TextArea<'static>) {
        self.cancel_pending(text);
        self.visual_lines = None;
        if self.mode != Mode::Insert {
            self.mode = if text.is_selecting() {
                Mode::Visual
            } else {
                Mode::Normal
            };
        }
    }

    pub(super) fn cancel_pending(&mut self, text: &mut TextArea<'static>) {
        self.pending = None;
        if self.operator.take().is_some() {
            text.cancel_selection();
        }
    }

    // Only Insert mode may pass input to the ordinary editor. Unknown commands are consumed.
    pub(super) fn handle_key(&mut self, key: KeyEvent, text: &mut TextArea<'static>) -> VimInput {
        let cancel = key.code == KeyCode::Esc
            || (matches!(key.code, KeyCode::Char('[' | 'c'))
                && key.modifiers == KeyModifiers::CONTROL);
        if cancel {
            if let Some(selection) = self.visual_lines.take() {
                text.cancel_selection();
                Self::jump(text, selection.cursor);
            } else if self.mode == Mode::Insert && text.cursor().1 > 0 {
                text.move_cursor(CursorMove::Back);
            }
            self.mode = Mode::Normal;
            self.cancel_pending(text);
            text.cancel_selection();
            return VimInput::Consumed;
        }
        if self.mode == Mode::Insert {
            return VimInput::Insert;
        }

        if self.mode == Mode::Normal
            && key.code == KeyCode::Tab
            && !key
                .modifiers
                .intersects(KeyModifiers::ALT | KeyModifiers::SHIFT)
        {
            self.cancel_pending(text);
            self.visit_jump(text, false);
            return VimInput::Consumed;
        }

        if key
            .modifiers
            .intersects(KeyModifiers::ALT | KeyModifiers::CONTROL)
        {
            self.cancel_pending(text);
            self.handle_control_key(key, text);
            return VimInput::Consumed;
        }

        if self.mode == Mode::VisualLine {
            return self.handle_visual_line(key.code, text);
        }

        let pending = self.pending.take();
        if let Some(Pending::Object(scope)) = pending {
            return self.handle_text_object(scope, key.code, text);
        }
        let was_pending_g = matches!(pending, Some(Pending::GoToTop));
        if let Some(operator) = self.operator {
            return self.handle_operator(operator, key.code, was_pending_g, text);
        }
        if self.mode == Mode::Visual
            && let KeyCode::Char(character) = key.code
            && let Some(scope) = ObjectScope::from_key(character)
        {
            self.pending = Some(Pending::Object(scope));
            return VimInput::Consumed;
        }
        if let Some(motion) = Self::motion(key.code, text) {
            text.move_cursor(motion);
            return VimInput::Consumed;
        }
        if let Some(operator) = Operator::from_key(key.code) {
            if self.mode == Mode::Visual {
                Self::include_visual_endpoint(text);
                return self.finish_selection(operator, text);
            }

            text.cancel_selection();
            text.start_selection();
            self.operator = Some(operator);
            return VimInput::Consumed;
        }

        self.handle_command(key.code, was_pending_g, text)
    }

    fn handle_operator(
        &mut self,
        operator: Operator,
        code: KeyCode,
        was_pending_g: bool,
        text: &mut TextArea<'static>,
    ) -> VimInput {
        if was_pending_g && code != KeyCode::Char('g') {
            self.cancel_pending(text);
            return VimInput::Consumed;
        }

        if let KeyCode::Char(character) = code
            && let Some(scope) = ObjectScope::from_key(character)
        {
            self.pending = Some(Pending::Object(scope));
            return VimInput::Consumed;
        }

        if Operator::from_key(code) == Some(operator) {
            let row = text.cursor().0;
            return self.line_operator(operator, row, row, text);
        } else if let Some(motion) = Self::motion(code, text) {
            text.move_cursor(motion);
            if matches!(code, KeyCode::Char('e' | '$') | KeyCode::End) {
                text.move_cursor(CursorMove::Forward);
            }
        } else {
            match code {
                KeyCode::Char('g') if was_pending_g => text.move_cursor(CursorMove::Top),
                KeyCode::Char('g') => {
                    self.pending = Some(Pending::GoToTop);
                    return VimInput::Consumed;
                }
                KeyCode::Char('G') => text.move_cursor(CursorMove::Bottom),
                _ => {
                    self.cancel_pending(text);
                    return VimInput::Consumed;
                }
            }
        }

        self.finish_selection(operator, text)
    }

    fn handle_text_object(
        &mut self,
        scope: ObjectScope,
        code: KeyCode,
        text: &mut TextArea<'static>,
    ) -> VimInput {
        let range = match code {
            KeyCode::Char(character) => text_object(text, scope, character),
            _ => None,
        };
        let Some(range) = range else {
            self.cancel_pending(text);
            return VimInput::Consumed;
        };

        if self.mode == Mode::Visual {
            if range.start != range.end {
                Self::select_object(text, &range);
                // Visual commands include the cursor character; the component's range excludes its end.
                text.move_cursor(CursorMove::Back);
            }
            return VimInput::Consumed;
        }

        if let Some(operator) = self.operator {
            Self::select_object(text, &range);
            return self.finish_selection(operator, text);
        }

        VimInput::Consumed
    }

    fn select_object(text: &mut TextArea<'static>, range: &ObjectRange) {
        text.cancel_selection();
        Self::jump(text, range.start);
        text.start_selection();
        Self::jump(text, range.end);
    }

    pub(super) fn record_jump(&mut self, from: DataCursor, to: DataCursor) {
        self.jumps.record(from, to);
    }

    pub(super) fn clear_jump_history(&mut self) {
        self.jumps = JumpList::default();
    }

    fn document_jump(&mut self, text: &mut TextArea<'static>, motion: CursorMove) {
        let from = text.cursor();
        text.move_cursor(motion);
        self.record_jump(from, text.cursor());
    }

    fn visit_jump(&mut self, text: &mut TextArea<'static>, older: bool) {
        let target = if older {
            self.jumps.older(text.cursor())
        } else {
            self.jumps.newer()
        };
        if let Some(target) = target {
            text.cancel_selection();
            Self::jump(text, target);
        }
    }

    fn finish_operator(&mut self, operator: Operator) -> VimInput {
        self.operator = None;
        self.pending = None;
        self.mode = if operator == Operator::Change {
            Mode::Insert
        } else {
            Mode::Normal
        };

        VimInput::Clipboard(operator.clipboard_action())
    }

    pub(super) fn is_visual(&self) -> bool {
        matches!(self.mode, Mode::Visual | Mode::VisualLine)
    }

    fn first_nonblank(text: &TextArea<'_>, row: usize) -> usize {
        text.lines()[row]
            .chars()
            .position(|character| !character.is_whitespace())
            .unwrap_or(0)
    }

    fn last_source_row(text: &TextArea<'_>) -> usize {
        // The final empty textarea row represents the file's trailing newline.
        let last = text.lines().len() - 1;
        if last > 0 && text.lines()[last].is_empty() {
            last - 1
        } else {
            last
        }
    }

    fn clamp_normal_cursor(text: &mut TextArea<'static>) {
        let cursor = text.cursor();
        let row = cursor.0.min(Self::last_source_row(text));
        let column = cursor
            .1
            .min(text.lines()[row].chars().count().saturating_sub(1));
        Self::jump(text, DataCursor(row, column));
    }

    fn finish_selection(&mut self, operator: Operator, text: &TextArea<'static>) -> VimInput {
        let start = text
            .selection_range()
            .map_or(text.cursor(), |(start, _)| start.into());
        self.completed_operator = Some(CompletedOperator {
            cursor: start,
            linewise_text: None,
        });
        self.finish_operator(operator)
    }

    pub(super) fn finish_clipboard(&mut self, text: &mut TextArea<'static>) {
        let Some(operation) = self.completed_operator.take() else {
            self.linewise_register = false;
            return;
        };
        self.linewise_register = operation.linewise_text.is_some();
        if let Some(contents) = operation.linewise_text {
            text.set_yank_text(contents);
        }

        text.cancel_selection();
        Self::jump(text, operation.cursor);
        if self.mode == Mode::Normal {
            Self::clamp_normal_cursor(text);
        }
    }

    fn line_operator(
        &mut self,
        operator: Operator,
        first: usize,
        last: usize,
        text: &mut TextArea<'static>,
    ) -> VimInput {
        let (first, last) = (first.min(last), first.max(last));
        let last = last.min(Self::last_source_row(text));
        let contents = format!("{}\n", text.lines()[first..=last].join("\n"));
        let mut start = DataCursor(first, 0);
        let mut end = DataCursor(last, text.lines()[last].chars().count());

        if operator != Operator::Change {
            if last + 1 < text.lines().len() {
                end = DataCursor(last + 1, 0);
            } else if first > 0 && operator == Operator::Delete {
                start = DataCursor(first - 1, text.lines()[first - 1].chars().count());
            }
        }
        Self::select_object(text, &ObjectRange { start, end });
        self.completed_operator = Some(CompletedOperator {
            cursor: DataCursor(first, 0),
            linewise_text: Some(contents),
        });
        self.visual_lines = None;
        self.finish_operator(operator)
    }

    fn refresh_line_selection(&mut self, text: &mut TextArea<'static>) {
        let Some(selection) = self.visual_lines else {
            return;
        };
        let first = selection.anchor.0.min(selection.cursor.0);
        let last = selection.anchor.0.max(selection.cursor.0);
        let start = DataCursor(first, 0);
        let end = DataCursor(last, text.lines()[last].chars().count());
        text.cancel_selection();
        // Keep the active end on the line being moved, in either direction.
        if selection.cursor.0 < selection.anchor.0 {
            Self::jump(text, end);
            text.start_selection();
            Self::jump(text, start);
        } else {
            Self::jump(text, start);
            text.start_selection();
            Self::jump(text, end);
        }
    }

    fn handle_visual_line(&mut self, code: KeyCode, text: &mut TextArea<'static>) -> VimInput {
        let Some(mut selection) = self.visual_lines else {
            self.mode = Mode::Normal;
            return VimInput::Consumed;
        };
        let operator = match code {
            KeyCode::Char('D' | 'x') => Some(Operator::Delete),
            KeyCode::Char('C' | 'S') => Some(Operator::Change),
            _ => Operator::from_key(code),
        };
        if let Some(operator) = operator {
            return self.line_operator(
                operator,
                selection.anchor.0.min(selection.cursor.0),
                selection.anchor.0.max(selection.cursor.0),
                text,
            );
        }

        match code {
            KeyCode::Char('V') => {
                text.cancel_selection();
                Self::jump(text, selection.cursor);
                self.visual_lines = None;
                self.mode = Mode::Normal;
                return VimInput::Consumed;
            }
            KeyCode::Char('v') => {
                text.cancel_selection();
                Self::jump(text, selection.anchor);
                text.start_selection();
                Self::jump(text, selection.cursor);
                self.visual_lines = None;
                self.mode = Mode::Visual;
                return VimInput::Consumed;
            }
            KeyCode::Char('k') | KeyCode::Up => {
                selection.cursor.0 = selection.cursor.0.saturating_sub(1);
            }
            KeyCode::Char('j') | KeyCode::Down => {
                selection.cursor.0 = (selection.cursor.0 + 1).min(Self::last_source_row(text));
            }
            KeyCode::Char('G') => selection.cursor.0 = Self::last_source_row(text),
            KeyCode::Char('g') => {
                if matches!(self.pending.take(), Some(Pending::GoToTop)) {
                    selection.cursor.0 = 0;
                } else {
                    self.pending = Some(Pending::GoToTop);
                }
            }
            _ => {
                self.pending = None;
            }
        }
        self.visual_lines = Some(selection);
        self.refresh_line_selection(text);
        VimInput::Consumed
    }

    fn include_visual_endpoint(text: &mut TextArea<'static>) {
        let Some((start, end)) = text.selection_range() else {
            return;
        };

        // The component selects an exclusive range. Include the final character in either direction.
        text.cancel_selection();
        Self::jump(text, start.into());
        text.start_selection();
        Self::jump(text, end.into());
        text.move_cursor(CursorMove::Forward);
    }

    fn jump(text: &mut TextArea<'static>, cursor: DataCursor) {
        text.move_cursor(CursorMove::Jump(
            u16::try_from(cursor.0).unwrap_or(u16::MAX),
            u16::try_from(cursor.1).unwrap_or(u16::MAX),
        ));
    }

    fn handle_command(
        &mut self,
        code: KeyCode,
        was_pending_g: bool,
        text: &mut TextArea<'static>,
    ) -> VimInput {
        match code {
            KeyCode::Char('g') if was_pending_g => self.document_jump(text, CursorMove::Top),
            KeyCode::Char('g') => self.pending = Some(Pending::GoToTop),
            KeyCode::Char('G') => self.document_jump(text, CursorMove::Bottom),
            KeyCode::Char('i' | 'a' | 'I' | 'A' | 'o' | 'O') if self.mode == Mode::Normal => {
                self.enter_insert(code, text);
            }
            KeyCode::Char('v') if self.mode == Mode::Visual => {
                text.cancel_selection();
                self.mode = Mode::Normal;
            }
            KeyCode::Char('V') => {
                let cursor = text.cursor();
                self.visual_lines = Some(LineSelection {
                    anchor: cursor,
                    cursor,
                });
                self.mode = Mode::VisualLine;
                self.refresh_line_selection(text);
            }
            KeyCode::Char('v') if self.mode == Mode::Normal => {
                text.start_selection();
                self.mode = Mode::Visual;
            }
            KeyCode::Char('D' | 'C' | 'S') => {
                if self.mode == Mode::Visual {
                    Self::include_visual_endpoint(text);
                } else {
                    text.cancel_selection();
                    if code == KeyCode::Char('S') {
                        text.move_cursor(CursorMove::Head);
                    }
                    let start = text.cursor();
                    text.start_selection();
                    text.move_cursor(CursorMove::End);
                    if text.cursor() == start {
                        text.move_cursor(CursorMove::Forward);
                    }
                }
                let operator = if code == KeyCode::Char('D') {
                    Operator::Delete
                } else {
                    Operator::Change
                };
                return self.finish_selection(operator, text);
            }
            KeyCode::Char('x') if self.mode == Mode::Normal => {
                let cursor = text.cursor();
                if cursor.1 < text.lines()[cursor.0].chars().count()
                    || text.lines()[cursor.0].is_empty()
                {
                    text.start_selection();
                    text.move_cursor(CursorMove::Forward);
                    return self.finish_selection(Operator::Delete, text);
                }
            }
            KeyCode::Char('J') => {
                self.join_lines(text);
            }
            KeyCode::Char('p' | 'P') => {
                if self.mode == Mode::Visual {
                    Self::include_visual_endpoint(text);
                }
                if self.linewise_register && self.mode == Mode::Normal {
                    let row = text.cursor().0;
                    if code == KeyCode::Char('P') {
                        Self::jump(text, DataCursor(row, 0));
                        text.paste();
                        Self::jump(text, DataCursor(row, 0));
                    } else {
                        Self::jump(text, DataCursor(row, text.lines()[row].chars().count()));
                        let contents = text.yank_text();
                        text.insert_str(format!(
                            "\n{}",
                            contents.strip_suffix('\n').unwrap_or(&contents)
                        ));
                        Self::jump(text, DataCursor(row + 1, 0));
                    }
                } else {
                    if code == KeyCode::Char('p') && self.mode == Mode::Normal {
                        let cursor = text.cursor();
                        if cursor.1 < text.lines()[cursor.0].chars().count() {
                            text.move_cursor(CursorMove::Forward);
                        }
                    }
                    text.paste();
                    if text.cursor().1 > 0 {
                        text.move_cursor(CursorMove::Back);
                    }
                }
                self.mode = Mode::Normal;
                Self::clamp_normal_cursor(text);
            }
            KeyCode::Char('u') => {
                text.cancel_selection();
                text.undo();
                self.mode = Mode::Normal;
            }
            KeyCode::PageUp => text.scroll(Scrolling::PageUp),
            KeyCode::PageDown => text.scroll(Scrolling::PageDown),
            _ => {}
        }

        VimInput::Consumed
    }

    fn join_lines(&mut self, text: &mut TextArea<'static>) {
        let row = text.cursor().0;
        let (start_row, end_row) = if self.mode == Mode::Visual {
            text.selection_range()
                .map_or((row, row), |(start, end)| (start.0, end.0))
        } else {
            let next_row = if row + 1 < text.lines().len() {
                row + 1
            } else {
                row
            };
            (row, next_row)
        };

        text.cancel_selection();
        Self::jump(text, DataCursor(start_row, 0));
        for _ in start_row..end_row {
            text.move_cursor(CursorMove::End);
            text.delete_next_char();
            text.insert_char(' ');
        }
        self.mode = Mode::Normal;
    }

    fn motion(code: KeyCode, text: &TextArea<'static>) -> Option<CursorMove> {
        let cursor = text.cursor();
        let last_column = text.lines()[cursor.0].chars().count().saturating_sub(1);
        let target = match code {
            KeyCode::Char('h') | KeyCode::Left => DataCursor(cursor.0, cursor.1.saturating_sub(1)),
            KeyCode::Char('l') | KeyCode::Right => {
                DataCursor(cursor.0, (cursor.1 + 1).min(last_column))
            }
            KeyCode::Char('j') | KeyCode::Down => {
                let row = (cursor.0 + 1).min(Self::last_source_row(text));
                DataCursor(
                    row,
                    cursor
                        .1
                        .min(text.lines()[row].chars().count().saturating_sub(1)),
                )
            }
            KeyCode::Char('k') | KeyCode::Up => {
                let row = cursor.0.saturating_sub(1);
                DataCursor(
                    row,
                    cursor
                        .1
                        .min(text.lines()[row].chars().count().saturating_sub(1)),
                )
            }
            KeyCode::Char('0') | KeyCode::Home => DataCursor(cursor.0, 0),
            KeyCode::Char('^') => DataCursor(cursor.0, Self::first_nonblank(text, cursor.0)),
            KeyCode::Char('$') | KeyCode::End => DataCursor(cursor.0, last_column),
            KeyCode::Char('w') => return Some(CursorMove::WordForward),
            KeyCode::Char('b') => return Some(CursorMove::WordBack),
            KeyCode::Char('e') => return Some(CursorMove::WordEnd),
            _ => return None,
        };

        Some(CursorMove::Jump(
            u16::try_from(target.0).unwrap_or(u16::MAX),
            u16::try_from(target.1).unwrap_or(u16::MAX),
        ))
    }

    fn handle_control_key(&mut self, key: KeyEvent, text: &mut TextArea<'static>) {
        if !key.modifiers.contains(KeyModifiers::CONTROL)
            || key.modifiers.contains(KeyModifiers::ALT)
        {
            return;
        }

        match key.code {
            KeyCode::Char('o') if self.mode == Mode::Normal => self.visit_jump(text, true),
            KeyCode::Char('i') if self.mode == Mode::Normal => self.visit_jump(text, false),
            KeyCode::Char('e') => text.scroll((1, 0)),
            KeyCode::Char('y') => text.scroll((-1, 0)),
            KeyCode::Char('d') => text.scroll(Scrolling::HalfPageDown),
            KeyCode::Char('u') => text.scroll(Scrolling::HalfPageUp),
            KeyCode::Char('f') => text.scroll(Scrolling::PageDown),
            KeyCode::Char('b') => text.scroll(Scrolling::PageUp),
            KeyCode::Char('r') => {
                text.cancel_selection();
                text.redo();
                self.mode = Mode::Normal;
            }
            _ => {}
        }
    }

    fn enter_insert(&mut self, code: KeyCode, text: &mut TextArea<'static>) {
        text.cancel_selection();
        match code {
            KeyCode::Char('a') => {
                let cursor = text.cursor();
                if cursor.1 < text.lines()[cursor.0].chars().count() {
                    text.move_cursor(CursorMove::Forward);
                }
            }
            KeyCode::Char('I') => Self::jump(
                text,
                DataCursor(text.cursor().0, Self::first_nonblank(text, text.cursor().0)),
            ),
            KeyCode::Char('A') => text.move_cursor(CursorMove::End),
            KeyCode::Char('o') => {
                text.move_cursor(CursorMove::End);
                text.insert_newline();
            }
            KeyCode::Char('O') => {
                text.move_cursor(CursorMove::Head);
                text.insert_newline();
                text.move_cursor(CursorMove::Up);
            }
            _ => {}
        }

        self.mode = Mode::Insert;
    }
}
