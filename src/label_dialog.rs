use std::path::PathBuf;

use ratatui::{
    Frame,
    crossterm::event::{Event, KeyCode, KeyModifiers, MouseButton, MouseEventKind},
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, BorderType, Borders, Clear, HighlightSpacing, List, ListItem, ListState, Padding,
        Paragraph,
    },
};
use ratatui_textarea::TextArea;
use texo::theme;

use crate::{
    dialog_ui::field,
    resume_label::{LabelColor, MAX_LABEL_CHARS, MAX_LABEL_WORDS, ResumeLabel},
    resume_project::Project,
};

const DIALOG_WIDTH: u16 = 52;
const DIALOG_HEIGHT: u16 = 21;
const NO_LABEL_INDEX: usize = 0;

pub enum LabelResult {
    Continue,
    Cancel,
    Apply(Option<ResumeLabel>),
}

pub struct LabelDialog {
    pub project_root: PathBuf,
    project_name: String,
    pub choices: Vec<ResumeLabel>,
    selection: ListState,
    custom: bool,
    name: TextArea<'static>,
    color_index: usize,
    color_focused: bool,
    pub error: String,
}

impl LabelDialog {
    pub fn new(project: &Project, choices: Vec<ResumeLabel>) -> Self {
        let label = project.manifest.label.as_ref();
        let selected = label.map_or(NO_LABEL_INDEX, |label| {
            choices
                .iter()
                .position(|choice| choice == label)
                .map_or(choices.len() + 1, |index| index + 1)
        });
        let mut name = TextArea::new(vec![
            label.map_or_else(String::new, |label| label.name.clone()),
        ]);
        name.set_style(theme::dialog());
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
            choices,
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
            .border_type(BorderType::Rounded)
            .padding(Padding::uniform(1))
            .style(theme::dialog())
            .border_style(theme::dialog_title())
    }

    fn rows(screen: Rect) -> [Rect; 4] {
        Layout::vertical([
            Constraint::Length(2),
            Constraint::Min(0),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .areas(Self::block().inner(Self::panel(screen)))
    }

    fn custom_rows(form: Rect) -> [Rect; 5] {
        Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(3),
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
                        .select(Some((selected + 1).min(self.choices.len() + 1))),
                    KeyCode::Home => self.selection.select(Some(NO_LABEL_INDEX)),
                    KeyCode::End => self.selection.select(Some(self.choices.len() + 1)),
                    KeyCode::PageUp | KeyCode::PageDown => {
                        let page = usize::from(Self::rows(screen)[1].height.max(1));
                        self.move_selection(key.code == KeyCode::PageDown, page);
                    }
                    _ => {}
                }
            }
            Event::Mouse(mouse)
                if !self.custom
                    && matches!(
                        mouse.kind,
                        MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
                    ) =>
            {
                if Self::rows(screen)[1].contains((mouse.column, mouse.row).into()) {
                    self.move_selection(mouse.kind == MouseEventKind::ScrollDown, 1);
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
                    if index <= self.choices.len() + 1 {
                        self.selection.select(Some(index));
                        return self.apply();
                    }
                }
            }
            _ => {}
        }

        LabelResult::Continue
    }

    fn move_selection(&mut self, down: bool, amount: usize) {
        let selected = self.selection.selected().unwrap_or(NO_LABEL_INDEX);
        let next = if down {
            selected.saturating_add(amount).min(self.choices.len() + 1)
        } else {
            selected.saturating_sub(amount)
        };
        self.selection.select(Some(next));
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
            index if index == self.choices.len() + 1 => {
                self.custom = true;
                self.color_focused = false;
                LabelResult::Continue
            }
            index => LabelResult::Apply(Some(self.choices[index - 1].clone())),
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
            frame.render_widget(Paragraph::new("Custom label"), name_label);
            field(frame, name, &self.name, !self.color_focused);
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
            items.extend(self.choices.iter().map(|label| {
                ListItem::new(Line::from(Span::styled(
                    label.name.as_str(),
                    Style::default().fg(label.color.color()),
                )))
            }));
            items.push(ListItem::new("Custom…"));
            frame.render_stateful_widget(
                List::new(items)
                    .highlight_symbol("› ")
                    .highlight_spacing(HighlightSpacing::Always)
                    .highlight_style(Style::default().bg(theme::SELECTION)),
                form,
                &mut self.selection,
            );
        }

        if !self.custom && self.error.is_empty() {
            let position = self.selection.selected().unwrap_or(0) + 1;
            let count = self.choices.len() + 2;
            frame.render_widget(
                Paragraph::new(format!("{position}/{count}")).style(theme::muted()),
                error,
            );
        } else {
            frame.render_widget(
                Paragraph::new(self.error.as_str()).style(theme::dialog().fg(theme::ERROR)),
                error,
            );
        }
        let bindings = if self.custom {
            vec![("Tab", "move"), ("Enter", "save"), ("Esc", "back")]
        } else {
            vec![("j/k", "move"), ("Enter", "select"), ("Esc", "close")]
        };
        frame.render_widget(
            Block::default()
                .borders(Borders::TOP)
                .border_style(theme::muted()),
            Rect::new(help.x, help.y.saturating_sub(1), help.width, 1),
        );
        frame.render_widget(Paragraph::new(theme::shortcuts(&bindings)), help);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{
        Terminal,
        backend::TestBackend,
        crossterm::event::{KeyEvent, MouseEvent},
    };

    #[test]
    fn long_label_picker_scrolls_and_clicks_the_visible_choice() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let project = crate::resume_project::create(directory.path(), "General")?;
        let choices = (0..40)
            .map(|index| ResumeLabel::new(&format!("Role {index}"), LabelColor::Teal))
            .collect::<anyhow::Result<Vec<_>>>()?;
        let mut dialog = LabelDialog::new(&project, choices);
        let screen = Rect::new(0, 0, 60, 20);
        let mut terminal = Terminal::new(TestBackend::new(screen.width, screen.height))?;
        let key = |code| Event::Key(KeyEvent::new(code, KeyModifiers::NONE));
        dialog.handle_event(key(KeyCode::End), screen);
        terminal.draw(|frame| dialog.draw(frame))?;
        assert!(dialog.selection.offset() > 0);

        dialog.handle_event(key(KeyCode::PageUp), screen);
        assert!(dialog.selection.selected().unwrap() < dialog.choices.len());
        dialog.handle_event(key(KeyCode::PageDown), screen);
        let form = LabelDialog::rows(screen)[1];
        dialog.handle_event(
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::ScrollUp,
                column: form.x,
                row: form.y,
                modifiers: KeyModifiers::NONE,
            }),
            screen,
        );
        assert_eq!(dialog.selection.selected(), Some(dialog.choices.len()));
        terminal.draw(|frame| dialog.draw(frame))?;

        let offset = dialog.selection.offset();
        let expected = dialog.choices[offset - 1].clone();
        let result = dialog.handle_event(
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: form.x,
                row: form.y,
                modifiers: KeyModifiers::NONE,
            }),
            screen,
        );
        assert!(matches!(result, LabelResult::Apply(Some(label)) if label == expected));
        Ok(())
    }
}
