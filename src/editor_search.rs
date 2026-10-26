use ratatui::{
    Frame,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
    layout::{Constraint, Layout, Rect},
    style::Style,
    widgets::Paragraph,
};
use ratatui_textarea::{DataCursor, TextArea};
use texo::theme;

const SEARCH_LABEL_WIDTH: u16 = 6;
const SEARCH_HINT_WIDTH: u16 = 31;

pub(super) struct Search {
    input: TextArea<'static>,
    pub(super) open: bool,
    notice: &'static str,
    jump_origin: Option<DataCursor>,
}

impl Default for Search {
    fn default() -> Self {
        let mut input = TextArea::default();
        input.set_style(theme::text());
        input.set_cursor_line_style(theme::text());
        input.set_cursor_style(Style::default());
        input.set_selection_style(theme::text().bg(theme::SELECTION));

        Self {
            input,
            open: false,
            notice: "",
            jump_origin: None,
        }
    }
}

impl Search {
    pub(super) fn start(&mut self, text: &mut TextArea<'static>) {
        text.cancel_selection();
        self.input.select_all();
        self.open = true;
        self.jump_origin = Some(text.cursor());
    }

    pub(super) fn take_jump_origin(&mut self) -> Option<DataCursor> {
        self.jump_origin.take()
    }

    pub(super) fn handle_key(&mut self, key: KeyEvent, text: &mut TextArea<'static>) {
        match key.code {
            KeyCode::Esc => {
                self.open = false;
                self.notice = "";
                let _ = text.set_search_pattern("");
            }
            KeyCode::Enter => {
                self.find(text, key.modifiers.contains(KeyModifiers::SHIFT), false);
            }
            KeyCode::Char('g') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.find(text, key.modifiers.contains(KeyModifiers::SHIFT), false);
            }
            KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.input.select_all();
            }
            _ => {
                if self.input.input(key) {
                    self.update_pattern(text);
                }
            }
        }
    }

    pub(super) fn paste(&mut self, value: &str, text: &mut TextArea<'static>) {
        self.input.insert_str(value.replace(['\r', '\n'], " "));
        self.update_pattern(text);
    }

    fn update_pattern(&mut self, text: &mut TextArea<'static>) {
        let query = self.input.lines().join("");
        if text.set_search_pattern(regex::escape(&query)).is_err() {
            let _ = text.set_search_pattern("");
            self.notice = "Query too large";
            return;
        }

        self.find(text, false, true);
    }

    pub(super) fn find(
        &mut self,
        text: &mut TextArea<'static>,
        backwards: bool,
        include_cursor: bool,
    ) {
        if text.search_pattern().is_none() {
            self.notice = "";
            return;
        }

        let found = if backwards {
            text.search_back(include_cursor)
        } else {
            text.search_forward(include_cursor)
        };
        self.notice = if found { "" } else { "No match" };
    }

    pub(super) fn draw(&mut self, frame: &mut Frame, area: Rect, focused: bool) {
        let [label, input, hint] = Layout::horizontal([
            Constraint::Length(SEARCH_LABEL_WIDTH),
            Constraint::Min(1),
            Constraint::Length(SEARCH_HINT_WIDTH.min(area.width / 2)),
        ])
        .areas(area);
        frame.render_widget(Paragraph::new("Find: ").style(theme::muted()), label);
        frame.render_widget(&self.input, input);
        if focused {
            crate::terminal::show_text_cursor(frame, &self.input, input);
        }
        let hint_text = if !self.notice.is_empty() {
            self.notice
        } else if focused {
            "Enter/⇧Enter next/prev · Esc end"
        } else {
            "Find paused"
        };
        frame.render_widget(Paragraph::new(hint_text).style(theme::muted()), hint);
    }
}
