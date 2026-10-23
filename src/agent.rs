use std::{
    collections::HashSet,
    sync::atomic::{AtomicBool, Ordering},
};

use anyhow::{Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::llm::AssistantReply;

pub const MAX_AGENT_TURNS: usize = 10;
pub const MAX_TOOL_CALLS: usize = 20;
pub const MAX_AGENT_CONTEXT_BYTES: usize = 1024 * 1024;
pub const MAX_TOOL_RESULT_BYTES: usize = 256 * 1024;
const MAX_CALL_ID_BYTES: usize = 128;

#[derive(Clone, Debug, Serialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

#[derive(Debug)]
pub enum AgentTurn {
    Complete(AssistantReply),
    Tools {
        calls: Vec<ToolCall>,
        continuation: Value,
    },
}

#[derive(Debug, Serialize)]
pub struct ToolResult {
    pub id: String,
    pub is_error: bool,
    pub output: Value,
}

#[derive(Debug, Serialize)]
pub struct ToolExchange {
    pub calls: Vec<ToolCall>,
    pub continuation: Value,
    pub results: Vec<ToolResult>,
}

/// Implementations expose only explicitly supported operations, never arbitrary shell commands.
pub trait ToolSet {
    fn definitions(&self) -> Vec<ToolDefinition>;

    fn execute(&mut self, name: &str, arguments: &Value, cancelled: &AtomicBool) -> Result<Value>;

    fn finish(&mut self, reply: AssistantReply) -> Result<AssistantReply> {
        Ok(reply)
    }
}

pub struct NoTools;

impl ToolSet for NoTools {
    fn definitions(&self) -> Vec<ToolDefinition> {
        Vec::new()
    }

    fn execute(&mut self, _: &str, _: &Value, _: &AtomicBool) -> Result<Value> {
        bail!("No tools are registered")
    }
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
    let definitions = tools.definitions();
    let mut history = Vec::new();
    let mut call_ids = HashSet::new();

    for _ in 0..MAX_AGENT_TURNS {
        ensure!(!cancelled.load(Ordering::Relaxed), "Assistant cancelled");

        let context_bytes = prompt.len()
            + serde_json::to_vec(&history)?.len()
            + serde_json::to_vec(&definitions)?.len();
        ensure!(
            context_bytes <= MAX_AGENT_CONTEXT_BYTES,
            "Agent context limit reached. No edit was applied"
        );

        let turn = next_turn(prompt, &definitions, &history, cancelled)?;
        ensure!(!cancelled.load(Ordering::Relaxed), "Assistant cancelled");

        match turn {
            AgentTurn::Complete(reply) => return tools.finish(reply),
            AgentTurn::Tools {
                calls,
                continuation,
            } => {
                ensure!(!calls.is_empty(), "The model returned an empty tool turn");
                ensure!(
                    call_ids.len() + calls.len() <= MAX_TOOL_CALLS,
                    "Agent tool limit reached. No edit was applied"
                );

                // Validate the whole batch before executing any operation.
                for call in &calls {
                    ensure!(
                        !call.id.is_empty() && call.id.len() <= MAX_CALL_ID_BYTES,
                        "Invalid tool call ID"
                    );
                    ensure!(call_ids.insert(call.id.clone()), "Repeated tool call ID");
                }

                let mut results = Vec::new();
                for call in &calls {
                    ensure!(!cancelled.load(Ordering::Relaxed), "Assistant cancelled");

                    let result = if definitions.iter().any(|tool| tool.name == call.name) {
                        tools.execute(&call.name, &call.arguments, cancelled)
                    } else {
                        Err(anyhow::anyhow!("Unknown tool. Choose a registered tool"))
                    };
                    ensure!(!cancelled.load(Ordering::Relaxed), "Assistant cancelled");

                    let (is_error, output) = match result {
                        Ok(value) => (false, value),
                        Err(error) => (true, Value::String(error.to_string())),
                    };
                    ensure!(
                        serde_json::to_vec(&output)?.len() <= MAX_TOOL_RESULT_BYTES,
                        "Tool output limit reached. No edit was applied"
                    );

                    results.push(ToolResult {
                        id: call.id.clone(),
                        is_error,
                        output,
                    });
                }

                history.push(ToolExchange {
                    calls,
                    continuation,
                    results,
                });
            }
        }
    }

    bail!("Agent turn limit reached. No edit was applied")
}

#[cfg(test)]
mod tests {
    use super::*;
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
