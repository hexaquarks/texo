use ratatui::style::{Color, Style};

pub const BACKGROUND: Color = Color::Rgb(12, 12, 12);
pub const EDITOR: Color = Color::Rgb(16, 16, 16);
pub const PREVIEW: Color = Color::Rgb(12, 12, 12);
pub const SIDEBAR: Color = BACKGROUND;
pub const ELEVATED: Color = Color::Rgb(24, 24, 24);
pub const SELECTION: Color = Color::Rgb(44, 44, 44);
pub const DIVIDER: Color = Color::Rgb(52, 52, 52);
pub const TEXT: Color = Color::Rgb(220, 220, 220);
pub const MUTED: Color = Color::Rgb(136, 136, 136);
pub const ACCENT: Color = Color::Rgb(240, 240, 240);
pub const SYNTAX_COMMAND: Color = Color::Rgb(230, 230, 230);
pub const SYNTAX_ARGUMENT: Color = Color::Rgb(172, 172, 172);
pub const BUTTON: Color = Color::Rgb(32, 32, 32);
pub const FILE_ICON: Color = ACCENT;
pub const ERROR: Color = Color::Rgb(238, 138, 138);
pub const PANEL_PADDING: u16 = 2;
pub const PANEL_HEADER_HEIGHT: u16 = 2;

pub fn text() -> Style {
    Style::default().fg(TEXT).bg(BACKGROUND)
}

pub fn muted() -> Style {
    Style::default().fg(MUTED)
}
