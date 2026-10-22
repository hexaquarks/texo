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
