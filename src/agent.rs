use std::{
    collections::HashSet,
    sync::atomic::{AtomicBool, Ordering},
};

use anyhow::{Result, anyhow, bail, ensure};
use serde_json::{Value, json};

use crate::{
    llm::AssistantReply,
    tools::{self, ToolCall, ToolDefinition, ToolExchange, ToolResult, ToolSet},
};

pub const MAX_AGENT_TURNS: usize = 10;
pub const MAX_TOOL_CALLS: usize = 20;
pub const MAX_AGENT_CONTEXT_BYTES: usize = 1024 * 1024;
const MAX_CALL_ID_BYTES: usize = 128;
const ASK_USER: &str = "ask_user";
const MAX_QUESTION_CHARS: usize = 1000;

#[derive(Debug)]
pub enum AgentTurn {
    Complete(AssistantReply),
    Tools {
        calls: Vec<ToolCall>,
        continuation: Value,
    },
}

/// Run one task against a fixed provider and document snapshot. Intermediate tool turns
/// never reach the editor; only a completed, validated result can be applied by the caller.
pub fn run(
    prompt: &str,
    tools: &mut impl ToolSet,
    cancelled: &AtomicBool,
    mut next_turn: impl FnMut(
        &str,
        &[ToolDefinition],
        &[ToolExchange],
        &AtomicBool,
    ) -> Result<AgentTurn>,
) -> Result<AssistantReply> {
    let mut definitions = tools.definitions();
    definitions.push(clarification_tool());

    let mut history = Vec::new();
    let mut call_ids = HashSet::new();
    let mut completion_error: Option<anyhow::Error> = None;

    for round in 1..=MAX_AGENT_TURNS {
        ensure!(!cancelled.load(Ordering::Relaxed), "Assistant cancelled");

        let recovery_prompt = completion_error.as_ref().map(|error| {
            format!(
                "{prompt}\n\nTexo rejected the previous final reply: {error}. \
                 Continue from the existing tools and results. Complete any missing steps, \
                 or call ask_user if you need information from the user."
            )
        });
        let turn_prompt = recovery_prompt.as_deref().unwrap_or(prompt);
        let context_bytes = turn_prompt.len()
            + serde_json::to_vec(&history)?.len()
            + serde_json::to_vec(&definitions)?.len();
        ensure!(
            context_bytes <= MAX_AGENT_CONTEXT_BYTES,
            "Agent context limit reached. No edit was applied"
        );

        let turn = next_turn(turn_prompt, &definitions, &history, cancelled)?;
        ensure!(!cancelled.load(Ordering::Relaxed), "Assistant cancelled");

        match turn {
            AgentTurn::Complete(reply) => {
                let completion = finish_turn(tools, reply, history.last());
                ensure!(!cancelled.load(Ordering::Relaxed), "Assistant cancelled");

                match completion {
                    Ok(reply) => return Ok(reply),
                    Err(error) => completion_error = Some(error),
                }
            }
            AgentTurn::Tools {
                calls,
                continuation,
            } => {
                validate_batch(&calls, &mut call_ids)?;

                let clarification = calls
                    .iter()
                    .find(|call| call.name == ASK_USER)
                    .map(|call| clarify(call, calls.len()));
                if let Some(Ok(reply)) = clarification {
                    return Ok(reply);
                }

                // Leave room to report the result before starting another operation.
                ensure!(
                    round < MAX_AGENT_TURNS,
                    "Agent turn limit reached. No edit was applied"
                );

                let results = match clarification {
                    Some(Err(error)) => calls
                        .iter()
                        .map(|call| ToolResult {
                            id: call.id.clone(),
                            is_error: true,
                            output: Value::String(error.to_string()),
                        })
                        .collect(),
                    _ => tools::execute_calls(tools, &definitions, &calls, cancelled)?,
                };

                completion_error = None;
                history.push(ToolExchange {
                    calls,
                    continuation,
                    results,
                });
            }
        }
    }

    if let Some(error) = completion_error {
        bail!("Agent turn limit reached. No edit was applied. {error}");
    }

    bail!("Agent turn limit reached. No edit was applied")
}

fn finish_turn(
    tools: &mut impl ToolSet,
    reply: AssistantReply,
    latest_exchange: Option<&ToolExchange>,
) -> Result<AssistantReply> {
    let reply = tools.finish(reply)?;
    let latest_batch_failed = latest_exchange
        .is_some_and(|exchange| exchange.results.iter().any(|result| result.is_error));
    ensure!(
        reply.proposed_tex.is_none() || !latest_batch_failed,
        "Resolve the latest tool failure before returning a staged edit"
    );

    Ok(reply)
}

fn validate_batch(calls: &[ToolCall], call_ids: &mut HashSet<String>) -> Result<()> {
    ensure!(!calls.is_empty(), "The model returned an empty tool turn");
    ensure!(
        call_ids.len() + calls.len() <= MAX_TOOL_CALLS,
        "Agent tool limit reached. No edit was applied"
    );

    // Validate the whole batch before executing any operation.
    for call in calls {
        ensure!(
            !call.id.trim().is_empty() && call.id.len() <= MAX_CALL_ID_BYTES,
            "Invalid tool call ID"
        );
        ensure!(
            !call.name.trim().is_empty() && call.arguments.is_object(),
            "Invalid tool name or arguments"
        );
        ensure!(call_ids.insert(call.id.clone()), "Repeated tool call ID");
    }

    Ok(())
}

fn clarification_tool() -> ToolDefinition {
    ToolDefinition {
        name: ASK_USER.into(),
        description: "Ask the user for information needed to proceed. Call this tool alone. \
            Include any completed steps the user needs to know about. This ends the task and \
            discards any staged edit. The user can answer in chat."
            .into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "question": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": MAX_QUESTION_CHARS
                }
            },
            "required": ["question"],
            "additionalProperties": false
        }),
    }
}

fn clarify(call: &ToolCall, batch_size: usize) -> Result<AssistantReply> {
    ensure!(batch_size == 1, "Call ask_user alone");
    ensure!(
        call.arguments
            .as_object()
            .is_some_and(|args| args.len() == 1),
        "ask_user accepts only question"
    );

    let question = call.arguments["question"]
        .as_str()
        .map(str::trim)
        .filter(|question| !question.is_empty() && question.chars().count() <= MAX_QUESTION_CHARS)
        .ok_or_else(|| anyhow!("ask_user requires a brief, non-empty question"))?;

    Ok(AssistantReply {
        reply: question.into(),
        proposed_tex: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resume_tools::ResumeTools;
    use serde_json::json;

    struct TestTools {
        executions: usize,
    }

    impl ToolSet for TestTools {
        fn definitions(&self) -> Vec<ToolDefinition> {
            vec![ToolDefinition {
                name: "check".into(),
                description: "Check a draft".into(),
                parameters: json!({"type":"object"}),
            }]
        }

        fn execute(&mut self, _: &str, arguments: &Value, _: &AtomicBool) -> Result<Value> {
            self.executions += 1;
            ensure!(arguments["valid"] == true, "Invalid draft");

            Ok(json!({"ok":true}))
        }
    }

    fn calls(id: &str, name: &str, valid: bool) -> AgentTurn {
        AgentTurn::Tools {
            calls: vec![ToolCall {
                id: id.into(),
                name: name.into(),
                arguments: json!({"valid":valid}),
            }],
            continuation: json!([]),
        }
    }

    fn complete() -> AgentTurn {
        AgentTurn::Complete(AssistantReply {
            reply: "Done".into(),
            proposed_tex: None,
        })
    }

    #[test]
    fn premature_completion_keeps_the_draft_available_and_can_end_in_clarification() {
        let mut tools = ResumeTools::new("resume.tex".into(), "original".into());
        let mut round = 0;
        let staged = "partially revised";

        let reply = run(
            "Revise the resume",
            &mut tools,
            &AtomicBool::new(false),
            |prompt, definitions, history, _| {
                round += 1;
                assert!(definitions.iter().any(|tool| tool.name == ASK_USER));

                let (name, arguments) = match round {
                    1 => ("propose_edit", json!({"tex": staged})),
                    2 => return Ok(complete()),
                    3 => {
                        assert!(prompt.contains("Compile the staged draft successfully"));
                        assert_eq!(history.len(), 1);
                        ("read_resume", json!({}))
                    }
                    4 => {
                        assert_eq!(history[1].results[0].output["tex"], staged);
                        (ASK_USER, json!({"question": "Which dates should I use?"}))
                    }
                    _ => unreachable!("Unexpected provider round"),
                };

                Ok(AgentTurn::Tools {
                    calls: vec![ToolCall {
                        id: round.to_string(),
                        name: name.into(),
                        arguments,
                    }],
                    continuation: Value::Null,
                })
            },
        )
        .unwrap();

        assert_eq!(reply.reply, "Which dates should I use?");
        assert!(reply.proposed_tex.is_none());
    }

    #[test]
    fn clarification_batched_with_an_edit_runs_neither_call() {
        let mut tools = TestTools { executions: 0 };

        let reply = run(
            "task",
            &mut tools,
            &AtomicBool::new(false),
            |_, _, history, _| {
                if let Some(exchange) = history.last() {
                    assert_eq!(exchange.results.len(), 2);
                    assert!(exchange.results.iter().all(|result| result.is_error));
                    assert_eq!(exchange.results[1].id, "question");
                    return Ok(complete());
                }

                Ok(AgentTurn::Tools {
                    calls: vec![
                        ToolCall {
                            id: "edit".into(),
                            name: "check".into(),
                            arguments: json!({"valid": true}),
                        },
                        ToolCall {
                            id: "question".into(),
                            name: ASK_USER.into(),
                            arguments: json!({"question": "Which dates should I use?"}),
                        },
                    ],
                    continuation: Value::Null,
                })
            },
        )
        .unwrap();

        assert!(reply.proposed_tex.is_none());
        assert_eq!(tools.executions, 0);
    }

    #[test]
    fn tool_errors_return_to_model_and_can_be_repaired_before_completion() {
        let mut tools = TestTools { executions: 0 };

        let result = run(
            "task",
            &mut tools,
            &AtomicBool::new(false),
            |_, _, history, _| {
                Ok(match history.len() {
                    0 => calls("first", "check", false),
                    1 => {
                        assert!(history[0].results[0].is_error);
                        calls("second", "check", true)
                    }
                    _ => {
                        assert!(!history[1].results[0].is_error);
                        complete()
                    }
                })
            },
        )
        .unwrap();

        assert_eq!(result.reply, "Done");
        assert_eq!(tools.executions, 2);
    }

    #[test]
    fn latest_tool_failure_prevents_returning_an_earlier_proposal() {
        let mut tools = TestTools { executions: 0 };
        let mut round = 0;

        let reply = run(
            "task",
            &mut tools,
            &AtomicBool::new(false),
            |prompt, _, history, _| {
                round += 1;
                match round {
                    1 => Ok(calls("failed", "check", false)),
                    2 | 4 => Ok(AgentTurn::Complete(AssistantReply {
                        reply: "Ready for review".into(),
                        proposed_tex: Some("validated draft".into()),
                    })),
                    3 => {
                        assert!(prompt.contains("Resolve the latest tool failure"));
                        assert!(history[0].results[0].is_error);
                        Ok(calls("repaired", "check", true))
                    }
                    _ => unreachable!("Unexpected provider round"),
                }
            },
        )
        .unwrap();

        assert_eq!(round, 4);
        assert_eq!(reply.proposed_tex.as_deref(), Some("validated draft"));
    }

    #[test]
    fn unknown_tools_are_not_executed_and_duplicate_ids_fail_closed() {
        let mut tools = TestTools { executions: 0 };

        let error = run(
            "task",
            &mut tools,
            &AtomicBool::new(false),
            |_, _, history, _| {
                if !history.is_empty() {
                    assert!(history[0].results[0].is_error);
                }
                Ok(calls("same", "shell", true))
            },
        )
        .unwrap_err();

        assert!(error.to_string().contains("Repeated"));
        assert_eq!(tools.executions, 0);
    }

    #[test]
    fn excessive_or_duplicate_batches_do_not_execute_any_tools() {
        for count in [2, MAX_TOOL_CALLS + 1] {
            let mut tools = TestTools { executions: 0 };

            let result = run("task", &mut tools, &AtomicBool::new(false), |_, _, _, _| {
                Ok(AgentTurn::Tools {
                    calls: (0..count)
                        .map(|_| ToolCall {
                            id: "same".into(),
                            name: "check".into(),
                            arguments: json!({"valid":true}),
                        })
                        .collect(),
                    continuation: json!([]),
                })
            });

            assert!(result.is_err());
            assert_eq!(tools.executions, 0);
        }
    }

    #[test]
    fn malformed_batch_is_rejected_before_any_tool_runs() {
        for (id, name, arguments) in [
            (" ", "check", json!({})),
            ("bad", " ", json!({})),
            ("bad", "check", json!(null)),
            ("bad", "check", json!([])),
        ] {
            let mut tools = TestTools { executions: 0 };

            let result = run("task", &mut tools, &AtomicBool::new(false), |_, _, _, _| {
                Ok(AgentTurn::Tools {
                    calls: vec![
                        ToolCall {
                            id: "valid".into(),
                            name: "check".into(),
                            arguments: json!({"valid": true}),
                        },
                        ToolCall {
                            id: id.into(),
                            name: name.into(),
                            arguments: arguments.clone(),
                        },
                    ],
                    continuation: Value::Null,
                })
            });

            assert!(result.is_err());
            assert_eq!(tools.executions, 0);
        }
    }

    #[test]
    fn cancellation_after_a_model_response_prevents_tool_execution() {
        let cancelled = AtomicBool::new(false);
        let mut tools = TestTools { executions: 0 };

        let result = run("task", &mut tools, &cancelled, |_, _, _, cancelled| {
            cancelled.store(true, Ordering::Relaxed);
            Ok(calls("one", "check", true))
        });

        assert!(result.is_err());
        assert_eq!(tools.executions, 0);
    }

    #[test]
    fn cancellation_limits_and_provider_errors_never_return_partial_edits() {
        let mut tools = TestTools { executions: 0 };
        assert!(
            run(
                "task",
                &mut tools,
                &AtomicBool::new(true),
                |_, _, _, _| panic!("cancelled")
            )
            .is_err()
        );

        assert!(
            run(
                &"x".repeat(MAX_AGENT_CONTEXT_BYTES + 1),
                &mut tools,
                &AtomicBool::new(false),
                |_, _, _, _| panic!("too large")
            )
            .is_err()
        );

        let error = run(
            "task",
            &mut tools,
            &AtomicBool::new(false),
            |_, _, history, _| Ok(calls(&history.len().to_string(), "check", true)),
        )
        .unwrap_err();

        assert!(error.to_string().contains("turn limit"));
        assert_eq!(tools.executions, MAX_AGENT_TURNS - 1);

        assert!(
            run(
                "task",
                &mut tools,
                &AtomicBool::new(false),
                |_, _, _, _| bail!("Provider unavailable")
            )
            .is_err()
        );
    }
}
