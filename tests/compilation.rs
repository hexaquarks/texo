use std::{fs, sync::atomic::AtomicBool};

use texo::compiler;

#[test]
#[ignore = "requires Tectonic and Poppler on PATH; first run may download TeX packages"]
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
#[ignore = "requires Tectonic on PATH"]
fn failed_compilation_keeps_the_previous_pdf() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("resume.tex");
    let pdf = path.with_extension("pdf");
    fs::write(&pdf, b"previous successful output")?;

    let source = "\\documentclass{article}\\begin{document}\\undefinedtexocommand\\end{document}";
    let error = compiler::render(source.into(), &path, &AtomicBool::new(false))
        .err()
        .expect("broken TeX must fail");
    assert!(error.to_string().contains("tectonic failed"));
    assert_eq!(fs::read(pdf)?, b"previous successful output");
    Ok(())
}
