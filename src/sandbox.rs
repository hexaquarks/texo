//! Offline compilation of an agent draft. No saved document is modified.
use anyhow::Result;
use std::{
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};

#[cfg(target_os = "macos")]
const POLICY: &str = r#"
(version 1)
(deny default)
(import "dyld-support.sb")
(allow process-exec (literal (param "COMPILER")))
(allow sysctl-read)

(allow file-read-metadata)
(allow file-read* (subpath "/System/Library") (subpath "/usr/lib") (subpath "/usr/share")
    (subpath "/Library/Fonts") (literal "/dev/urandom") (literal "/dev/null")
    (literal (param "COMPILER")) (subpath (param "CACHE"))
    (subpath (param "RESUME")) (subpath (param "BUILD"))
    (regex #"^/opt/homebrew/.*\.dylib$") (regex #"^/usr/local/.*\.dylib$"))
(allow file-write* (subpath (param "BUILD")) (literal "/dev/null"))
(allow mach-lookup (global-name "com.apple.FontObjectsServer")
    (global-name "com.apple.fonts") (global-name "com.apple.system.logger"))
"#;

pub struct CompiledDraft {
    pub pdf_path: PathBuf,
    pub diagnostics: String,
    _directory: tempfile::TempDir,
}

pub fn compile(source: &str, directory: &Path, cancelled: &AtomicBool) -> Result<String> {
    Ok(build(source, directory, cancelled)?.diagnostics)
}

pub fn build(source: &str, directory: &Path, cancelled: &AtomicBool) -> Result<CompiledDraft> {
    #[cfg(target_os = "macos")]
    {
        compile_macos(source, directory, cancelled)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (source, directory, cancelled);
        anyhow::bail!("Isolated compilation is currently supported on macOS only")
    }
}

#[cfg(target_os = "macos")]
fn compile_macos(source: &str, directory: &Path, cancelled: &AtomicBool) -> Result<CompiledDraft> {
    use anyhow::{Context, ensure};
    use std::{io::Write, time::Duration};

    const COMPILE_TIMEOUT: Duration = Duration::from_secs(120);

    let compiler = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|directory| directory.join("tectonic"))
        .find(|path| path.is_file())
        .context("Install Tectonic to compile resumes")?
        .canonicalize()?;

    let home = directories::BaseDirs::new().context("Cannot locate the local TeX cache")?;
    let cache = home.cache_dir().join("TectonicProject.Tectonic").canonicalize()
        .context("Local LaTeX resources are missing. Prepare Tectonic's cache before using offline compilation")?;

    let directory = directory.canonicalize()?;
    let project_root = project_root(&directory);
    let build = tempfile::tempdir()?;
    let build_path = build.path().canonicalize()?;

    let input_directory = link_project_assets(&project_root, &directory, &build_path)?;

    let mut input = tempfile::Builder::new()
        .prefix(".texo-agent-")
        .suffix(".tex")
        .tempfile_in(&input_directory)?;
    input.write_all(source.as_bytes())?;

    let mut command = isolated_command(&compiler, &cache, &project_root, &build_path)?;
    command.current_dir(&directory);
    command
        .args([
            "-X",
            "compile",
            "--untrusted",
            "--only-cached",
            "--keep-logs",
            "--outdir",
        ])
        .arg(&input_directory)
        .arg(input.path())
        .env("HOME", home.home_dir());

    let diagnostics = crate::process::run(&mut command, COMPILE_TIMEOUT, cancelled)?;
    ensure!(
        input.path().with_extension("pdf").is_file(),
        "Compilation produced no PDF"
    );

    Ok(CompiledDraft {
        pdf_path: input.path().with_extension("pdf"),
        diagnostics,
        _directory: build,
    })
}

// Mirror only the directories leading to the entrypoint. Linking a whole parent directory
// would put the draft outside temporary storage; flattening it breaks ../ includes.
#[cfg(target_os = "macos")]
fn link_project_assets(root: &Path, directory: &Path, build: &Path) -> Result<PathBuf> {
    use std::fs;

    const MAX_ROOT_ENTRIES: usize = 1024;
    let relative = directory.strip_prefix(root)?;
    let mut components = relative.components().peekable();
    let mut original = root.to_path_buf();
    let mut destination = build.to_path_buf();

    loop {
        for (index, entry) in fs::read_dir(&original)?.enumerate() {
            anyhow::ensure!(
                index < MAX_ROOT_ENTRIES,
                "Resume folder contains too many entries"
            );
            let entry = entry?;
            let target = destination.join(entry.file_name());
            let is_ancestor = components
                .peek()
                .is_some_and(|part| part.as_os_str() == entry.file_name());

            if is_ancestor {
                fs::create_dir(&target)?;
            } else {
                // The sandbox checks symlink targets, so external assets remain unreadable.
                std::os::unix::fs::symlink(entry.path(), target)?;
            }
        }

        let Some(component) = components.next() else {
            return Ok(destination);
        };
        original.push(component);
        destination.push(component);
    }
}

pub const PROJECT_MANIFEST_FILE: &str = "texo.toml";
pub const MAX_PROJECT_MANIFEST_BYTES: u64 = 16 * 1024;

/// A nested entrypoint may include assets elsewhere in its own project, never in a sibling project.
#[cfg(target_os = "macos")]
fn project_root(directory: &Path) -> PathBuf {
    #[derive(serde::Deserialize)]
    struct CompileManifest {
        entrypoint: PathBuf,
    }

    for root in directory.ancestors() {
        let path = root.join(PROJECT_MANIFEST_FILE);
        let Ok(metadata) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if !metadata.is_file() || metadata.len() > MAX_PROJECT_MANIFEST_BYTES {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        let Ok(manifest) = toml_edit::de::from_str::<CompileManifest>(&text) else {
            continue;
        };

        if !manifest
            .entrypoint
            .components()
            .all(|part| matches!(part, std::path::Component::Normal(_)))
        {
            continue;
        }
        if root
            .join(manifest.entrypoint)
            .parent()
            .and_then(|path| path.canonicalize().ok())
            .as_deref()
            == Some(directory)
        {
            return root.to_path_buf();
        }
    }
    directory.to_path_buf()
}

// Use Apple's loader rules rather than reproducing platform-specific loader permissions.
// All other access is denied unless explicitly listed in POLICY.
#[cfg(target_os = "macos")]
fn isolated_command(
    compiler: &Path,
    cache: &Path,
    resume: &Path,
    build: &Path,
) -> Result<std::process::Command> {
    let mut command = std::process::Command::new("/usr/bin/sandbox-exec");

    // Parameters are separate argv entries, never interpolated into sandbox syntax.
    for (name, path) in [
        ("COMPILER", compiler),
        ("CACHE", cache),
        ("RESUME", resume),
        ("BUILD", build),
    ] {
        command
            .arg("-D")
            .arg(format!("{name}={}", path.canonicalize()?.display()));
    }

    command
        .args(["-p", POLICY])
        .arg(compiler.canonicalize()?)
        .current_dir(resume)
        .env_clear()
        .env("TMPDIR", build)
        .env("PATH", "/usr/bin:/bin");

    Ok(command)
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires Tectonic and cached LaTeX packages"]
    fn cancellation_stops_a_nonterminating_tex_document() {
        use std::{
            sync::{Arc, atomic::Ordering},
            time::{Duration, Instant},
        };

        let folder = tempfile::tempdir().unwrap();
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = Arc::clone(&cancelled);
        let started = Instant::now();

        let worker = std::thread::spawn(move || {
            compile(
                r"\documentclass{article}\begin{document}\loop\iftrue\repeat\end{document}",
                folder.path(),
                &worker_cancelled,
            )
        });

        std::thread::sleep(Duration::from_millis(200));
        cancelled.store(true, Ordering::Relaxed);

        let error = worker.join().unwrap().unwrap_err();
        assert!(error.to_string().contains("cancelled"));
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[test]
    #[ignore = "requires Tectonic and cached LaTeX packages"]
    fn latex_cannot_overwrite_a_resume_asset() {
        let folder = tempfile::tempdir().unwrap();
        let asset = folder.path().join("protected.txt");
        std::fs::write(&asset, "original").unwrap();

        let source = format!(
            concat!(
                r"\documentclass{{article}}\begin{{document}}",
                r"\newwrite\target\immediate\openout\target={} ",
                r"\immediate\write\target{{changed}}\immediate\closeout\target",
                r" Test\end{{document}}",
            ),
            asset.display()
        );
        let _ = compile(&source, folder.path(), &AtomicBool::new(false));

        assert_eq!(std::fs::read_to_string(asset).unwrap(), "original");
    }

    #[test]
    #[ignore = "requires installed Tectonic and cached LaTeX packages"]
    fn offline_compilation_builds_a_pdf() {
        let folder = tempfile::tempdir().unwrap();
        std::fs::write(folder.path().join("section.tex"), "Hello").unwrap();

        compile(
            r"\documentclass{article}\begin{document}\input{section.tex}\end{document}",
            folder.path(),
            &AtomicBool::new(false),
        )
        .unwrap();
    }

    fn command(program: &str, resume: &Path, build: &Path) -> std::process::Command {
        isolated_command(Path::new(program), build, resume, build).unwrap()
    }

    #[test]
    fn sandbox_blocks_outside_reads_symlink_escapes_and_source_writes() {
        let resume = tempfile::tempdir().unwrap();
        let build = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();

        let secret = outside.path().join("private.txt");
        std::fs::write(&secret, "synthetic-private-marker").unwrap();

        let link = resume.path().join("escape.txt");
        std::os::unix::fs::symlink(&secret, &link).unwrap();

        let local = resume.path().join("local.txt");
        std::fs::write(&local, "local asset").unwrap();

        for denied in [&secret, &link] {
            let result = command("/bin/cat", resume.path(), build.path())
                .arg(denied)
                .output()
                .unwrap();

            assert!(!result.status.success());
            assert!(!String::from_utf8_lossy(&result.stdout).contains("synthetic-private-marker"));
        }

        let result = command("/bin/cat", resume.path(), build.path())
            .arg(&local)
            .output()
            .unwrap();

        assert!(result.status.success());
        assert_eq!(result.stdout, b"local asset");

        let result = command("/bin/bash", resume.path(), build.path())
            .args(["-c", "printf changed > \"$1\"", "test"])
            .arg(&local)
            .output()
            .unwrap();

        assert!(!result.status.success());
        assert_eq!(std::fs::read_to_string(local).unwrap(), "local asset");

        let output = build.path().join("allowed.txt");

        let result = command("/bin/bash", resume.path(), build.path())
            .args(["-c", "printf allowed > \"$1\"", "test"])
            .arg(&output)
            .output()
            .unwrap();

        assert!(result.status.success(), "{:?}", result);
    }

    #[test]
    fn sandbox_blocks_network_subprocesses_and_inherited_secrets() {
        let resume = tempfile::tempdir().unwrap();
        let build = tempfile::tempdir().unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();

        let result = command("/usr/bin/curl", resume.path(), build.path())
            .args(["--silent", "--max-time", "1"])
            .arg(format!("http://{}", listener.local_addr().unwrap()))
            .output()
            .unwrap();

        assert!(!result.status.success());
        assert!(listener.accept().is_err());

        let result = command("/bin/bash", resume.path(), build.path())
            .args(["-c", "exec /bin/cat /dev/null"])
            .output()
            .unwrap();

        assert!(!result.status.success());

        let environment = command("/bin/bash", resume.path(), build.path());
        assert!(
            !environment
                .get_envs()
                .any(|(key, _)| key == "OPENAI_API_KEY" || key == "ANTHROPIC_API_KEY")
        );
    }

    #[test]
    #[ignore = "requires installed Tectonic and cached LaTeX packages"]
    fn latex_cannot_read_an_outside_file_through_a_symlink() {
        let resume = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();

        let secret = outside.path().join("private.tex");
        std::fs::write(&secret, r"\typeout{SYNTHETIC_PRIVATE_MARKER}").unwrap();
        std::os::unix::fs::symlink(&secret, resume.path().join("escape.tex")).unwrap();

        let error = compile(
            r"\documentclass{article}\begin{document}\input{escape.tex}\end{document}",
            resume.path(),
            &AtomicBool::new(false),
        )
        .unwrap_err();

        assert!(!error.to_string().contains("SYNTHETIC_PRIVATE_MARKER"));
    }
}

#[cfg(all(test, target_os = "macos"))]
mod project_compile_tests {
    use super::*;

    #[test]
    #[ignore = "requires Tectonic and cached LaTeX packages"]
    fn nested_entrypoint_can_include_project_assets_but_not_sibling_projects() -> Result<()> {
        let workspace = tempfile::tempdir()?;
        let root = workspace.path().join("resume");
        let nested = root.join("source");
        std::fs::create_dir_all(&nested)?;
        std::fs::write(
            root.join(PROJECT_MANIFEST_FILE),
            r#"entrypoint = "source/main.tex""#,
        )?;
        std::fs::write(root.join("section.tex"), "Local experience")?;
        std::fs::write(workspace.path().join("private.tex"), "Other project")?;

        let cancelled = AtomicBool::new(false);
        let good = r"\documentclass{article}\begin{document}\input{../section.tex}\end{document}";
        build(good, &nested, &cancelled)?;

        let bad = r"\documentclass{article}\begin{document}\input{../../private.tex}\end{document}";
        assert!(build(bad, &nested, &cancelled).is_err());
        Ok(())
    }
}
