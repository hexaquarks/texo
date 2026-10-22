use ratatui::style::{Color, Style};

pub const BACKGROUND: Color = Color::Rgb(17, 20, 29);
pub const EDITOR: Color = Color::Rgb(24, 28, 39);
pub const PREVIEW: Color = Color::Rgb(21, 25, 35);
pub const ELEVATED: Color = Color::Rgb(32, 38, 52);
pub const TEXT: Color = Color::Rgb(222, 228, 242);
pub const MUTED: Color = Color::Rgb(136, 149, 175);
pub const ACCENT: Color = Color::Rgb(155, 175, 255);
pub const BUTTON: Color = Color::Rgb(176, 191, 255);
pub const ERROR: Color = Color::Rgb(255, 160, 167);
pub const PANEL_PADDING: u16 = 2;
pub const PANEL_HEADER_HEIGHT: u16 = 3;

pub fn text() -> Style {
    Style::default().fg(TEXT).bg(BACKGROUND)
}

pub fn muted() -> Style {
    Style::default().fg(MUTED)
}
