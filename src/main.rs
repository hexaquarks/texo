mod app;

fn main() -> anyhow::Result<()> {
    ratatui::run(app::run)
}
