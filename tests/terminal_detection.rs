#![cfg(unix)]

use std::process::Command;

#[test]
#[ignore = "requires Python 3 and a Unix PTY"]
fn graphics_detection_precedes_alternate_screen() -> anyhow::Result<()> {
    check_detection("supported")
}

#[test]
#[ignore = "requires Python 3 and a Unix PTY"]
fn graphics_timeout_does_not_consume_keyboard_input() -> anyhow::Result<()> {
    check_detection("timeout")
}

#[test]
#[ignore = "requires Python 3 and a Unix PTY"]
fn unsupported_terminal_still_accepts_keyboard_input() -> anyhow::Result<()> {
    check_detection("unsupported")
}

fn check_detection(scenario: &str) -> anyhow::Result<()> {
    let output = Command::new("python3")
        .arg("tests/terminal_detection.py")
        .arg(env!("CARGO_BIN_EXE_texo"))
        .arg(scenario)
        .output()?;

    anyhow::ensure!(
        output.status.success(),
        "terminal detection ({scenario}): {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    Ok(())
}
