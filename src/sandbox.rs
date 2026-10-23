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
    use std::{fs, io::Write, time::Duration};
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
    let build = tempfile::tempdir()?;
    let build_path = build.path().canonicalize()?;
    // Link local assets into the build root so relative TeX includes resolve normally.
    // The OS policy follows symlink targets: links outside the resume remain unreadable.
    const MAX_ROOT_ENTRIES: usize = 1024;
    for (index, entry) in fs::read_dir(&directory)?.enumerate() {
        ensure!(
            index < MAX_ROOT_ENTRIES,
            "Resume folder contains too many entries"
        );
        let entry = entry?;
        std::os::unix::fs::symlink(entry.path(), build_path.join(entry.file_name()))?;
    }
    let mut input = tempfile::Builder::new()
        .prefix(".texo-agent-")
        .suffix(".tex")
        .tempfile_in(&build_path)?;
    input.write_all(source.as_bytes())?;

    let mut command = isolated_command(&compiler, &cache, &directory, &build_path)?;
    command
        .args([
            "-X",
            "compile",
            "--untrusted",
            "--only-cached",
            "--keep-logs",
            "--outdir",
        ])
        .arg(&build_path)
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
