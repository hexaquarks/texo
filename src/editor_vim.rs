// Adapted from ratatui-textarea 0.9.3 examples/vim.rs (MIT).
// Copyright (c) 2022 rhysd; Copyright (c) The Ratatui Developers.
// See LICENSE for the license.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui_textarea::{CursorMove, DataCursor, Scrolling, TextArea};

use super::clipboard::ClipboardAction;

#[path = "vim_jumps.rs"]
mod jumps;
#[path = "vim_motions.rs"]
mod motions;
#[path = "vim_objects.rs"]
mod objects;

use jumps::JumpList;
use objects::{ObjectRange, ObjectScope, text_object};

#[derive(Clone, Copy)]
enum Pending {
    Center,
    GoToTop,
    Object(ObjectScope),
    Find(FindMotion),
    Replace,
}

#[derive(Clone, Copy)]
struct FindMotion {
    forward: bool,
    till: bool,
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
    Center,
    Search(bool),
    SearchNext { reverse: bool, count: usize },
}

#[derive(Clone, Copy)]
struct LineSelection {
    anchor: DataCursor,
    cursor: DataCursor,
    moved: bool,
}

struct CompletedOperator {
    cursor: DataCursor,
    linewise_text: Option<String>,
    linewise_delete: bool,
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
    count: Option<usize>,
    operator_count: usize,
    preferred_column: Option<usize>,
    last_find: Option<(FindMotion, char)>,
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

    pub(super) fn leave_visual(&mut self) {
        self.visual_lines = None;
        self.mode = Mode::Normal;
    }

    pub(super) fn cancel_pending(&mut self, text: &mut TextArea<'static>) {
        self.pending = None;
        self.count = None;
        self.operator_count = 1;
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

        if !matches!(
            self.pending,
            Some(Pending::Object(_) | Pending::Find(_) | Pending::Replace)
        ) && let KeyCode::Char(character) = key.code
            && let Some(digit) = character.to_digit(10)
            && (digit != 0 || self.count.is_some())
        {
            // CursorMove::Jump uses u16 coordinates; bound counts to the same supported range.
            let count = self
                .count
                .unwrap_or(0)
                .saturating_mul(10)
                .saturating_add(digit as usize);
            self.count = Some(count.min(usize::from(u16::MAX)));
            return VimInput::Consumed;
        }

        if matches!(self.pending, Some(Pending::Center)) {
            self.cancel_pending(text);
            return if key.code == KeyCode::Char('z') {
                VimInput::Center
            } else {
                VimInput::Consumed
            };
        }
        if key.code == KeyCode::Char('z') && self.pending.is_none() && self.operator.is_none() {
            self.pending = Some(Pending::Center);
            return VimInput::Consumed;
        }

        if self.mode == Mode::VisualLine {
            return self.handle_visual_line(key.code, text);
        }

        let pending = self.pending.take();
        if let Some(Pending::Object(scope)) = pending {
            return self.handle_text_object(scope, key.code, text);
        }
        if let Some(Pending::Find(motion)) = pending {
            return self.find_character(motion, key.code, false, text);
        }
        if matches!(pending, Some(Pending::Replace)) {
            return self.replace_character(key.code, text);
        }
        let was_pending_g = matches!(pending, Some(Pending::GoToTop));
        if was_pending_g && key.code != KeyCode::Char('g') {
            self.cancel_pending(text);
            return VimInput::Consumed;
        }
        if let Some(motion) = Self::find_motion(key.code) {
            self.pending = Some(Pending::Find(motion));
            return VimInput::Consumed;
        }
        if matches!(key.code, KeyCode::Char(';' | ','))
            && let Some((mut motion, character)) = self.last_find
        {
            if key.code == KeyCode::Char(',') {
                motion.forward = !motion.forward;
            }
            let saved = self.last_find;
            let result = self.find_character(motion, KeyCode::Char(character), true, text);
            self.last_find = saved;
            return result;
        }
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
        if Self::is_motion(key.code) {
            let count = self.take_count();
            self.move_motion(key.code, count, text);
            return VimInput::Consumed;
        }
        if let Some(operator) = Operator::from_key(key.code) {
            if self.mode == Mode::Visual {
                Self::include_visual_endpoint(text);
                return self.finish_selection(operator, text);
            }

            text.cancel_selection();
            text.start_selection();
            self.operator_count = self.take_count();
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

        if code == KeyCode::Char('g') && !was_pending_g {
            self.pending = Some(Pending::GoToTop);
            return VimInput::Consumed;
        }

        let explicit_count = self.count.is_some() || self.operator_count > 1;
        let count = self.take_count().saturating_mul(self.operator_count.max(1));
        let origin = text.cursor();
        let row = origin.0;
        let last = Self::last_source_row(text);
        let line_target = match code {
            _ if Operator::from_key(code) == Some(operator) => Some((row + count - 1).min(last)),
            KeyCode::Char('j') | KeyCode::Down => Some((row + count).min(last)),
            KeyCode::Char('k') | KeyCode::Up => Some(row.saturating_sub(count)),
            KeyCode::Char('G') => Some(if explicit_count {
                count.saturating_sub(1).min(last)
            } else {
                last
            }),
            KeyCode::Char('g') if was_pending_g => Some(count.saturating_sub(1).min(last)),
            _ => None,
        };
        if let Some(target) = line_target {
            return self.line_operator(operator, row.min(target), row.max(target), text);
        }

        if !Self::is_motion(code) {
            self.cancel_pending(text);
            return VimInput::Consumed;
        }
        if operator == Operator::Change
            && matches!(code, KeyCode::Char('w' | 'W'))
            && text.lines()[row]
                .chars()
                .nth(origin.1)
                .is_some_and(|character| !character.is_whitespace())
        {
            // Vim treats cw as ce on a word, leaving the following whitespace intact.
            let object = if code == KeyCode::Char('W') { 'W' } else { 'w' };
            for _ in 1..count {
                self.move_motion(code, 1, text);
            }
            if let Some(range) = text_object(text, ObjectScope::Inner, object) {
                Self::jump(text, range.end);
            }
        } else {
            self.move_motion(code, count, text);
            if matches!(code, KeyCode::Char('e' | 'E' | '$') | KeyCode::End) {
                text.move_cursor(CursorMove::Forward);
            } else if matches!(code, KeyCode::Char('w' | 'W')) && text.cursor().0 > row {
                // A word delete ending at the next line's start leaves the newline intact.
                let end = text.cursor();
                if end.1 == Self::first_nonblank(text, end.0) {
                    Self::jump(
                        text,
                        DataCursor(end.0 - 1, text.lines()[end.0 - 1].chars().count()),
                    );
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

    pub(super) fn reset_document_state(&mut self) {
        self.jumps = JumpList::default();
        self.visual_lines = None;
        self.completed_operator = None;
        self.pending = None;
        self.operator = None;
        self.count = None;
        self.preferred_column = None;
        self.mode = Mode::Normal;
    }

    fn jump_to_line(&mut self, row: usize, text: &mut TextArea<'static>) {
        let from = text.cursor();
        Self::jump(text, DataCursor(row, Self::first_nonblank(text, row)));
        self.preferred_column = None;
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
        self.count = None;
        self.operator_count = 1;
        self.preferred_column = None;
        self.mode = if operator == Operator::Change {
            Mode::Insert
        } else {
            Mode::Normal
        };

        VimInput::Clipboard(operator.clipboard_action())
    }

    pub(super) fn command_pending(&self) -> bool {
        self.pending.is_some() || self.operator.is_some() || self.count.is_some()
    }

    pub(super) fn repeat_count(&mut self, key: KeyEvent) -> Option<Option<usize>> {
        if self.mode == Mode::Normal
            && self.pending.is_none()
            && self.operator.is_none()
            && key.code == KeyCode::Char('.')
            && !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            Some(self.count.take())
        } else {
            None
        }
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
            linewise_delete: false,
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
        if operation.linewise_delete {
            let row = text.cursor().0.min(Self::last_source_row(text));
            Self::jump(text, DataCursor(row, Self::first_nonblank(text, row)));
        }
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
        let origin = text.cursor();
        Self::select_object(text, &ObjectRange { start, end });
        let cursor = if operator == Operator::Yank {
            self.visual_lines.map_or(origin, |selection| {
                DataCursor(
                    first,
                    if selection.cursor.0 < selection.anchor.0 || !selection.moved {
                        selection.cursor.1
                    } else {
                        0
                    },
                )
            })
        } else {
            DataCursor(first, 0)
        };
        self.completed_operator = Some(CompletedOperator {
            cursor,
            linewise_text: Some(contents),
            linewise_delete: operator == Operator::Delete,
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
        if matches!(self.pending, Some(Pending::GoToTop)) && code != KeyCode::Char('g') {
            self.cancel_pending(text);
            return VimInput::Consumed;
        }

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

        let explicit_count = self.count.is_some();
        let count = if code == KeyCode::Char('g') && self.pending.is_none() {
            self.count.unwrap_or(1)
        } else {
            self.take_count()
        };
        match code {
            KeyCode::Char('J') => {
                self.join_lines(text);
                return VimInput::Consumed;
            }
            KeyCode::Char('p' | 'P') => {
                self.paste_over_lines(selection, code == KeyCode::Char('P'), count, text);
                return VimInput::Consumed;
            }
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
                selection.cursor.0 = selection.cursor.0.saturating_sub(count);
            }
            KeyCode::Char('j') | KeyCode::Down => {
                selection.cursor.0 = (selection.cursor.0 + count).min(Self::last_source_row(text));
            }
            KeyCode::Char('G') => {
                selection.cursor.0 = if explicit_count {
                    count.saturating_sub(1).min(Self::last_source_row(text))
                } else {
                    Self::last_source_row(text)
                }
            }
            KeyCode::Char('g') => {
                if matches!(self.pending.take(), Some(Pending::GoToTop)) {
                    selection.cursor.0 = count.saturating_sub(1).min(Self::last_source_row(text));
                } else {
                    self.pending = Some(Pending::GoToTop);
                }
            }
            _ => {
                self.pending = None;
            }
        }
        selection.moved = true;
        self.visual_lines = Some(selection);
        self.refresh_line_selection(text);
        VimInput::Consumed
    }

    fn paste_over_lines(
        &mut self,
        selection: LineSelection,
        preserve_register: bool,
        count: usize,
        text: &mut TextArea<'static>,
    ) {
        let first = selection.anchor.0.min(selection.cursor.0);
        let last = selection.anchor.0.max(selection.cursor.0);
        let old_lines = format!("{}\n", text.lines()[first..=last].join("\n"));
        let contents = text.yank_text().repeat(count);
        if contents.is_empty() {
            return;
        }
        let contents = if self.linewise_register {
            contents.strip_suffix('\n').unwrap_or(&contents)
        } else {
            &contents
        };
        let (end, replacement) = if last + 1 < text.lines().len() {
            (DataCursor(last + 1, 0), format!("{contents}\n"))
        } else {
            (
                DataCursor(last, text.lines()[last].chars().count()),
                contents.to_owned(),
            )
        };
        Self::select_object(
            text,
            &ObjectRange {
                start: DataCursor(first, 0),
                end,
            },
        );
        text.insert_str(replacement);
        Self::jump(text, DataCursor(first, 0));
        if !preserve_register {
            text.set_yank_text(old_lines);
            self.linewise_register = true;
        }
        self.mode = Mode::Normal;
        self.visual_lines = None;
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
            KeyCode::Char('g') if was_pending_g => {
                let row = self
                    .take_count()
                    .saturating_sub(1)
                    .min(Self::last_source_row(text));
                self.jump_to_line(row, text);
            }
            KeyCode::Char('g') => self.pending = Some(Pending::GoToTop),
            KeyCode::Char('G') => {
                let row = self
                    .count
                    .take()
                    .map_or(Self::last_source_row(text), |count| {
                        count.saturating_sub(1).min(Self::last_source_row(text))
                    });
                self.jump_to_line(row, text);
            }
            KeyCode::Char('/' | '?') if self.mode == Mode::Normal => {
                return VimInput::Search(code == KeyCode::Char('?'));
            }
            KeyCode::Char('n' | 'N') if self.mode == Mode::Normal => {
                return VimInput::SearchNext {
                    reverse: code == KeyCode::Char('N'),
                    count: self.take_count(),
                };
            }
            KeyCode::Char('r') if self.mode == Mode::Normal => {
                self.pending = Some(Pending::Replace)
            }
            KeyCode::Char('i' | 'a' | 'I' | 'A' | 'o' | 'O') if self.mode == Mode::Normal => {
                self.enter_insert(code, text);
            }
            KeyCode::Char('v') if self.mode == Mode::Visual => {
                text.cancel_selection();
                self.mode = Mode::Normal;
            }
            KeyCode::Char('V') => {
                let cursor = text.cursor();
                let anchor = text.selection_range().map_or(cursor, |(start, end)| {
                    if cursor == start {
                        end.into()
                    } else {
                        start.into()
                    }
                });
                let count = self.take_count();
                self.visual_lines = Some(LineSelection {
                    anchor,
                    cursor: DataCursor(
                        (cursor.0 + count - 1).min(Self::last_source_row(text)),
                        cursor.1,
                    ),
                    moved: false,
                });
                self.mode = Mode::VisualLine;
                self.refresh_line_selection(text);
            }
            KeyCode::Char('v') if self.mode == Mode::Normal => {
                text.start_selection();
                self.mode = Mode::Visual;
            }
            KeyCode::Char('Y' | 'S') if self.mode == Mode::Normal => {
                let row = text.cursor().0;
                let last = (row + self.take_count() - 1).min(Self::last_source_row(text));
                let operator = if code == KeyCode::Char('Y') {
                    Operator::Yank
                } else {
                    Operator::Change
                };
                return self.line_operator(operator, row, last, text);
            }
            KeyCode::Char('D' | 'C' | 'S') => {
                if self.mode == Mode::Visual {
                    Self::include_visual_endpoint(text);
                } else {
                    text.cancel_selection();
                    if code == KeyCode::Char('S') {
                        text.move_cursor(CursorMove::Head);
                    }
                    text.start_selection();
                    text.move_cursor(CursorMove::End);
                }
                let operator = if code == KeyCode::Char('D') {
                    Operator::Delete
                } else {
                    Operator::Change
                };
                return self.finish_selection(operator, text);
            }
            KeyCode::Char('x' | 's') if self.mode == Mode::Normal => {
                let cursor = text.cursor();
                let count = self.take_count();
                let length = text.lines()[cursor.0].chars().count();
                if cursor.1 < length {
                    text.start_selection();
                    Self::jump(text, DataCursor(cursor.0, (cursor.1 + count).min(length)));
                    let operator = if code == KeyCode::Char('s') {
                        Operator::Change
                    } else {
                        Operator::Delete
                    };
                    return self.finish_selection(operator, text);
                }
            }
            KeyCode::Char('J') => {
                self.join_lines(text);
            }
            KeyCode::Char('p' | 'P') => {
                let count = self.take_count();
                if self.mode == Mode::Visual {
                    Self::include_visual_endpoint(text);
                }
                if self.linewise_register && self.mode == Mode::Normal {
                    let row = text.cursor().0;
                    if code == KeyCode::Char('P') {
                        Self::jump(text, DataCursor(row, 0));
                        for _ in 0..count {
                            text.paste();
                        }
                        Self::jump(text, DataCursor(row, 0));
                    } else {
                        Self::jump(text, DataCursor(row, text.lines()[row].chars().count()));
                        let contents = text.yank_text().repeat(count);
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
                    for _ in 0..count {
                        text.paste();
                    }
                    if text.cursor().1 > 0 {
                        text.move_cursor(CursorMove::Back);
                    }
                }
                self.mode = Mode::Normal;
                Self::clamp_normal_cursor(text);
            }
            KeyCode::Char('u') => {
                let count = self.take_count();
                self.move_history(text, false, count);
            }
            KeyCode::PageUp => text.scroll(Scrolling::PageUp),
            KeyCode::PageDown => text.scroll(Scrolling::PageDown),
            _ => {}
        }

        if self.pending.is_none() {
            self.count = None;
        }
        VimInput::Consumed
    }

    fn join_lines(&mut self, text: &mut TextArea<'static>) {
        let row = text.cursor().0;
        let (first, last) = if self.is_visual() {
            text.selection_range()
                .map_or((row, row), |(start, end)| (start.0, end.0))
        } else {
            let count = self.take_count().max(2);
            (row, (row + count - 1).min(Self::last_source_row(text)))
        };
        text.cancel_selection();
        let mut join_column = text.lines()[first].chars().count();
        for _ in first..last {
            let column = text.lines()[first].chars().count();
            join_column = column;
            let next = &text.lines()[first + 1];
            let indentation = next
                .chars()
                .take_while(|character| character.is_whitespace())
                .count();
            let separator = if text.lines()[first].ends_with(char::is_whitespace)
                || text.lines()[first].is_empty()
                || next.trim_start().starts_with(')')
            {
                ""
            } else {
                " "
            };
            Self::select_object(
                text,
                &ObjectRange {
                    start: DataCursor(first, column),
                    end: DataCursor(first + 1, indentation),
                },
            );
            text.insert_str(separator);
        }
        Self::jump(text, DataCursor(first, join_column));
        Self::clamp_normal_cursor(text);
        self.visual_lines = None;
        self.mode = Mode::Normal;
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
                self.move_history(text, true, 1);
            }
            _ => {}
        }
        if self.mode == Mode::VisualLine
            && let Some(mut selection) = self.visual_lines
        {
            selection.cursor.0 = text.cursor().0.min(Self::last_source_row(text));
            selection.moved = true;
            self.visual_lines = Some(selection);
            self.refresh_line_selection(text);
        }
    }

    fn move_history(&mut self, text: &mut TextArea<'static>, redo: bool, count: usize) {
        text.cancel_selection();
        for _ in 0..count {
            let before = text.lines().to_vec();
            let changed = if redo { text.redo() } else { text.undo() };
            if !changed {
                break;
            }
            // Native history stores the selection endpoint. Vim places the cursor at
            // the changed text instead; derive that position without owning undo data.
            let row = before
                .iter()
                .zip(text.lines())
                .position(|(old, new)| old != new)
                .unwrap_or(before.len().min(text.lines().len()).saturating_sub(1));
            let column = before
                .get(row)
                .zip(text.lines().get(row))
                .map_or(0, |(old, new)| {
                    old.chars()
                        .zip(new.chars())
                        .take_while(|(old, new)| old == new)
                        .count()
                });
            Self::jump(text, DataCursor(row, column));
            Self::clamp_normal_cursor(text);
        }
        self.mode = Mode::Normal;
        self.visual_lines = None;
        self.preferred_column = None;
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
                let row = text.cursor().0;
                text.move_cursor(CursorMove::Head);
                text.insert_newline();
                Self::jump(text, DataCursor(row, 0));
            }
            _ => {}
        }

        self.mode = Mode::Insert;
    }
}
