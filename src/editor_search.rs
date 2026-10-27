use ratatui::{
    Frame,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
    layout::{Constraint, Layout, Rect},
    style::Style,
    widgets::Paragraph,
};
use ratatui_textarea::{CursorMove, DataCursor, TextArea};
use texo::theme;

const SEARCH_LABEL_WIDTH: u16 = 6;
const SEARCH_HINT_WIDTH: u16 = 31;

pub(super) struct Search {
    input: TextArea<'static>,
    pub(super) open: bool,
    notice: &'static str,
    jump_origin: Option<DataCursor>,
    vim_prompt: bool,
    backwards: bool,
    previous_pattern: String,
    previous_query: String,
    previous_backwards: bool,
    vim_edited: bool,
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
            vim_prompt: false,
            backwards: false,
            previous_pattern: String::new(),
            previous_query: String::new(),
            previous_backwards: false,
            vim_edited: false,
        }
    }
}

impl Search {
    pub(super) fn start(&mut self, text: &mut TextArea<'static>) {
        self.vim_prompt = false;
        text.cancel_selection();
        self.input.select_all();
        self.open = true;
        self.jump_origin = Some(text.cursor());
    }

    pub(super) fn start_vim(&mut self, text: &mut TextArea<'static>, backwards: bool) {
        self.start(text);
        self.vim_prompt = true;
        self.vim_edited = false;
        self.previous_query = self.input.lines().join("");
        self.previous_backwards = self.backwards;
        self.backwards = backwards;
        self.previous_pattern = text
            .search_pattern()
            .map_or_else(String::new, |pattern| pattern.as_str().to_owned());
    }

    pub(super) fn repeat_vim(&mut self, text: &mut TextArea<'static>, reverse: bool, count: usize) {
        for _ in 0..count {
            self.find(text, self.backwards != reverse, false);
        }
    }

    pub(super) fn take_jump_origin(&mut self) -> Option<DataCursor> {
        self.jump_origin.take()
    }

    pub(super) fn handle_key(&mut self, key: KeyEvent, text: &mut TextArea<'static>) {
        match key.code {
            KeyCode::Enter if self.vim_prompt => {
                if !self.vim_edited {
                    self.find(text, self.backwards, false);
                }
                self.open = false;
                self.vim_prompt = false;
            }
            KeyCode::Esc if self.vim_prompt => {
                self.open = false;
                self.vim_prompt = false;
                let _ = text.set_search_pattern(&self.previous_pattern);
                self.backwards = self.previous_backwards;
                self.input.select_all();
                self.input.insert_str(&self.previous_query);
                if let Some(origin) = self.jump_origin {
                    text.move_cursor(CursorMove::Jump(
                        u16::try_from(origin.0).unwrap_or(u16::MAX),
                        u16::try_from(origin.1).unwrap_or(u16::MAX),
                    ));
                }
            }
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
        self.vim_edited = true;
        let query = self.input.lines().join("");
        if text.set_search_pattern(regex::escape(&query)).is_err() {
            let _ = text.set_search_pattern("");
            self.notice = "Query too large";
            return;
        }

        if self.vim_prompt
            && let Some(origin) = self.jump_origin
        {
            text.move_cursor(CursorMove::Jump(
                u16::try_from(origin.0).unwrap_or(u16::MAX),
                u16::try_from(origin.1).unwrap_or(u16::MAX),
            ));
        }
        self.find(text, self.vim_prompt && self.backwards, !self.vim_prompt);
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
        let prompt = if self.vim_prompt {
            if self.backwards { "? " } else { "/ " }
        } else {
            "Find: "
        };
        frame.render_widget(Paragraph::new(prompt).style(theme::muted()), label);
        frame.render_widget(&self.input, input);
        if focused {
            crate::terminal::show_text_cursor(frame, &self.input, input);
        }
        let hint_text = if !self.notice.is_empty() {
            self.notice
        } else if focused && self.vim_prompt {
            "Enter confirm · Esc cancel"
        } else if focused {
            "Enter/⇧Enter next/prev · Esc end"
        } else {
            "Find paused"
        };
        frame.render_widget(Paragraph::new(hint_text).style(theme::muted()), hint);
    }
}
