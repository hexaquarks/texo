use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::llm::AssistantReply;

pub const MAX_TOOL_RESULT_BYTES: usize = 256 * 1024;

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

    /// Validate a final reply. On rejection, leave the draft usable and return feedback safe to send to the provider.
    fn finish(&mut self, reply: AssistantReply) -> Result<AssistantReply> {
        Ok(reply)
    }
}

/// Dispatch tools in order. After a failure, report the remaining calls as skipped so the model
/// can repair the failed step before dependent operations run.
pub fn execute_calls(
    tools: &mut impl ToolSet,
    definitions: &[ToolDefinition],
    calls: &[ToolCall],
    cancelled: &AtomicBool,
) -> Result<Vec<ToolResult>> {
    let mut results = Vec::new();
    let mut failed = false;

    for call in calls {
        ensure!(!cancelled.load(Ordering::Relaxed), "Assistant cancelled");

        let result = if failed {
            Err(anyhow::anyhow!(
                "Skipped after an earlier tool in this batch failed. Retry this step if still needed"
            ))
        } else if definitions.iter().any(|tool| tool.name == call.name) {
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

        failed = is_error;
        results.push(ToolResult {
            id: call.id.clone(),
            is_error,
            output,
        });
    }

    Ok(results)
}
