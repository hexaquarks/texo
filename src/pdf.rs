use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::AtomicBool,
    time::Duration,
};

use anyhow::{Context, Result, ensure};
use image::DynamicImage;
use tempfile::TempDir;

use crate::process;

const RASTER_TIMEOUT: Duration = Duration::from_secs(30);
const PREVIEW_LONG_EDGE_PIXELS: u32 = 1600;
pub const MAX_PREVIEW_PAGES: usize = 20;

pub struct PdfPages {
    // Retain the temporary files for as long as the preview can navigate between pages.
    _directory: TempDir,
    pages: Vec<PathBuf>,
}

impl PdfPages {
    pub fn render(pdf: &Path, cancelled: &AtomicBool) -> Result<Self> {
        let pdf = pdf.canonicalize().context("Could not open the PDF")?;
        let directory = tempfile::tempdir()?;
        let page_count = inspect_page_count(&pdf, cancelled)?;

        process::run(
            Command::new("pdftoppm")
                .args(["-png", "-scale-to"])
                .arg(PREVIEW_LONG_EDGE_PIXELS.to_string())
                .arg(&pdf)
                .arg(directory.path().join("page")),
            RASTER_TIMEOUT,
            cancelled,
        )?;

        let mut pages = fs::read_dir(directory.path())?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<std::io::Result<Vec<_>>>()?;
        pages.retain(|path| path.extension().is_some_and(|extension| extension == "png"));
        // Poppler pads page numbers consistently, so lexical sorting preserves page order.
        pages.sort();
        ensure!(
            pages.len() == page_count,
            "The PDF renderer did not produce every page"
        );

        Ok(Self {
            _directory: directory,
            pages,
        })
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    pub fn load_page(&self, index: usize) -> Result<DynamicImage> {
        let path = self.pages.get(index).context("PDF page is out of range")?;
        image::open(path).context("Could not load the PDF preview image")
    }
}

fn inspect_page_count(pdf: &Path, cancelled: &AtomicBool) -> Result<usize> {
    let information = process::run(
        Command::new("pdfinfo").env("LC_ALL", "C").arg(pdf),
        RASTER_TIMEOUT,
        cancelled,
    )?;
    parse_page_count(&information)
}

fn parse_page_count(information: &str) -> Result<usize> {
    let count = information
        .lines()
        .find_map(|line| line.strip_prefix("Pages:"))
        .context("The PDF has no page count")?
        .trim()
        .parse::<usize>()?;
    ensure!(count > 0, "The PDF has no pages");
    ensure!(
        count <= MAX_PREVIEW_PAGES,
        "Preview supports at most {MAX_PREVIEW_PAGES} pages"
    );
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_counts_are_validated_before_rasterizing() {
        assert_eq!(parse_page_count("Title: Resume\nPages:  2\n").unwrap(), 2);
        assert!(parse_page_count("Pages: 0").is_err());
        assert!(parse_page_count(&format!("Pages: {}", MAX_PREVIEW_PAGES + 1)).is_err());
        assert!(parse_page_count("Not a PDF").is_err());
    }
}
