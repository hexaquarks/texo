use super::*;

#[test]
fn pending_and_disconnected_workers_are_distinguished() {
    let (job, sender) = AssistantJob::pending();
    assert!(job.poll().is_none());

    drop(sender);
    let error = job
        .poll()
        .expect("Disconnected worker must finish")
        .err()
        .unwrap();
    assert_eq!(
        error.to_string(),
        "The assistant worker stopped unexpectedly"
    );
}

#[test]
fn oversized_context_is_rejected_before_a_worker_can_start() {
    let source = "x".repeat(MAX_CONTEXT_BYTES + 1);
    let result = AssistantJob::start(&source, Path::new("resume.tex"), &[]);

    let error = result
        .err()
        .expect("Oversized context must fail synchronously");
    assert!(error.to_string().contains("conversation is too large"));
}

#[cfg(unix)]
#[test]
fn missing_or_failed_cli_never_returns_a_proposal_or_process_diagnostics() -> Result<()> {
    let folder = tempfile::tempdir()?;
    let cancelled = AtomicBool::new(false);
    let missing = request(
        &mut Command::new(folder.path().join("missing-codex")),
        "synthetic prompt",
        &cancelled,
        REQUEST_TIMEOUT,
    )
    .unwrap_err();
    assert!(missing.to_string().contains("Could not start Codex"));

    let mut failed = Command::new("sh");
    failed.args([
        "-c",
        "printf synthetic-private-marker >&2; exit 1",
        "fake-codex",
    ]);
    let error = request(&mut failed, "synthetic prompt", &cancelled, REQUEST_TIMEOUT).unwrap_err();
    assert!(error.to_string().contains("could not complete"));
    assert!(!format!("{error:#}").contains("synthetic-private-marker"));

    let mut empty = Command::new("sh");
    empty.args(["-c", "exit 0", "fake-codex"]);
    let error = request(&mut empty, "synthetic prompt", &cancelled, REQUEST_TIMEOUT).unwrap_err();
    assert!(error.to_string().contains("returned no response"));
    Ok(())
}

#[cfg(unix)]
#[test]
fn cli_response_size_is_bounded_before_json_parsing() {
    let mut command = Command::new("sh");
    command.args([
        "-c",
        r#"
        size="$1"
        shift
        while [ "$#" -gt 0 ]; do
            if [ "$1" = '-o' ]; then
                shift
                head -c "$size" /dev/zero > "$1"
                exit 0
            fi
            shift
        done
        exit 1
        "#,
        "fake-codex",
        &(MAX_RESPONSE_BYTES + 1).to_string(),
    ]);

    let error = request(
        &mut command,
        "synthetic prompt",
        &AtomicBool::new(false),
        REQUEST_TIMEOUT,
    )
    .unwrap_err();
    assert_eq!(error.to_string(), "The assistant response is too large");
}
