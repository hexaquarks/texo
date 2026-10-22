use ratatui::style::{Color, Style};

pub const BACKGROUND: Color = Color::Rgb(23, 27, 28);
pub const EDITOR: Color = Color::Rgb(29, 33, 34);
pub const PREVIEW: Color = Color::Rgb(23, 27, 28);
pub const SIDEBAR: Color = BACKGROUND;
pub const ELEVATED: Color = Color::Rgb(34, 40, 40);
pub const SELECTION: Color = Color::Rgb(47, 64, 56);
pub const DIVIDER: Color = Color::Rgb(79, 104, 96);
pub const TEXT: Color = Color::Rgb(220, 225, 232);
pub const MUTED: Color = Color::Rgb(143, 153, 165);
pub const ACCENT: Color = Color::Rgb(170, 231, 190);
pub const SYNTAX_COMMAND: Color = Color::Rgb(126, 197, 246);
pub const SYNTAX_ARGUMENT: Color = Color::Rgb(174, 225, 187);
pub const BUTTON: Color = Color::Rgb(35, 46, 41);
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
