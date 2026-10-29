use super::*;
use lopdf::{Stream, dictionary};

fn linked_document() -> Document {
    let mut document = Document::with_version("1.7");
    let pages = document.new_object_id();
    let action = document.add_object(dictionary! {
        "S" => "URI",
        "URI" => Object::string_literal("https://example.com/icon"),
    });
    let annotation = document.add_object(dictionary! {
        "Type" => "Annot",
        "Subtype" => "Link",
        "Rect" => vec![30.into(), 50.into(), 50.into(), 70.into()],
        "A" => action,
    });
    // A black vector square has no text for a text-based link extractor to match.
    let contents = document.add_object(Stream::new(
        dictionary! {},
        b"0 0 0 rg 30 50 20 20 re f".to_vec(),
    ));
    let page = document.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages,
        "Contents" => contents,
        "Resources" => dictionary! {},
        "Annots" => vec![annotation.into()],
    });
    document.objects.insert(
        pages,
        dictionary! {
            "Type" => "Pages",
            "Count" => 1,
            "Kids" => vec![page.into()],
            "MediaBox" => vec![10.into(), 20.into(), 110.into(), 220.into()],
            "CropBox" => vec![20.into(), 30.into(), 100.into(), 210.into()],
            "Rotate" => 90,
        }
        .into(),
    );
    let catalog = document.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages,
    });
    document.trailer.set("Root", catalog);
    document
}

#[test]
fn extracts_vector_annotation_with_inherited_page_geometry() {
    let document = linked_document();
    let page = *document.get_pages().values().next().unwrap();
    let links = page_links(&document, page).unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].uri, "https://example.com/icon");
    assert_bounds(links[0].bounds, [0.15, 0.2, 0.25, 0.4]);
}

fn assert_bounds(actual: [f64; 4], expected: [f64; 4]) {
    for (actual, expected) in actual.into_iter().zip(expected) {
        assert!((actual - expected).abs() < 1e-9, "{actual} != {expected}");
    }
}

#[test]
fn transforms_rotations_and_clips_media_box() {
    let media = [10.0, 20.0, 110.0, 220.0];
    let rect = [0.0, 0.0, 50.0, 70.0];
    for (rotation, expected) in [
        (0, [0.0, 0.75, 0.4, 1.0]),
        (90, [0.0, 0.0, 0.25, 0.4]),
        (180, [0.6, 0.0, 1.0, 0.25]),
        (270, [0.75, 0.6, 1.0, 1.0]),
    ] {
        assert_bounds(normalized_bounds(rect, media, rotation).unwrap(), expected);
    }
    assert!(normalized_bounds([0.0, 0.0, 5.0, 5.0], media, 0).is_none());
}

#[test]
fn only_web_and_email_uris_can_be_opened() {
    for uri in [
        "https://example.com",
        "http://example.com",
        "mailto:person@example.com",
    ] {
        assert!(safe_uri(uri).is_some(), "{uri}");
    }

    for uri in [
        "file:///tmp/resume",
        "javascript:alert(1)",
        "data:text/html,test",
        "tel:123",
        "../local.pdf",
        "--help",
        " https://example.com",
        "https://example.com\n",
        "https://exa\tmple.com",
        "mailto:",
    ] {
        assert!(safe_uri(uri).is_none(), "{uri:?}");
    }
}

#[test]
fn cancellation_and_unsupported_geometry_disable_extraction() {
    assert!(extract(Path::new("does-not-exist"), 1, &AtomicBool::new(true)).is_err());
    let mut document = linked_document();
    let page = *document.get_pages().values().next().unwrap();
    document.get_dictionary_mut(page).unwrap().set("Rotate", 45);
    assert!(page_links(&document, page).is_err());
}

#[test]
#[ignore = "requires Poppler"]
fn rendered_vector_icon_matches_its_link_rectangle() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let pdf = directory.path().join("vector-links.pdf");
    linked_document().save(&pdf)?;
    let pages = crate::pdf::PdfPages::render(&pdf, &AtomicBool::new(false))?;
    assert!(!pages.links_unavailable());
    let image = pages.load_page(0)?.to_rgb8();
    let link = &pages.links(0)[0];
    let center_x = ((link.bounds[0] + link.bounds[2]) / 2.0 * f64::from(image.width())) as u32;
    let center_y = ((link.bounds[1] + link.bounds[3]) / 2.0 * f64::from(image.height())) as u32;
    assert_eq!(image.get_pixel(center_x, center_y).0, [0, 0, 0]);
    assert_eq!(
        image.get_pixel(center_x + image.width() / 5, center_y).0,
        [255, 255, 255]
    );
    Ok(())
}

#[test]
#[ignore = "requires Poppler"]
fn link_extraction_failure_preserves_the_rendered_pdf() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let pdf = directory.path().join("malformed-links.pdf");
    let mut document = linked_document();
    let page = *document.get_pages().values().next().unwrap();
    let malformed_annotations = document.add_object(dictionary! { "Unexpected" => true });
    document
        .get_dictionary_mut(page)?
        .set("Annots", malformed_annotations);
    document.save(&pdf)?;

    let pages = crate::pdf::PdfPages::render(&pdf, &AtomicBool::new(false))?;
    assert!(pages.links_unavailable());
    assert!(pages.links(0).is_empty());
    assert!(pages.load_page(0)?.width() > 0);
    Ok(())
}

#[test]
#[ignore = "requires Poppler"]
fn media_box_rendering_ignores_crop_box_at_each_rotation() -> Result<()> {
    let directory = tempfile::tempdir()?;
    for rotation in [0, 90, 180, 270] {
        let pdf = directory.path().join(format!("crop-{rotation}.pdf"));
        let mut document = linked_document();
        let page = *document.get_pages().values().next().unwrap();
        let contents = document.add_object(Stream::new(
            dictionary! {},
            b"0 0 0 rg 10 50 40 20 re f".to_vec(),
        ));
        let page_dictionary = document.get_dictionary_mut(page)?;
        page_dictionary.set("Contents", contents);
        page_dictionary.set("Rotate", rotation);
        let annotation = dictionary! {
            "Subtype" => "Link",
            "Rect" => vec![14.into(), 59.into(), 16.into(), 61.into()],
            "A" => dictionary! {
                "S" => "URI",
                "URI" => Object::string_literal("https://example.com/outside-crop"),
            },
        };
        page_dictionary.set("Annots", vec![Object::Dictionary(annotation)]);
        document.save(&pdf)?;

        let pages = crate::pdf::PdfPages::render(&pdf, &AtomicBool::new(false))?;
        let image = pages.load_page(0)?.to_rgb8();
        assert_eq!(pages.links(0).len(), 1);
        let outside_crop = pages.links(0)[0].bounds;
        let x = ((outside_crop[0] + outside_crop[2]) / 2.0 * f64::from(image.width())) as u32;
        let y = ((outside_crop[1] + outside_crop[3]) / 2.0 * f64::from(image.height())) as u32;
        assert_eq!(image.get_pixel(x, y).0, [0, 0, 0], "rotation {rotation}");
    }
    Ok(())
}
