use std::{path::Path, process::Command};

use anyhow::{Context, Result};
use display_info::DisplayInfo;
use texo::theme;

const WINDOW_SIZE_PERCENT: u32 = 90;
const FONT_SIZE_POINTS: u16 = 13;
const FONT_FAMILY: &str = "Menlo";
const WINDOW_PADDING_POINTS: u16 = 0;

pub fn open(document: &Path) -> Result<()> {
    let display = DisplayInfo::all()?
        .into_iter()
        .find(|display| display.is_primary)
        .context("Could not find the primary display")?;
    let mut command = workspace_command(document, &display)?;
    let status = command.status().context(
        "Could not open the workspace window. Install Kitty, or use texo --inline in your terminal",
    )?;
    anyhow::ensure!(
        status.success(),
        "The workspace window exited with {status}"
    );
    Ok(())
}

fn workspace_command(document: &Path, display: &DisplayInfo) -> Result<Command> {
    // On macOS these are logical display coordinates, matching Kitty's window coordinates.
    let width = display.width * WINDOW_SIZE_PERCENT / 100;
    let height = display.height * WINDOW_SIZE_PERCENT / 100;
    let x = display.x + i32::try_from((display.width - width) / 2)?;
    let y = display.y + i32::try_from((display.height - height) / 2)?;

    let executable = std::env::current_exe()?;
    let directory = std::env::current_dir()?;
    let mut command = Command::new("kitty");
    command
        // This dedicated window uses the requested theme rather than the shell's plain-output mode.
        .env_remove("NO_COLOR")
        // The new terminal is outside the launching shell's multiplexer session.
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .env_remove("STY")
        .env_remove("TERM_PROGRAM")
        .env_remove("TERM_PROGRAM_VERSION")
        .args(["--config", "NONE", "--title", "texo", "--position"])
        .arg(format!("{x}x{y}"))
        .arg("--directory")
        .arg(directory)
        .args(["--override", "remember_window_size=no"])
        .args(["--override", "remember_window_position=no"])
        .args(["--override", &format!("initial_window_width={width}")])
        .args(["--override", &format!("initial_window_height={height}")])
        .args(["--override", &format!("font_size={FONT_SIZE_POINTS}")])
        .args(["--override", &format!("font_family={FONT_FAMILY}")])
        .args([
            "--override",
            &format!("macos_titlebar_color={}", theme::BACKGROUND),
        ])
        .args([
            "--override",
            &format!("window_padding_width={WINDOW_PADDING_POINTS}"),
        ])
        .args(["--override", &format!("background={}", theme::BACKGROUND)])
        .args(["--override", &format!("foreground={}", theme::TEXT)])
        .arg(executable)
        .arg("--inline")
        .arg(document);

    Ok(command)
}
