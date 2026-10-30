use ratatui::{
    Frame,
    layout::{Alignment, Margin, Rect},
    style::Modifier,
    widgets::{Block, BorderType, Borders, Clear, Padding, Paragraph},
};
use ratatui_textarea::TextArea;
use texo::theme;

const FORM_WIDTH: u16 = 76;
const FOOTER_HEIGHT: u16 = 6;

#[derive(Clone, Copy)]
pub enum DialogAction {
    Cancel,
    Submit,
}

/// Keep the actions visible while the focused field scrolls into a short terminal.
pub struct FormLayout {
    pub area: Rect,
    pub body: Rect,
    pub buttons: [Rect; 2],
    pub help: Rect,
    scroll: u16,
}

impl FormLayout {
    pub fn new(screen: Rect, rows: u16, focus_row: u16, action: &str) -> Self {
        let width = screen.width.min(FORM_WIDTH);
        let height = screen.height.min(rows + FOOTER_HEIGHT + 4);
        let area = Rect::new(
            screen.x + (screen.width - width) / 2,
            screen.y + (screen.height - height) / 2,
            width,
            height,
        );
        let inner = area.inner(Margin::new(2, 1));
        let body = Rect {
            height: inner.height.saturating_sub(FOOTER_HEIGHT),
            ..inner
        };
        let button_row = Rect::new(inner.x, body.bottom(), inner.width, inner.height.min(3));
        let action_width = (action.chars().count() as u16 + 4).min(button_row.width / 2);
        let cancel_width = 12.min(button_row.width.saturating_sub(action_width + 2));
        let buttons = [
            Rect::new(
                button_row
                    .right()
                    .saturating_sub(action_width + cancel_width + 2),
                button_row.y,
                cancel_width,
                button_row.height,
            ),
            Rect::new(
                button_row.right().saturating_sub(action_width),
                button_row.y,
                action_width,
                button_row.height,
            ),
        ];
        let help = Rect::new(
            inner.x,
            inner.bottom().saturating_sub(1),
            inner.width,
            inner.height.min(1),
        );
        let scroll = focus_row
            .saturating_add(3)
            .saturating_sub(body.height)
            .min(rows.saturating_sub(body.height));

        Self {
            area,
            body,
            buttons,
            help,
            scroll,
        }
    }

    pub fn row(&self, row: u16, height: u16) -> Rect {
        if row < self.scroll {
            return Rect::default();
        }
        Rect::new(
            self.body.x,
            self.body.y + row - self.scroll,
            self.body.width,
            height,
        )
        .intersection(self.body)
    }

    pub fn draw(&self, frame: &mut Frame, title: &str) {
        frame.render_widget(Clear, self.area);
        frame.render_widget(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .title(format!(" {title} "))
                .style(theme::dialog())
                .border_style(theme::dialog_title()),
            self.area,
        );
        let rule = Rect::new(
            self.help.x,
            self.help.y.saturating_sub(1),
            self.help.width,
            1,
        );
        frame.render_widget(
            Block::default()
                .borders(Borders::TOP)
                .border_style(theme::muted()),
            rule,
        );
        frame.render_widget(
            Paragraph::new(theme::shortcuts(&[
                ("Tab", "move"),
                ("Enter", "activate"),
                ("Esc", "cancel"),
            ])),
            self.help,
        );
    }
}

pub enum ButtonStyle {
    Secondary,
    Primary,
    Destructive,
}

pub fn button(frame: &mut Frame, area: Rect, label: &str, focused: bool, kind: ButtonStyle) {
    let style = match kind {
        ButtonStyle::Destructive => theme::dialog().bg(theme::ERROR).fg(theme::BACKGROUND),
        ButtonStyle::Primary => theme::dialog().bg(theme::ACCENT).fg(theme::BACKGROUND),
        ButtonStyle::Secondary if focused => {
            theme::dialog().bg(theme::ACCENT).fg(theme::BACKGROUND)
        }
        ButtonStyle::Secondary => theme::dialog().bg(theme::SELECTION),
    };
    let label = if focused {
        format!("› {label}")
    } else {
        label.to_owned()
    };
    let style = if focused {
        style.add_modifier(Modifier::BOLD)
    } else {
        style
    };

    // Use full-cell backgrounds for every button so their visible bounds match.
    frame.render_widget(
        Paragraph::new(label)
            .alignment(Alignment::Center)
            .style(style)
            .block(Block::default().padding(Padding::vertical(1))),
        area,
    );
}

pub fn field(frame: &mut Frame, area: Rect, input: &TextArea<'_>, focused: bool) {
    let mut input = input.clone();
    input.set_style(theme::dialog().bg(theme::SELECTION));
    input.set_cursor_line_style(theme::dialog().bg(theme::SELECTION));
    input.set_cursor_style(if focused {
        theme::dialog().fg(theme::BACKGROUND).bg(theme::ACCENT)
    } else {
        theme::dialog().bg(theme::SELECTION)
    });
    input.set_block(
        Block::bordered()
            .padding(Padding::horizontal(1))
            .border_style(if focused {
                theme::dialog_title()
            } else {
                theme::muted()
            }),
    );
    frame.render_widget(&input, area);
}
