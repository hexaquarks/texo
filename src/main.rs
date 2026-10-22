mod app;
mod editor;
mod render_job;
mod terminal;
mod window;

use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
#[command(version, about = "A terminal workspace for LaTeX resumes")]
struct Arguments {
    /// Use the current terminal instead of opening a dedicated workspace window.
    #[arg(long)]
    inline: bool,

    /// TeX document to open or create.
    #[arg(default_value = "resume.tex")]
    file: PathBuf,
}

fn main() -> anyhow::Result<()> {
    let arguments = Arguments::parse();
    if !arguments.inline {
        return window::open(&arguments.file);
    }
    let editor = editor::Editor::open(arguments.file)?;

    ratatui::run(|terminal| {
        let _input_mode = terminal::InputMode::enable()?;
        app::run(terminal, editor)
    })
}
