use anyhow::{Result, anyhow, bail, ensure};
use serde_json::{Value, json};

use super::{AssistantReply, TurnContext, parse_reply, reply_schema};
use crate::{
    agent::AgentTurn,
    auth::{Profile, Provider},
    tools::ToolCall,
};

const MAX_OUTPUT_TOKENS: u32 = 16_384;

pub(super) fn payload(profile: &Profile, context: &TurnContext<'_>) -> Value {
    let prompt = context.prompt;
    let mut body = match profile.provider {
        Provider::OpenAi => json!({
            "model": profile.model,
            "input": prompt,
            "store": false,
            "reasoning": {"effort": "low"},
            "max_output_tokens": MAX_OUTPUT_TOKENS,
            "text": {
                "format": {
                    "type": "json_schema",
                    "name": "resume_edit",
                    "strict": true,
                    "schema": reply_schema(),
                },
            },
        }),
        Provider::Anthropic => json!({
            "model": profile.model,
            "max_tokens": MAX_OUTPUT_TOKENS,
            "messages": [{"role": "user", "content": prompt}],
            "output_config": {
                "format": {"type": "json_schema", "schema": reply_schema()},
            },
        }),
        _ => Value::Null,
    };

    match profile.provider {
        Provider::OpenAi => {
            let mut input = vec![json!({"role":"user", "content":prompt})];

            for exchange in context.history {
                if let Some(items) = exchange.continuation.as_array() {
                    input.extend(items.clone());
                }

                for result in &exchange.results {
                    input.push(json!({
                        "type":"function_call_output",
                        "call_id":result.id,
                        "output":json!(result).to_string(),
                    }));
                }
            }

            body["input"] = json!(input);

            // Reasoning items must be replayable without server-side response storage.
            body["include"] = json!(["reasoning.encrypted_content"]);

            if !context.tools.is_empty() {
                body["tools"] = json!(
                    context
                        .tools
                        .iter()
                        .map(|tool| json!({
                            "type": "function",
                            "name": tool.name,
                            "description": tool.description,
                            "parameters": tool.parameters,
                            "strict": true,
                        }))
                        .collect::<Vec<_>>()
                );
            }
        }
        Provider::Anthropic => {
            let mut messages = vec![json!({"role":"user", "content":prompt})];

            for exchange in context.history {
                messages.push(json!({"role":"assistant", "content":exchange.continuation}));

                let results: Vec<_> = exchange
                    .results
                    .iter()
                    .map(|result| {
                        json!({
                            "type": "tool_result",
                            "tool_use_id": result.id,
                            "content": result.output.to_string(),
                            "is_error": result.is_error,
                        })
                    })
                    .collect();

                messages.push(json!({"role":"user", "content":results}));
            }

            body["messages"] = json!(messages);

            if !context.tools.is_empty() {
                let tools: Vec<_> = context
                    .tools
                    .iter()
                    .map(|tool| {
                        json!({
                            "name": tool.name,
                            "description": tool.description,
                            "input_schema": tool.parameters,
                        })
                    })
                    .collect();

                body["tools"] = json!(tools);
            }
        }
        _ => {}
    }

    body
}

pub(super) fn parse_turn(provider: Provider, response: &Value) -> Result<AgentTurn> {
    let items = match provider {
        Provider::OpenAi => {
            ensure!(
                response["status"] == "completed",
                "The response was incomplete. No edit was applied"
            );
            response["output"].as_array()
        }
        Provider::Anthropic => {
            ensure!(
                matches!(
                    response["stop_reason"].as_str(),
                    Some("tool_use" | "end_turn")
                ),
                "The response was incomplete. No edit was applied"
            );
            response["content"].as_array()
        }
        _ => bail!("Unsupported provider"),
    }
    .ok_or_else(|| anyhow!("Invalid provider response"))?;

    let mut calls = Vec::new();
    for item in items {
        let call = match item["type"].as_str() {
            Some("function_call") if provider == Provider::OpenAi => Some(ToolCall {
                id: item["call_id"].as_str().unwrap_or_default().into(),
                name: item["name"].as_str().unwrap_or_default().into(),
                arguments: serde_json::from_str(item["arguments"].as_str().unwrap_or_default())
                    .map_err(|_| anyhow!("Invalid tool arguments"))?,
            }),
            Some("tool_use") if provider == Provider::Anthropic => Some(ToolCall {
                id: item["id"].as_str().unwrap_or_default().into(),
                name: item["name"].as_str().unwrap_or_default().into(),
                arguments: item["input"].clone(),
            }),
            _ => None,
        };

        if let Some(call) = call {
            calls.push(call);
        }
    }

    if provider == Provider::Anthropic {
        let expects_tools = response["stop_reason"] == "tool_use";
        let has_tool_calls = !calls.is_empty();

        ensure!(
            expects_tools == has_tool_calls,
            "The provider returned an inconsistent tool response"
        );
    }

    for call in &calls {
        ensure!(
            !call.id.trim().is_empty()
                && !call.name.trim().is_empty()
                && call.arguments.is_object(),
            "The provider returned an invalid tool call"
        );
    }

    if calls.is_empty() {
        Ok(AgentTurn::Complete(parse_response(provider, response)?))
    } else {
        Ok(AgentTurn::Tools {
            calls,
            continuation: json!(items),
        })
    }
}

fn parse_response(provider: Provider, response: &Value) -> Result<AssistantReply> {
    let blocks: Vec<&Value> = match provider {
        Provider::OpenAi => {
            ensure!(
                response["status"] == "completed",
                "The response was incomplete. No resume edit was applied"
            );
            response["output"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|item| item["type"] == "message")
                .flat_map(|item| item["content"].as_array().into_iter().flatten())
                .collect()
        }
        Provider::Anthropic => {
            ensure!(
                response["stop_reason"] == "end_turn",
                "The response was incomplete or refused. No resume edit was applied"
            );
            response["content"]
                .as_array()
                .into_iter()
                .flatten()
                .collect()
        }
        _ => bail!("Unsupported response provider"),
    };
    let mut text = String::new();
    for block in blocks {
        ensure!(
            block["type"] != "refusal",
            "The provider declined this request. No resume edit was applied"
        );
        if matches!(block["type"].as_str(), Some("text" | "output_text")) {
            text.push_str(block["text"].as_str().unwrap_or_default());
        }
    }
    parse_reply(text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_native_tool_responses_are_rejected() {
        for (provider, response) in [
            (
                Provider::OpenAi,
                json!({
                    "status": "completed",
                    "output": [{"type": "function_call", "call_id": "one", "arguments": "{}"}]
                }),
            ),
            (
                Provider::OpenAi,
                json!({
                    "status": "completed",
                    "output": [{
                        "type": "function_call",
                        "call_id": "one",
                        "name": "read_resume",
                        "arguments": "null"
                    }]
                }),
            ),
            (
                Provider::Anthropic,
                json!({
                    "stop_reason": "end_turn",
                    "content": [{
                        "type": "tool_use",
                        "id": "one",
                        "name": "read_resume",
                        "input": {}
                    }]
                }),
            ),
            (
                Provider::Anthropic,
                json!({
                    "stop_reason": "tool_use",
                    "content": [{"type": "text", "text": "{\"reply\":\"Done\",\"proposed_tex\":null}"}]
                }),
            ),
        ] {
            assert!(parse_turn(provider, &response).is_err());
        }
    }

    #[test]
    fn refused_truncated_and_invalid_edits_are_never_applied() {
        for response in [
            json!({"status":"incomplete"}),
            json!({"status":"completed", "output":[{"type":"message", "content":[{"type":"refusal"}]}]}),
            json!({"status":"completed", "output":[]}),
        ] {
            assert!(parse_response(Provider::OpenAi, &response).is_err());
        }
        assert!(parse_response(Provider::Anthropic, &json!({"stop_reason":"max_tokens"})).is_err());
        for text in [
            "not json",
            r#"{"reply":"ok","proposed_tex":""}"#,
            r#"{"reply":"","proposed_tex":null}"#,
        ] {
            assert!(parse_reply(text.as_bytes()).is_err());
        }
    }
}
