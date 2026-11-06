use super::*;

use std::{
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
};

use ratatui::{
    Terminal, TerminalOptions, Viewport,
    backend::CrosstermBackend,
    crossterm::{
        queue,
        terminal::{
            BeginSynchronizedUpdate, EndSynchronizedUpdate, disable_raw_mode, enable_raw_mode,
        },
    },
    layout::Rect,
};

const DRIVER_DIRECTORY: &str = "TEXO_TEST_TMUX_PREVIEW";
const FRAME_COUNT: usize = 12;
const PREVIEW_TIMEOUT: Duration = Duration::from_secs(5);
const PREVIEW_POLL_INTERVAL: Duration = Duration::from_millis(5);

#[test]
#[ignore = "requires tmux and Kitty"]
fn tmux_redraw_preserves_pdf_uploads_and_placements() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let kitty = if cfg!(target_os = "macos") {
        "/Applications/kitty.app/Contents/MacOS/kitty"
    } else {
        "kitty"
    };
    let output = Command::new(kitty)
        .args([
            "+runpy",
            concat!(
                "exec(compile(open('tests/tmux_preview.py').read(), ",
                "'tests/tmux_preview.py', 'exec', optimize=0), {'__name__': '__main__'})",
            ),
        ])
        .arg(std::env::current_exe()?)
        .arg(directory.path())
        .arg(FRAME_COUNT.to_string())
        .output()?;

    anyhow::ensure!(
        output.status.success(),
        "tmux preview regression: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    Ok(())
}

// Run in an isolated tmux pane so detection configures the real production path.
#[test]
fn tmux_preview_driver() -> anyhow::Result<()> {
    let Some(directory) = std::env::var_os(DRIVER_DIRECTORY) else {
        return Ok(());
    };

    enable_raw_mode()?;
    let result = draw_replaced_pages(Path::new(&directory));
    disable_raw_mode()?;
    result
}

fn draw_replaced_pages(directory: &Path) -> anyhow::Result<()> {
    ratatui::crossterm::style::force_color_output(true);
    let mut preview = Preview::detect();
    anyhow::ensure!(!preview.uses_basic_rendering(), "Kitty detection failed");
    let area = Rect::new(0, 0, 40, 20);
    let mut wire = tempfile::tempfile()?;
    let mut terminal = Terminal::with_options(
        CrosstermBackend::new(wire.try_clone()?),
        TerminalOptions {
            viewport: Viewport::Fixed(area),
        },
    )?;

    for frame_index in 0..FRAME_COUNT {
        // Opening another resume must not reset the terminal's passthrough policy.
        if frame_index % 3 == 0 {
            preview = preview.empty_like();
        }

        let color = if frame_index % 2 == 0 {
            [255; 3]
        } else {
            [240; 3]
        };
        let source = image::RgbImage::from_pixel(40, 60, image::Rgb(color));
        preview.set_source(Some(DynamicImage::ImageRgb8(source)));

        let deadline = Instant::now() + PREVIEW_TIMEOUT;
        let start = loop {
            let frame_start = wire.metadata()?.len();
            terminal.draw(|frame| preview.draw(frame, frame.area()))?;
            if !preview.is_preparing() && preview.has_presented_image() {
                break frame_start;
            }

            anyhow::ensure!(Instant::now() < deadline, "Preview preparation timed out");
            thread::sleep(PREVIEW_POLL_INTERVAL);
        };
        preview.flush_retired_images(terminal.backend_mut())?;

        // Resetting and setting the default foreground defers a tmux redraw every time.
        // Keep it with the small upload in one write so both reach the same input batch.
        let mut output = b"\x1b]110\x07\x1b]10;#ffffff\x07".to_vec();

        // Match the app's synchronized frames so tmux does not discard partial redraws.
        queue!(output, BeginSynchronizedUpdate)?;
        wire.seek(SeekFrom::Start(start))?;
        wire.read_to_end(&mut output)?;
        queue!(output, EndSynchronizedUpdate)?;

        std::io::stdout().write_all(&output)?;
        std::io::stdout().flush()?;
        std::fs::write(directory.join(format!("ready-{frame_index}")), [])?;

        // The parent checks actual terminal image layers before allowing another page.
        std::io::stdin().read_exact(&mut [0])?;
    }
    Ok(())
}
