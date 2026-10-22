mod app;
mod editor;
mod terminal;

use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
#[command(version, about = "A terminal workspace for LaTeX resumes")]
struct Arguments {
    /// TeX document to open or create.
    #[arg(default_value = "resume.tex")]
    file: PathBuf,
}

fn main() -> anyhow::Result<()> {
    let arguments = Arguments::parse();
    let editor = editor::Editor::open(arguments.file)?;

    ratatui::run(|terminal| {
        let _input_mode = terminal::InputMode::enable()?;
        app::run(terminal, editor)
    })
}
