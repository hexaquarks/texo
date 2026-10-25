use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
};

pub const BACKGROUND: Color = Color::Rgb(0, 0, 0);
pub const EDITOR: Color = BACKGROUND;
pub const PREVIEW: Color = Color::Rgb(0, 0, 0);
pub const SIDEBAR: Color = BACKGROUND;
pub const ELEVATED: Color = Color::Rgb(24, 24, 37);
pub const SELECTION: Color = Color::Rgb(30, 30, 46);
pub const DIVIDER: Color = Color::Rgb(49, 50, 68);
pub const TEXT: Color = Color::Rgb(205, 214, 244);
pub const MUTED: Color = Color::Rgb(127, 132, 156);
pub const ACCENT: Color = Color::Rgb(137, 180, 250);
pub const SYNTAX_COMMAND: Color = ACCENT;
pub const SYNTAX_ARGUMENT: Color = LAVENDER;
pub const BUTTON: Color = Color::Rgb(32, 32, 32);
pub const KEYCAP: Color = Color::Rgb(40, 42, 48);
pub const LAVENDER: Color = Color::Rgb(203, 166, 247);
pub const GREEN: Color = Color::Rgb(166, 227, 161);
pub const TEAL: Color = Color::Rgb(148, 226, 213);
pub const ERROR: Color = Color::Rgb(243, 139, 168);
pub const PANEL_PADDING: u16 = 2;

pub fn text() -> Style {
    Style::default().fg(TEXT).bg(BACKGROUND)
}

pub fn muted() -> Style {
    Style::default().fg(MUTED)
}

pub fn pane_title(focused: bool) -> Style {
    Style::default().fg(if focused { ACCENT } else { MUTED })
}

pub fn keycap(key: &str) -> Span<'static> {
    Span::styled(format!(" {key} "), Style::default().fg(TEXT).bg(KEYCAP))
}

pub fn shortcuts(bindings: &[(&str, &str)]) -> Line<'static> {
    let mut spans = Vec::new();

    for (index, (key, description)) in bindings.iter().enumerate() {
        if index > 0 {
            spans.push(Span::raw("  "));
        }

        spans.push(keycap(key));
        // Descriptions inherit the panel background; only the key gets a shaded box.
        spans.push(Span::styled(
            format!(" {description}"),
            Style::default().fg(MUTED),
        ));
    }

    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{
        buffer::Buffer,
        layout::Rect,
        widgets::{Paragraph, Widget},
    };

    #[test]
    fn shortcut_keys_are_shaded_and_descriptions_keep_the_panel_background() {
        let hints = shortcuts(&[("Ctrl+a u", "Maximize"), ("Esc", "cancel")]);
        let mut buffer = Buffer::empty(Rect::new(0, 0, 60, 1));

        Paragraph::new(hints)
            .style(Style::default().bg(ELEVATED))
            .render(buffer.area, &mut buffer);

        assert_eq!(buffer[(1, 0)].symbol(), "C");
        assert_eq!(buffer[(1, 0)].bg, KEYCAP);
        assert_eq!(buffer[(11, 0)].symbol(), "M");
        assert_eq!(buffer[(11, 0)].bg, ELEVATED);
        assert_eq!(buffer[(11, 0)].fg, MUTED);
    }
}
