use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

use anyhow::{Context, Result, ensure};

use crate::{pdf::PdfPages, sandbox};

pub struct RenderedDocument {
    pub pages: PdfPages,
    pub pdf_path: PathBuf,
    pub source: String,
}

pub fn render(
    source: String,
    document_path: &Path,
    cancelled: &AtomicBool,
) -> Result<RenderedDocument> {
    let directory = document_path
        .parent()
        .context("The document needs a parent directory")?;
    let build = sandbox::build(&source, directory, cancelled)?;
    let compiled_pdf = &build.pdf_path;
    let pages = PdfPages::render(compiled_pdf, cancelled)?;
    ensure!(!cancelled.load(Ordering::Relaxed), "Render cancelled");

    // Publish only complete, successfully rasterized builds; errors preserve the previous PDF.
    let pdf_path = document_path.with_extension("pdf");
    let temporary_pdf = tempfile::NamedTempFile::new_in(directory)?;
    fs::copy(compiled_pdf, temporary_pdf.path())?;
    temporary_pdf
        .persist(&pdf_path)
        .context("Could not save the compiled PDF")?;

    Ok(RenderedDocument {
        pages,
        pdf_path,
        source,
    })
}
