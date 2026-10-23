use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail, ensure};

const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(50);
const MAX_DIAGNOSTIC_BYTES: u64 = 32 * 1024;
const MAX_PROCESS_OUTPUT_BYTES: u64 = 1024 * 1024;

pub(crate) fn run(
    command: &mut Command,
    timeout: Duration,
    cancelled: &AtomicBool,
) -> Result<String> {
    ensure!(!cancelled.load(Ordering::Relaxed), "Render cancelled");
    let program = command.get_program().to_string_lossy().into_owned();
    let mut output = tempfile::tempfile()?;

    // Rendering documents must not expose provider credentials to external tools.
    command
        .env_remove("OPENAI_API_KEY")
        .env_remove("ANTHROPIC_API_KEY");

    // File-backed output avoids blocking the child on a full pipe during verbose TeX errors.
    command.stdin(Stdio::null());
    command.stdout(output.try_clone()?);
    command.stderr(output.try_clone()?);
    let child = command.spawn().with_context(|| {
        format!("Could not start {program}. Install the required tool and ensure it is on PATH")
    })?;
    let mut running = RunningProcess(child);
    let started = Instant::now();

    loop {
        if cancelled.load(Ordering::Relaxed) {
            bail!("Render cancelled");
        }

        let status = running.0.try_wait()?;
        ensure!(
            output.metadata()?.len() <= MAX_PROCESS_OUTPUT_BYTES,
            "{program} exceeded its diagnostic output limit"
        );

        if let Some(status) = status {
            let diagnostics = read_diagnostics(&mut output)?;
            ensure!(
                status.success(),
                "{program} failed ({status}):\n{diagnostics}"
            );

            return Ok(diagnostics);
        }

        if started.elapsed() >= timeout {
            bail!(
                "{program} exceeded its {} second time limit",
                timeout.as_secs()
            );
        }

        thread::sleep(PROCESS_POLL_INTERVAL);
    }
}

fn read_diagnostics(output: &mut File) -> Result<String> {
    let start = output
        .metadata()?
        .len()
        .saturating_sub(MAX_DIAGNOSTIC_BYTES);
    output.seek(SeekFrom::Start(start))?;
    let mut bytes = Vec::new();
    output.read_to_end(&mut bytes)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

struct RunningProcess(Child);

impl Drop for RunningProcess {
    fn drop(&mut self) {
        // Reap the child on every path, including timeout, cancellation, and I/O failure.
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn renderer_subprocesses_do_not_receive_provider_credentials() {
        let mut command = Command::new("sh");
        command.env("OPENAI_API_KEY", "synthetic-openai-secret");
        command.env("ANTHROPIC_API_KEY", "synthetic-anthropic-secret");
        command.args([
            "-c",
            "test -z \"${OPENAI_API_KEY+x}\" && test -z \"${ANTHROPIC_API_KEY+x}\"",
        ]);
        run(
            &mut command,
            Duration::from_secs(1),
            &AtomicBool::new(false),
        )
        .unwrap();
    }

    #[test]
    fn failed_commands_report_their_diagnostics() {
        let error = run(
            Command::new("sh").args(["-c", "printf 'broken document' >&2; exit 1"]),
            Duration::from_secs(1),
            &AtomicBool::new(false),
        )
        .unwrap_err();
        assert!(error.to_string().contains("broken document"));
    }

    #[test]
    fn timeout_terminates_a_running_command() {
        let started = Instant::now();
        let error = run(
            Command::new("sleep").arg("10"),
            Duration::from_millis(100),
            &AtomicBool::new(false),
        )
        .unwrap_err();
        assert!(error.to_string().contains("time limit"));
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn excessive_subprocess_output_is_rejected() {
        let error = run(
            Command::new("head").args([
                "-c",
                &(MAX_PROCESS_OUTPUT_BYTES + 1).to_string(),
                "/dev/zero",
            ]),
            Duration::from_secs(2),
            &AtomicBool::new(false),
        )
        .unwrap_err();

        assert!(error.to_string().contains("diagnostic output limit"));
    }

    #[test]
    fn cancellation_prevents_starting_a_command() {
        let error = run(
            &mut Command::new("nonexistent-texo-test-command"),
            Duration::from_secs(1),
            &AtomicBool::new(true),
        )
        .unwrap_err();
        assert_eq!(error.to_string(), "Render cancelled");
    }
}
