use std::{fs, sync::atomic::AtomicBool};

use texo::compiler;

#[test]
#[ignore = "requires macOS, Tectonic, Poppler, and cached LaTeX packages"]
fn renders_unsaved_text_with_relative_includes_and_refreshes_the_pdf() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("my resume.tex");
    let saved = "saved content must remain unchanged";
    fs::write(&path, saved)?;
    fs::write(
        directory.path().join("experience.tex"),
        "Experience from a local file.",
    )?;
    let source = "\\documentclass{article}\n\\begin{document}\n\\input{experience}\n\\newpage\nSecond page.\n\\end{document}";

    let first = compiler::render(source.into(), &path, &AtomicBool::new(false))?;
    assert_eq!(first.pages.page_count(), 2);
    assert_eq!(fs::read_to_string(&path)?, saved);
    let original_pdf = fs::read(&first.pdf_path)?;

    let second_source =
        "\\documentclass{article}\n\\begin{document}\nUpdated resume.\n\\end{document}";
    let second = compiler::render(second_source.into(), &path, &AtomicBool::new(false))?;
    assert_eq!(second.pages.page_count(), 1);
    assert_ne!(fs::read(&second.pdf_path)?, original_pdf);
    assert_eq!(fs::read_to_string(&path)?, saved);
    assert!(!fs::read_dir(directory.path())?.any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".texo-draft-")
    }));
    Ok(())
}

#[test]
#[ignore = "requires macOS, Tectonic, and cached LaTeX packages"]
fn failed_compilation_keeps_the_previous_pdf() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("resume.tex");
    let pdf = path.with_extension("pdf");
    fs::write(&pdf, b"previous successful output")?;

    let source = "\\documentclass{article}\\begin{document}\\undefinedtexocommand\\end{document}";
    let error = compiler::render(source.into(), &path, &AtomicBool::new(false))
        .err()
        .expect("broken TeX must fail");
    assert!(error.to_string().contains("failed"));
    assert_eq!(fs::read(pdf)?, b"previous successful output");
    Ok(())
}

#[test]
#[ignore = "requires macOS, Tectonic cache, and Poppler"]
fn draft_preview_preserves_published_files_and_cleans_up_on_discard() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("resume.tex");
    let published_pdf = path.with_extension("pdf");
    fs::write(&path, "saved original")?;
    fs::write(&published_pdf, b"previous PDF")?;

    let source = r"\documentclass{article}\begin{document}Draft\end{document}";
    let draft = compiler::render_draft(source.into(), &path, &AtomicBool::new(false))?;
    let temporary_pdf = draft.pdf_path.clone();

    assert!(temporary_pdf.is_file());
    assert_eq!(draft.pages.page_count(), 1);
    assert_eq!(fs::read_to_string(&path)?, "saved original");
    assert_eq!(fs::read(&published_pdf)?, b"previous PDF");

    drop(draft);

    assert!(!temporary_pdf.exists());
    assert_eq!(fs::read(&published_pdf)?, b"previous PDF");
    Ok(())
}
