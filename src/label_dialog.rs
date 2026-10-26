use std::path::PathBuf;

use ratatui::{
    Frame,
    crossterm::event::{Event, KeyCode, KeyModifiers, MouseButton, MouseEventKind},
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Clear, List, ListItem, ListState, Padding, Paragraph},
};
use ratatui_textarea::TextArea;
use texo::theme;

use crate::{
    resume_label::{LabelColor, MAX_LABEL_CHARS, MAX_LABEL_WORDS, PRESET_LABELS, ResumeLabel},
    resume_project::Project,
};

const DIALOG_WIDTH: u16 = 52;
const DIALOG_HEIGHT: u16 = 17;
const NO_LABEL_INDEX: usize = 0;
const CUSTOM_LABEL_INDEX: usize = PRESET_LABELS.len() + 1;

pub enum LabelResult {
    Continue,
    Cancel,
    Apply(Option<ResumeLabel>),
}

pub struct LabelDialog {
    pub project_root: PathBuf,
    project_name: String,
    selection: ListState,
    custom: bool,
    name: TextArea<'static>,
    color_index: usize,
    color_focused: bool,
    pub error: String,
}

impl LabelDialog {
    pub fn new(project: &Project) -> Self {
        let label = project.manifest.label.as_ref();
        let selected = label.map_or(NO_LABEL_INDEX, |label| {
            PRESET_LABELS
                .iter()
                .position(|(name, color)| *name == label.name && *color == label.color)
                .map_or(CUSTOM_LABEL_INDEX, |index| index + 1)
        });
        let mut name = TextArea::new(vec![
            label.map_or_else(String::new, |label| label.name.clone()),
        ]);
        name.set_style(theme::text().bg(theme::ELEVATED));
        name.set_cursor_line_style(Style::default());
        name.set_selection_style(Style::default().bg(theme::SELECTION));
        name.select_all();

        let color_index = label
            .and_then(|label| {
                LabelColor::ALL
                    .iter()
                    .position(|color| *color == label.color)
            })
            .unwrap_or(0);

        Self {
            project_root: project.root.clone(),
            project_name: project.manifest.name.clone(),
            selection: ListState::default().with_selected(Some(selected)),
            custom: false,
            name,
            color_index,
            color_focused: false,
            error: String::new(),
        }
    }

    fn panel(screen: Rect) -> Rect {
        let width = screen.width.min(DIALOG_WIDTH);
        let height = screen.height.min(DIALOG_HEIGHT);
        Rect::new(
            screen.x + (screen.width - width) / 2,
            screen.y + (screen.height - height) / 2,
            width,
            height,
        )
    }

    fn block() -> Block<'static> {
        Block::bordered()
            .title(" Resume label ")
            .padding(Padding::horizontal(1))
            .style(theme::text())
            .border_style(theme::pane_title(true))
    }

    fn rows(screen: Rect) -> [Rect; 4] {
        Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(2),
            Constraint::Length(1),
        ])
        .areas(Self::block().inner(Self::panel(screen)))
    }

    fn custom_rows(form: Rect) -> [Rect; 5] {
        Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(2),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .areas(form)
    }

    fn color_areas(row: Rect) -> [Rect; LabelColor::ALL.len()] {
        Layout::horizontal(
            [Constraint::Ratio(1, LabelColor::ALL.len() as u32); LabelColor::ALL.len()],
        )
        .areas(row)
    }

    pub fn handle_event(&mut self, event: Event, screen: Rect) -> LabelResult {
        match event {
            Event::Key(key) if key.code == KeyCode::Esc => {
                if self.custom {
                    self.custom = false;
                    self.error.clear();
                } else {
                    return LabelResult::Cancel;
                }
            }
            Event::Key(key) if key.code == KeyCode::Enter => return self.apply(),
            Event::Key(key) if self.custom => {
                self.error.clear();
                match key.code {
                    KeyCode::Tab | KeyCode::BackTab => self.color_focused = !self.color_focused,
                    KeyCode::Left if self.color_focused => {
                        self.color_index = self.color_index.saturating_sub(1);
                    }
                    KeyCode::Right if self.color_focused => {
                        self.color_index = (self.color_index + 1).min(LabelColor::ALL.len() - 1);
                    }
                    KeyCode::Char('a')
                        if !self.color_focused && key.modifiers == KeyModifiers::CONTROL =>
                    {
                        self.name.select_all();
                    }
                    _ if !self.color_focused => {
                        self.name.input(key);
                    }
                    _ => {}
                }
            }
            Event::Key(key) => {
                let selected = self.selection.selected().unwrap_or(NO_LABEL_INDEX);
                match key.code {
                    KeyCode::Up | KeyCode::Char('k') => {
                        self.selection.select(Some(selected.saturating_sub(1)))
                    }
                    KeyCode::Down | KeyCode::Char('j') => self
                        .selection
                        .select(Some((selected + 1).min(CUSTOM_LABEL_INDEX))),
                    _ => {}
                }
            }
            Event::Paste(text) if self.custom && !self.color_focused => {
                self.error.clear();
                self.name.insert_str(text.replace(['\n', '\r'], " "));
            }
            Event::Mouse(mouse) if mouse.kind == MouseEventKind::Down(MouseButton::Left) => {
                let [_, form, _, _] = Self::rows(screen);
                let position = (mouse.column, mouse.row).into();
                if self.custom {
                    let [_, name, _, colors, _] = Self::custom_rows(form);
                    if name.contains(position) {
                        self.color_focused = false;
                    }
                    for (index, area) in Self::color_areas(colors).iter().enumerate() {
                        if area.contains(position) {
                            self.color_focused = true;
                            self.color_index = index;
                        }
                    }
                } else if form.contains(position) {
                    let index = self.selection.offset() + usize::from(mouse.row - form.y);
                    if index <= CUSTOM_LABEL_INDEX {
                        self.selection.select(Some(index));
                        return self.apply();
                    }
                }
            }
            _ => {}
        }

        LabelResult::Continue
    }

    fn apply(&mut self) -> LabelResult {
        if self.custom {
            return match ResumeLabel::new(
                &self.name.lines().join(""),
                LabelColor::ALL[self.color_index],
            ) {
                Ok(label) => LabelResult::Apply(Some(label)),
                Err(error) => {
                    self.error = error.to_string();
                    LabelResult::Continue
                }
            };
        }

        match self.selection.selected().unwrap_or(NO_LABEL_INDEX) {
            NO_LABEL_INDEX => LabelResult::Apply(None),
            CUSTOM_LABEL_INDEX => {
                self.custom = true;
                self.color_focused = false;
                LabelResult::Continue
            }
            index => {
                let (name, color) = PRESET_LABELS[index - 1];
                LabelResult::Apply(Some(ResumeLabel {
                    name: name.into(),
                    color,
                }))
            }
        }
    }

    pub fn draw(&mut self, frame: &mut Frame) {
        let screen = frame.area();
        let area = Self::panel(screen);
        frame.render_widget(Clear, area);
        frame.render_widget(Self::block(), area);
        let [title, form, error, help] = Self::rows(screen);
        frame.render_widget(
            Paragraph::new(self.project_name.as_str()).style(theme::muted()),
            title,
        );

        if self.custom {
            let [name_label, name, limits, colors, _] = Self::custom_rows(form);
            self.name.set_cursor_style(if self.color_focused {
                Style::default()
            } else {
                theme::text().bg(theme::ACCENT).fg(theme::BACKGROUND)
            });
            frame.render_widget(Paragraph::new("Custom label"), name_label);
            frame.render_widget(&self.name, name);
            frame.render_widget(
                Paragraph::new(format!(
                    "Up to {MAX_LABEL_WORDS} words · {MAX_LABEL_CHARS} characters"
                ))
                .style(theme::muted()),
                limits,
            );
            for (index, (color, area)) in LabelColor::ALL
                .into_iter()
                .zip(Self::color_areas(colors))
                .enumerate()
            {
                let mut style = Style::default().fg(color.color());
                if index == self.color_index {
                    style = style.bg(theme::SELECTION).add_modifier(Modifier::BOLD);
                }
                frame.render_widget(Paragraph::new(color.name()).style(style), area);
            }
        } else {
            let mut items = vec![ListItem::new("No label")];
            items.extend(PRESET_LABELS.iter().map(|(name, color)| {
                ListItem::new(Line::from(Span::styled(
                    *name,
                    Style::default().fg(color.color()),
                )))
            }));
            items.push(ListItem::new("Custom…"));
            frame.render_stateful_widget(
                List::new(items).highlight_style(Style::default().bg(theme::SELECTION)),
                form,
                &mut self.selection,
            );
        }

        frame.render_widget(
            Paragraph::new(self.error.as_str()).style(theme::text().fg(theme::ERROR)),
            error,
        );
        let bindings = if self.custom {
            vec![("Enter", "Save"), ("Tab", "Name/color"), ("Esc", "Back")]
        } else {
            vec![("Enter", "Select"), ("Esc", "Cancel")]
        };
        frame.render_widget(Paragraph::new(theme::shortcuts(&bindings)), help);
    }
}
