use ratatui::{
    DefaultTerminal, Frame,
    crossterm::event::{self, Event, KeyCode, KeyEventKind},
    widgets::{Block, Paragraph},
};

pub fn run(terminal: &mut DefaultTerminal) -> anyhow::Result<()> {
    loop {
        terminal.draw(draw)?;

        if let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
            && matches!(key.code, KeyCode::Esc | KeyCode::Char('q'))
        {
            return Ok(());
        }
    }
}

fn draw(frame: &mut Frame) {
    let welcome =
        Paragraph::new("texo\n\nPress q or Esc to quit.").block(Block::bordered().title("texo"));

    frame.render_widget(welcome, frame.area());
}
