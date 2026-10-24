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
    // Pending reviews retain their temporary PDF until acceptance or discard.
    pub build: Option<sandbox::CompiledDraft>,
}

pub fn render(
    source: String,
    document_path: &Path,
    cancelled: &AtomicBool,
) -> Result<RenderedDocument> {
    let mut document = render_draft(source, document_path, cancelled)?;
    ensure!(!cancelled.load(Ordering::Relaxed), "Render cancelled");
    document.pdf_path = publish_pdf(&document.pdf_path, document_path)?;
    document.build = None;

    Ok(document)
}

pub fn render_draft(
    source: String,
    document_path: &Path,
    cancelled: &AtomicBool,
) -> Result<RenderedDocument> {
    let directory = document_path
        .parent()
        .context("The document needs a parent directory")?;
    let build = sandbox::build(&source, directory, cancelled)?;
    let pages = PdfPages::render(&build.pdf_path, cancelled)?;
    ensure!(!cancelled.load(Ordering::Relaxed), "Render cancelled");

    Ok(RenderedDocument {
        pages,
        pdf_path: build.pdf_path.clone(),
        source,
        build: Some(build),
    })
}

pub fn publish_pdf(compiled_pdf: &Path, document_path: &Path) -> Result<PathBuf> {
    let directory = document_path
        .parent()
        .context("The document needs a parent directory")?;
    let pdf_path = document_path.with_extension("pdf");
    let temporary_pdf = tempfile::NamedTempFile::new_in(directory)?;
    fs::copy(compiled_pdf, temporary_pdf.path())?;
    temporary_pdf
        .persist(&pdf_path)
        .context("Could not save the compiled PDF")?;

    Ok(pdf_path)
}
