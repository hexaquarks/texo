use super::*;

#[cfg(target_os = "macos")]
mod codex_workflow {
    use super::*;
    use serde_json::{Value, json};
    use texo::tools::{ToolDefinition, ToolSet};

    const SOURCE: &str = concat!(
        r"\documentclass{article}\begin{document}",
        "I built reporting tools and maintained tests.",
        r"\end{document}",
    );
    const SOURCE_URL: &str = "https://example.test/job";
    const RESEARCH_SUMMARY: &str = "The job posting highlights reporting and testing.";

    struct WorkflowTools {
        resume: ResumeTools,
        original: Option<String>,
        researched: bool,
    }

    impl ToolSet for WorkflowTools {
        fn definitions(&self) -> Vec<ToolDefinition> {
            let mut definitions = self.resume.definitions();
            definitions.extend([
                ToolDefinition {
                    name: "research_job".into(),
                    description: "Read the job posting".into(),
                    parameters: json!({"type": "object", "properties": {}}),
                },
                ToolDefinition {
                    name: "preserve_original".into(),
                    description: "Make a separate copy of the current draft".into(),
                    parameters: json!({"type": "object", "properties": {}}),
                },
            ]);

            definitions
        }

        fn execute(
            &mut self,
            name: &str,
            arguments: &Value,
            cancelled: &AtomicBool,
        ) -> Result<Value> {
            match name {
                "research_job" => {
                    self.researched = true;
                    Ok(json!({"url": SOURCE_URL, "requirements": ["reporting", "testing"]}))
                }
                "preserve_original" => {
                    assert!(self.researched);
                    assert!(self.original.is_none(), "Preserve the original only once");

                    let draft = self.resume.execute("read_resume", arguments, cancelled)?;
                    self.original = draft["tex"].as_str().map(str::to_owned);

                    Ok(json!({"copy_id": "preserved-resume"}))
                }
                _ => {
                    assert!(
                        self.original.is_some(),
                        "Preserve the original before editing"
                    );
                    self.resume.execute(name, arguments, cancelled)
                }
            }
        }

        fn finish(&mut self, reply: AssistantReply) -> Result<AssistantReply> {
            self.resume.finish(reply)
        }
    }

    fn call(id: &str, name: &str, arguments: Value) -> Value {
        json!({
            "id": id,
            "name": name,
            "arguments_json": arguments.to_string()
        })
    }

    fn turn(reply: &str, calls: Vec<Value>) -> Value {
        json!({"reply": reply, "proposed_tex": null, "tool_calls": calls})
    }

    fn tool_turn(id: &str, name: &str, arguments: Value) -> Value {
        turn("", vec![call(id, name, arguments)])
    }

    fn fake_codex(response: &Value, input_path: &Path) -> Command {
        let mut command = Command::new("sh");
        command
            .args([
                "-c",
                r#"
                response="$1"
                input_path="$2"
                shift 2
                cat > "$input_path" || exit 1

                while [ "$#" -gt 0 ]; do
                    if [ "$1" = '-o' ]; then
                        printf '%s' "$response" > "$2"
                        exit
                    fi
                    shift
                done
                exit 1
                "#,
                "fake-codex",
                &response.to_string(),
            ])
            .arg(input_path);

        command
    }

    #[test]
    #[ignore = "requires macOS and cached LaTeX packages"]
    fn codex_replays_research_and_tool_results_until_the_compiled_draft_is_ready_for_review()
    -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("resume.tex");
        fs::write(&path, SOURCE)?;
        let input_path = directory.path().join("provider-input.txt");
        let messages = [ConversationMessage {
            role: "user",
            content:
                "Research the role, preserve the original, and revise the open resume for review."
                    .into(),
        }];
        let prompt = build_prompt(SOURCE, &path, &messages)?;
        let mut tools = WorkflowTools {
            resume: ResumeTools::new("resume.tex".into(), SOURCE.into())
                .in_directory(directory.path().to_owned()),
            original: None,
            researched: false,
        };

        let broken = SOURCE.replace("I built", r"\undefinedcommand");
        let revised = SOURCE.replace("I built", "Built");
        let final_reply = format!("The revised draft is ready for review. Source: {SOURCE_URL}");
        let mut rounds = [
            (
                turn(
                    RESEARCH_SUMMARY,
                    vec![call("research", "research_job", json!({}))],
                ),
                "",
            ),
            (
                tool_turn("preserve", "preserve_original", json!({})),
                RESEARCH_SUMMARY,
            ),
            (
                turn(
                    "",
                    vec![
                        call(
                            "wrong-match",
                            "replace_text",
                            json!({"old_text": "missing", "new_text": "new"}),
                        ),
                        call("skipped-compile", "compile_resume", json!({})),
                    ],
                ),
                "preserved-resume",
            ),
            (
                tool_turn("stage", "propose_edit", json!({"tex": broken})),
                "Skipped after an earlier tool in this batch failed",
            ),
            (turn("Done", vec![]), ""),
            (
                tool_turn("failed-compile", "compile_resume", json!({})),
                "Compile the staged draft successfully",
            ),
            (
                turn(
                    "",
                    vec![
                        call("repair", "propose_edit", json!({"tex": revised})),
                        call("compile", "compile_resume", json!({})),
                    ],
                ),
                r#""id":"failed-compile","is_error":true"#,
            ),
            (
                tool_turn("review", "read_resume", json!({})),
                r#""compiled":true"#,
            ),
            (
                turn(&final_reply, vec![]),
                "Built reporting tools and maintained tests.",
            ),
        ]
        .into_iter();

        let result = agent::run(
            &prompt,
            &mut tools,
            &AtomicBool::new(false),
            |prompt, definitions, history, cancelled| {
                let (response, expected_input) =
                    rounds.next().context("Unexpected provider round")?;
                let context = llm::TurnContext {
                    prompt,
                    tools: definitions,
                    history,
                };

                let turn = request_codex_turn(
                    &mut fake_codex(&response, &input_path),
                    &context,
                    cancelled,
                )?;
                let input = fs::read_to_string(&input_path)?;
                assert!(
                    input.contains(expected_input),
                    "Expected Codex input to contain {expected_input:?}"
                );

                Ok(turn)
            },
        )?;

        let last_input = fs::read_to_string(input_path)?;
        assert!(last_input.contains(SOURCE_URL));
        assert!(last_input.contains(RESEARCH_SUMMARY));
        assert!(rounds.next().is_none());
        assert_eq!(result.reply, final_reply);
        assert_eq!(result.proposed_tex.as_deref(), Some(revised.as_str()));
        assert!(tools.resume.take_build().is_some());
        assert_eq!(tools.original.as_deref(), Some(SOURCE));
        assert_eq!(fs::read_to_string(path)?, SOURCE);

        Ok(())
    }
}

#[test]
fn pending_and_disconnected_workers_are_distinguished() {
    let (job, sender) = AssistantJob::pending();
    assert!(job.poll().is_none());

    drop(sender);
    let error = job
        .poll()
        .expect("Disconnected worker must finish")
        .reply
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
    let result = AssistantJob::start(&source, Path::new("resume.tex"), &[], None);

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
