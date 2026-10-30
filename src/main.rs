mod actions;
mod app;
mod assistant;
mod assistant_job;
mod build_diagnostics;
mod config;
mod editor;
mod history;
mod keybindings;
mod label_dialog;
mod login;
mod project_dialog;
mod projects;
mod proposal;
mod render_job;
mod resume_label;
mod resume_project;
mod settings;
mod terminal;
mod workspace;

use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
#[command(version, about = "A terminal workspace for LaTeX resumes")]
struct Arguments {
    /// Workspace directory or TeX document. Defaults to the configured folder or ~/resumes.
    path: Option<PathBuf>,
}

fn main() -> anyhow::Result<()> {
    let arguments = Arguments::parse();
    let workspace = workspace::Workspace::resolve(arguments.path.as_deref())?;
    let editor = if workspace.empty {
        editor::Editor::placeholder(workspace.document.clone())?
    } else {
        editor::Editor::open(workspace.document.clone())?
    };

    ratatui::run(|terminal| {
        let _input_mode = terminal::InputMode::enable()?;
        app::run(terminal, editor, workspace.root, workspace.empty)
    })
}
