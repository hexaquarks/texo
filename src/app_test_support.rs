use super::*;
use ratatui::{Terminal, backend::TestBackend};

pub(super) fn application() -> anyhow::Result<(tempfile::TempDir, App)> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("resume.tex");
    std::fs::write(&path, "Original document")?;
    let editor = Editor::open(path)?;
    Ok((directory, App::new(editor, Preview::default())?))
}

pub(super) fn wait_for_visible_pdf(
    app: &mut App,
    terminal: &mut Terminal<TestBackend>,
) -> anyhow::Result<()> {
    const PREVIEW_TIMEOUT: Duration = Duration::from_secs(30);
    const PREVIEW_POLL_INTERVAL: Duration = Duration::from_millis(10);
    let started = std::time::Instant::now();

    loop {
        app.receive_render();
        anyhow::ensure!(
            app.session.diagnostics.is_none(),
            "Build failed: {:?}",
            app.session.diagnostics
        );
        terminal.draw(|frame| app.draw(frame))?;
        let visible = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .any(|cell| cell.symbol() == "▀");
        if app.session.job.is_none() && visible {
            return Ok(());
        }

        anyhow::ensure!(
            started.elapsed() < PREVIEW_TIMEOUT,
            "Startup PDF never became visible"
        );
        std::thread::sleep(PREVIEW_POLL_INTERVAL);
    }
}

pub(super) fn wait_for_reviews(app: &mut App) -> anyhow::Result<()> {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while app.session.job.is_some() || app.session.proposed_job.is_some() {
        anyhow::ensure!(
            std::time::Instant::now() < deadline,
            "Review build did not finish"
        );
        app.receive_render();
        app.receive_proposed_render();
        std::thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}

pub(super) fn finish_test_render(app: &mut App) -> anyhow::Result<()> {
    const BUILD_TIMEOUT: Duration = Duration::from_secs(30);
    let started = std::time::Instant::now();
    while app.session.job.is_some() {
        anyhow::ensure!(started.elapsed() < BUILD_TIMEOUT, "Build did not finish");
        app.receive_render();
        std::thread::sleep(EVENT_POLL_INTERVAL);
    }
    anyhow::ensure!(
        app.session.diagnostics.is_none(),
        "Build failed: {:?}",
        app.session.diagnostics
    );
    Ok(())
}
