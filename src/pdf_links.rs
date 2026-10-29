use std::{
    fs::File,
    io::Read,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

use anyhow::{Context, Result, ensure};
use lopdf::{Dictionary, Document, LoadOptions, Object, ObjectId};

const MAX_LINK_PDF_BYTES: u64 = 32 * 1024 * 1024;
const MAX_LINK_STREAM_BYTES: usize = 8 * 1024 * 1024;
const MAX_PAGE_TREE_DEPTH: usize = 64;
const MAX_PAGE_LINKS: usize = 4096;

/// A URI annotation in normalized, top-left page coordinates, independent of text or icons.
#[derive(Clone, Debug)]
pub struct PdfLink {
    /// [left, top, right, bottom], as fractions of the rendered page.
    pub bounds: [f64; 4],
    pub uri: String,
}

pub(super) fn extract(
    pdf: &Path,
    page_count: usize,
    cancelled: &AtomicBool,
) -> Result<Vec<Vec<PdfLink>>> {
    check_cancelled(cancelled)?;
    let mut bytes = Vec::new();
    File::open(pdf)?
        .take(MAX_LINK_PDF_BYTES + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_LINK_PDF_BYTES,
        "PDF exceeds the link extraction size limit"
    );

    let options = LoadOptions::with_max_decompressed_size(MAX_LINK_STREAM_BYTES);
    let document = Document::load_mem_with_options(&bytes, options)?;
    check_cancelled(cancelled)?;

    let pages = document.get_pages();
    ensure!(pages.len() == page_count, "PDF page counts disagree");

    pages
        .values()
        .map(|page| {
            check_cancelled(cancelled)?;
            page_links(&document, *page)
        })
        .collect()
}

fn check_cancelled(cancelled: &AtomicBool) -> Result<()> {
    ensure!(
        !cancelled.load(Ordering::Relaxed),
        "Link extraction cancelled"
    );
    Ok(())
}

fn page_links(document: &Document, page: ObjectId) -> Result<Vec<PdfLink>> {
    let media = rectangle(
        inherited(document, page, b"MediaBox")?.context("Missing page dimensions")?,
        document,
    )?;
    let rotation = inherited(document, page, b"Rotate")?
        .map(Object::as_i64)
        .transpose()?
        .unwrap_or(0)
        .rem_euclid(360);
    ensure!(
        matches!(rotation, 0 | 90 | 180 | 270),
        "Unsupported page rotation"
    );

    let annotations = document.get_page_annotations(page)?;
    ensure!(annotations.len() <= MAX_PAGE_LINKS, "Too many PDF links");

    let mut links = Vec::new();
    for annotation in annotations {
        // Malformed or non-URI annotations cannot prevent the remaining links from working.
        if let Some(link) = annotation_link(document, annotation, media, rotation) {
            links.push(link);
        }
    }

    Ok(links)
}

fn annotation_link(
    document: &Document,
    annotation: &Dictionary,
    media: [f64; 4],
    rotation: i64,
) -> Option<PdfLink> {
    let subtype = annotation
        .get_deref(b"Subtype", document)
        .ok()?
        .as_name()
        .ok()?;
    if subtype != b"Link" {
        return None;
    }

    let action = annotation.get_deref(b"A", document).ok()?.as_dict().ok()?;
    if action.get_deref(b"S", document).ok()?.as_name().ok()? != b"URI" {
        return None;
    }

    let uri = action.get_deref(b"URI", document).ok()?.as_str().ok()?;
    let uri = safe_uri(std::str::from_utf8(uri).ok()?)?;

    let rect = rectangle(annotation.get_deref(b"Rect", document).ok()?, document).ok()?;
    let bounds = normalized_bounds(rect, media, rotation)?;
    Some(PdfLink { bounds, uri })
}

fn safe_uri(value: &str) -> Option<String> {
    if value.chars().any(char::is_control) || value.trim() != value {
        return None;
    }

    let url = reqwest::Url::parse(value).ok()?;
    match url.scheme() {
        "http" | "https" if url.host_str().is_some() => Some(url.into()),
        "mailto" if !url.path().is_empty() => Some(url.into()),
        _ => None,
    }
}

fn inherited<'a>(
    document: &'a Document,
    mut page: ObjectId,
    key: &[u8],
) -> Result<Option<&'a Object>> {
    for _ in 0..MAX_PAGE_TREE_DEPTH {
        let node = document.get_dictionary(page)?;
        if node.has(key) {
            return Ok(Some(node.get_deref(key, document)?));
        }

        match node.get(b"Parent") {
            Ok(parent) => page = parent.as_reference()?,
            Err(_) => return Ok(None),
        }
    }

    anyhow::bail!("PDF page tree exceeds the link extraction depth limit")
}

// PDF rectangle arrays use [left, bottom, right, top].
fn rectangle(value: &Object, document: &Document) -> Result<[f64; 4]> {
    let values = document.dereference(value)?.1.as_array()?;
    ensure!(values.len() == 4, "Invalid PDF rectangle");

    let mut coordinates = [0.0; 4];
    for (coordinate, value) in coordinates.iter_mut().zip(values) {
        *coordinate = f64::from(document.dereference(value)?.1.as_float()?);
        ensure!(coordinate.is_finite(), "Invalid PDF coordinate");
    }

    let [x1, y1, x2, y2] = coordinates;
    ensure!(x1 != x2 && y1 != y2, "Empty PDF rectangle");
    Ok([x1.min(x2), y1.min(y2), x1.max(x2), y1.max(y2)])
}

fn normalized_bounds(rect: [f64; 4], media: [f64; 4], rotation: i64) -> Option<[f64; 4]> {
    let left = rect[0].max(media[0]);
    let bottom = rect[1].max(media[1]);
    let right = rect[2].min(media[2]);
    let top = rect[3].min(media[3]);
    if left >= right || bottom >= top {
        return None;
    }

    // pdftoppm uses the full MediaBox without -cropbox, so CropBox must not clip links.
    let width = media[2] - media[0];
    let height = media[3] - media[1];
    let left = (left - media[0]) / width;
    let right = (right - media[0]) / width;
    let top = 1.0 - (top - media[1]) / height;
    let bottom = 1.0 - (bottom - media[1]) / height;
    Some(match rotation {
        0 => [left, top, right, bottom],
        90 => [1.0 - bottom, left, 1.0 - top, right],
        180 => [1.0 - right, 1.0 - bottom, 1.0 - left, 1.0 - top],
        270 => [top, 1.0 - right, bottom, 1.0 - left],
        _ => return None,
    })
}

#[cfg(test)]
#[path = "pdf_links_tests.rs"]
mod tests;
