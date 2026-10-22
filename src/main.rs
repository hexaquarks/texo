mod actions;
mod app;
mod editor;
mod explorer;
mod render_job;
mod terminal;
mod window;
mod workspace;

use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
#[command(version, about = "A terminal workspace for LaTeX resumes")]
struct Arguments {
    /// Use the current terminal instead of opening a dedicated workspace window.
    #[arg(long)]
    inline: bool,

    /// Workspace directory or TeX document. Defaults to ~/resumes.
    path: Option<PathBuf>,
}

fn main() -> anyhow::Result<()> {
    let arguments = Arguments::parse();
    let workspace = workspace::Workspace::resolve(arguments.path.as_deref())?;
    if !arguments.inline {
        let launch_path = arguments.path.as_deref().unwrap_or(&workspace.root);
        return window::open(launch_path);
    }
    let editor = editor::Editor::open(workspace.document.clone())?;

    ratatui::run(|terminal| {
        let _input_mode = terminal::InputMode::enable()?;
        app::run(terminal, editor, workspace.root)
    })
}
