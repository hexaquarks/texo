use ratatui::style::{Color, Style};

pub const BACKGROUND: Color = Color::Rgb(20, 20, 20);
pub const EDITOR: Color = BACKGROUND;
pub const PREVIEW: Color = Color::Rgb(26, 26, 26);
pub const SIDEBAR: Color = BACKGROUND;
pub const ELEVATED: Color = Color::Rgb(37, 37, 37);
pub const SELECTION: Color = Color::Rgb(47, 47, 47);
pub const DIVIDER: Color = Color::Rgb(43, 43, 43);
pub const TEXT: Color = Color::Rgb(204, 204, 204);
pub const MUTED: Color = Color::Rgb(132, 132, 132);
pub const ACCENT: Color = TEXT;
pub const BUTTON: Color = Color::Rgb(50, 50, 50);
pub const FILE_ICON: Color = Color::Rgb(196, 180, 98);
pub const ERROR: Color = Color::Rgb(238, 138, 138);
pub const PANEL_PADDING: u16 = 2;
pub const PANEL_HEADER_HEIGHT: u16 = 2;

pub fn text() -> Style {
    Style::default().fg(TEXT).bg(BACKGROUND)
}

pub fn muted() -> Style {
    Style::default().fg(MUTED)
}
