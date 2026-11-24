use std::{
    fs, io,
    path::{Path, PathBuf},
};

use anyhow::{Result, ensure};
use sha2::{Digest, Sha256};

use crate::resume_project::Project;

const EXPORT_RECORDS: &str = ".texo-exports";
const EXPORT_SUFFIX: &str = "-export.pdf";

pub fn document_filename(document: &Path) -> String {
    let stem = document.file_stem().unwrap_or_default().to_string_lossy();
    format!("{stem}{EXPORT_SUFFIX}")
}

fn content_digest(path: &Path) -> Result<String> {
    let mut digest = Sha256::new();
    io::copy(&mut fs::File::open(path)?, &mut digest)?;
    Ok(format!("{:x}", digest.finalize()))
}

fn record_directory(pdf: &Path) -> PathBuf {
    pdf.parent()
        .unwrap()
        .join(EXPORT_RECORDS)
        .join(pdf.file_name().unwrap())
}

fn create_record_directory(directory: &Path) -> Result<()> {
    match fs::create_dir(directory) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            ensure!(
                fs::symlink_metadata(directory)?.is_dir(),
                "Invalid export records folder"
            );
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

// Record the exact contents before publishing. Independent records let concurrent
// exports finish in either order and keep replacement assets eligible for copying.
pub fn record_export(destination: &Path, contents: &Path) -> Result<()> {
    let directory = record_directory(destination);
    create_record_directory(directory.parent().unwrap())?;
    create_record_directory(&directory)?;

    let record = directory.join(content_digest(contents)?);
    match fs::File::create_new(&record) {
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            ensure!(
                fs::symlink_metadata(record)?.is_file(),
                "Invalid export record"
            );
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

pub struct ProjectOutputs {
    build: PathBuf,
    old_default: PathBuf,
}

impl ProjectOutputs {
    pub fn new(project: &Project) -> Self {
        let document = project.document();
        Self {
            build: document.with_extension("pdf"),
            old_default: project.root.join(document_filename(&document)),
        }
    }

    pub fn contains(&self, path: &Path) -> Result<bool> {
        if path == self.build {
            return Ok(true);
        }
        if path.extension().is_none_or(|extension| extension != "pdf") {
            return Ok(false);
        }

        let directory = record_directory(path);
        if directory.is_dir() && directory.join(content_digest(path)?).is_file() {
            return Ok(true);
        }

        // Older exports have no record. Only recognize the old default if the
        // published build confirms it; a similarly named asset should survive.
        if path == self.old_default
            && fs::symlink_metadata(&self.build).is_ok_and(|entry| entry.is_file())
        {
            return Ok(content_digest(path)? == content_digest(&self.build)?);
        }

        Ok(false)
    }
}
