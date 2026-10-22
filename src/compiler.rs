use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use anyhow::{Context, Result, ensure};

use crate::{pdf::PdfPages, process};

const COMPILATION_TIMEOUT: Duration = Duration::from_secs(120);

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
    // A sibling snapshot preserves relative includes without overwriting the editor's saved file.
    let mut snapshot = tempfile::Builder::new()
        .prefix(".texo-draft-")
        .suffix(".tex")
        .tempfile_in(directory)?;
    snapshot.write_all(source.as_bytes())?;
    let build = tempfile::tempdir()?;

    process::run(
        Command::new("tectonic")
            .current_dir(directory)
            .args(["-X", "compile", "--untrusted", "--keep-logs", "--outdir"])
            .arg(build.path())
            .arg(snapshot.path()),
        COMPILATION_TIMEOUT,
        cancelled,
    )?;

    let filename = snapshot
        .path()
        .file_name()
        .context("The draft has no filename")?;
    let compiled_pdf = build.path().join(filename).with_extension("pdf");
    let pages = PdfPages::render(&compiled_pdf, cancelled)?;
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
