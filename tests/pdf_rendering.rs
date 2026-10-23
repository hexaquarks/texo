use std::{path::Path, sync::atomic::AtomicBool};

use texo::pdf::PdfPages;

#[test]
#[ignore = "requires Poppler (pdfinfo and pdftoppm) on PATH"]
fn renders_all_pdf_pages_in_order() -> anyhow::Result<()> {
    let document = PdfPages::render(
        Path::new("tests/fixtures/two-pages.pdf"),
        &AtomicBool::new(false),
    )?;

    assert_eq!(document.page_count(), 2);
    let first = document.load_page(0)?;
    let second = document.load_page(1)?;
    assert!(first.width() > 0 && first.height() > 0);
    assert_eq!(first.width(), second.width());
    assert_ne!(first.as_bytes(), second.as_bytes());
    assert!(document.load_page(2).is_err());
    Ok(())
}

#[test]
#[ignore = "requires Poppler (pdfinfo) on PATH"]
fn reports_invalid_pdf_errors() -> anyhow::Result<()> {
    let file = tempfile::NamedTempFile::new()?;
    std::fs::write(file.path(), "not a PDF")?;

    let result = PdfPages::render(file.path(), &AtomicBool::new(false));
    let error = result.err().expect("invalid PDF must fail");
    assert!(error.to_string().contains("pdfinfo failed"));
    Ok(())
}

#[test]
#[ignore = "requires Poppler (pdfinfo and pdftoppm) on PATH"]
fn previews_real_pdf_pages_without_terminal_graphics() -> anyhow::Result<()> {
    use ratatui::{Terminal, backend::TestBackend, style::Color};
    use texo::preview::Preview;

    let pages = PdfPages::render(
        Path::new("tests/fixtures/two-pages.pdf"),
        &AtomicBool::new(false),
    )?;
    let mut preview = Preview::default();
    assert!(preview.uses_basic_rendering());
    preview.set_pages(pages)?;

    let mut terminal = Terminal::new(TestBackend::new(100, 50))?;
    terminal.draw(|frame| preview.draw(frame, frame.area()))?;
    let first = terminal.backend().buffer().clone();
    assert!(
        first
            .content
            .iter()
            .any(|cell| cell.bg == Color::Rgb(255, 255, 255))
    );
    assert!(first.content.iter().any(|cell| cell.symbol() == "▀"));
    assert_eq!(preview.page_status().as_deref(), Some("1 / 2"));

    preview.next_page()?;
    terminal.draw(|frame| preview.draw(frame, frame.area()))?;
    assert_ne!(&first, terminal.backend().buffer());
    assert_eq!(preview.page_status().as_deref(), Some("2 / 2"));

    preview.previous_page()?;
    terminal.draw(|frame| preview.draw(frame, frame.area()))?;
    assert_eq!(&first, terminal.backend().buffer());
    preview.clear();
    terminal.draw(|frame| preview.draw(frame, frame.area()))?;
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("Your PDF will appear here."));
    Ok(())
}
