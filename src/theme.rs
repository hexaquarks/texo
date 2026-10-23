use ratatui::style::{Color, Style};

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
pub const LAVENDER: Color = Color::Rgb(203, 166, 247);
pub const TEAL: Color = Color::Rgb(148, 226, 213);
pub const ERROR: Color = Color::Rgb(243, 139, 168);
pub const PANEL_PADDING: u16 = 2;

pub fn text() -> Style {
    Style::default().fg(TEXT).bg(BACKGROUND)
}

pub fn muted() -> Style {
    Style::default().fg(MUTED)
}
