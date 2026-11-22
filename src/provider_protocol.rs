use anyhow::{Result, anyhow, bail, ensure};
use serde_json::{Value, json};

use super::{
    AssistantReply, TurnContext, WEB_RESEARCH_INSTRUCTIONS, parse_reply, reply_schema,
    supported_models,
};
use crate::{
    agent::AgentTurn,
    auth::{Profile, Provider},
    tools::ToolCall,
};

const MAX_OUTPUT_TOKENS: u32 = 16_384;
const MAX_OPENAI_WEB_CALLS_PER_RESPONSE: u32 = 10;
const MAX_ANTHROPIC_PAUSES: usize = 3;
const MAX_ANTHROPIC_WEB_USES: u32 = 5;
const MAX_ANTHROPIC_FETCH_TOKENS: u32 = 6_000;

pub(super) struct ProviderRequest {
    pub(super) body: Value,
    provider: Provider,
    paused_blocks: Vec<Value>,
    pauses: usize,
}

impl ProviderRequest {
    pub(super) fn new(profile: &Profile, context: &TurnContext<'_>) -> Self {
        let body = match profile.provider {
            Provider::OpenAi => openai_payload(profile, context),
            Provider::Anthropic => anthropic_payload(profile, context),
            _ => Value::Null,
        };

        Self {
            body,
            provider: profile.provider,
            paused_blocks: Vec::new(),
            pauses: 0,
        }
    }

    pub(super) fn receive(&mut self, mut response: Value) -> Result<Option<AgentTurn>> {
        if self.provider == Provider::Anthropic && response["stop_reason"] == "pause_turn" {
            self.continue_anthropic_turn(response)?;
            return Ok(None);
        }

        if self.provider == Provider::Anthropic && !self.paused_blocks.is_empty() {
            // Claude returns only new blocks after a pause, even when it then asks for a local tool.
            let new_blocks = take_anthropic_content(&mut response)?;
            self.paused_blocks.extend(new_blocks);
            response["content"] = Value::Array(std::mem::take(&mut self.paused_blocks));
        }

        let mut turn = parse_turn(self.provider, &response)?;
        if let AgentTurn::Complete(reply) = &mut turn {
            let blocks = response_blocks(self.provider, &response);
            let earlier_messages = self.body["messages"]
                .as_array()
                .map_or(&[][..], Vec::as_slice);
            append_public_sources(&mut reply.reply, self.provider, &blocks, earlier_messages);
        }

        Ok(Some(turn))
    }

    fn continue_anthropic_turn(&mut self, mut response: Value) -> Result<()> {
        ensure!(
            self.pauses < MAX_ANTHROPIC_PAUSES,
            "Claude paused research too many times. Retry with a narrower question"
        );
        let blocks = take_anthropic_content(&mut response)?;
        ensure!(
            !blocks.is_empty() && blocks.iter().all(|block| block["type"] != "tool_use"),
            "Claude returned an invalid paused response"
        );

        self.paused_blocks.extend(blocks);
        let messages = self.body["messages"]
            .as_array_mut()
            .expect("Claude request messages are an array");
        let assistant = json!({"role": "assistant", "content": self.paused_blocks});
        if self.pauses == 0 {
            messages.push(assistant);
        } else {
            let last_index = messages.len() - 1;
            messages[last_index] = assistant;
        }
        self.pauses += 1;

        Ok(())
    }
}

fn take_anthropic_content(response: &mut Value) -> Result<Vec<Value>> {
    match response.get_mut("content").map(Value::take) {
        Some(Value::Array(blocks)) => Ok(blocks),
        _ => bail!("Claude returned invalid response content"),
    }
}

fn openai_payload(profile: &Profile, context: &TurnContext<'_>) -> Value {
    let mut input = vec![json!({"role": "user", "content": context.prompt})];

    for exchange in context.history {
        if let Some(items) = exchange.continuation.as_array() {
            input.extend(items.clone());
        }

        for result in &exchange.results {
            input.push(json!({
                "type": "function_call_output",
                "call_id": result.id,
                "output": json!(result).to_string(),
            }));
        }
    }

    let mut tools: Vec<_> = context
        .tools
        .iter()
        .map(|tool| {
            json!({
                "type": "function",
                "name": tool.name,
                "description": tool.description,
                "parameters": tool.parameters,
                "strict": true,
            })
        })
        .collect();

    let web_enabled = profile.model == Provider::OpenAi.default_model()
        || supported_models(Provider::OpenAi).contains(&profile.model.as_str());
    let instructions = if web_enabled {
        tools.push(json!({"type": "web_search", "external_web_access": true}));
        WEB_RESEARCH_INSTRUCTIONS
    } else {
        "Texo has no hosted web tool configured for this model. If research is requested, \
         explain this and suggest choosing a supported model with /model or pasting an excerpt."
    };

    let mut body = json!({
        "model": profile.model,
        "instructions": instructions,
        "input": input,
        "store": false,
        "reasoning": {"effort": "low"},
        // Reasoning items must be replayable without server-side response storage.
        "include": ["reasoning.encrypted_content"],
        "max_output_tokens": MAX_OUTPUT_TOKENS,
        "tools": tools,
        "text": {
            "format": {
                "type": "json_schema",
                "name": "resume_edit",
                "strict": true,
                "schema": reply_schema(),
            },
        },
    });

    if web_enabled {
        body["max_tool_calls"] = json!(MAX_OPENAI_WEB_CALLS_PER_RESPONSE);
    }

    body
}

fn anthropic_payload(profile: &Profile, context: &TurnContext<'_>) -> Value {
    let mut messages = vec![json!({"role": "user", "content": context.prompt})];

    for exchange in context.history {
        messages.push(json!({"role": "assistant", "content": exchange.continuation}));

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

        messages.push(json!({"role": "user", "content": results}));
    }

    let mut tools = vec![
        json!({
            "type": "web_search_20250305",
            "name": "web_search",
            "max_uses": MAX_ANTHROPIC_WEB_USES,
        }),
        json!({
            "type": "web_fetch_20250910",
            "name": "web_fetch",
            "max_uses": MAX_ANTHROPIC_WEB_USES,
            "url_sources": {"client_tool_results": {"type": "none"}},
            "citations": {"enabled": true},
            "max_content_tokens": MAX_ANTHROPIC_FETCH_TOKENS,
        }),
    ];
    tools.extend(context.tools.iter().map(|tool| {
        json!({
            "name": tool.name,
            "description": tool.description,
            "input_schema": tool.parameters,
        })
    }));

    // Claude citations require plain text output. Resume tools still stage and validate edits.
    let system = format!(
        "{WEB_RESEARCH_INSTRUCTIONS} Return the final reply as plain text. \
         Use the Texo document tools for resume edits."
    );

    json!({
        "model": profile.model,
        "system": system,
        "max_tokens": MAX_OUTPUT_TOKENS,
        "messages": messages,
        "tools": tools,
    })
}

fn parse_turn(provider: Provider, response: &Value) -> Result<AgentTurn> {
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

fn response_blocks(provider: Provider, response: &Value) -> Vec<&Value> {
    match provider {
        Provider::OpenAi => response["output"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|item| item["type"] == "message")
            .flat_map(|item| item["content"].as_array().into_iter().flatten())
            .collect(),
        Provider::Anthropic => response["content"]
            .as_array()
            .into_iter()
            .flatten()
            .collect(),
        _ => Vec::new(),
    }
}

fn parse_response(provider: Provider, response: &Value) -> Result<AssistantReply> {
    let mut text = String::new();
    for block in response_blocks(provider, response) {
        ensure!(
            block["type"] != "refusal",
            "The provider declined this request. No resume edit was applied"
        );
        if matches!(block["type"].as_str(), Some("text" | "output_text")) {
            text.push_str(block["text"].as_str().unwrap_or_default());
        }
    }

    if provider == Provider::OpenAi {
        return parse_reply(text.as_bytes());
    }

    ensure!(
        !text.trim().is_empty(),
        "The assistant returned an empty reply"
    );
    Ok(AssistantReply {
        reply: text,
        proposed_tex: None,
    })
}

fn append_public_sources(
    reply: &mut String,
    provider: Provider,
    blocks: &[&Value],
    earlier_messages: &[Value],
) {
    let (field, kind) = match provider {
        Provider::OpenAi => ("annotations", "url_citation"),
        _ => ("citations", "web_search_result_location"),
    };
    let mut sources = Vec::new();

    for block in blocks {
        for citation in block[field].as_array().into_iter().flatten() {
            if citation["type"] == kind {
                add_public_source(&mut sources, &citation["url"]);
            }
        }

        add_fetch_source(&mut sources, block);
    }

    // Claude's fetched-document citations can be offsets, including for pages read before a local tool call.
    for message in earlier_messages {
        if message["role"] == "assistant" {
            for block in message["content"].as_array().into_iter().flatten() {
                add_fetch_source(&mut sources, block);
            }
        }
    }

    sources.retain(|url| !reply.contains(url));

    if !sources.is_empty() {
        reply.push_str("\n\nSources");
        for url in sources {
            reply.push_str("\n- ");
            reply.push_str(url);
        }
    }
}

fn add_fetch_source<'a>(sources: &mut Vec<&'a str>, block: &'a Value) {
    if block["type"] == "web_fetch_tool_result" && block["content"]["type"] == "web_fetch_result" {
        add_public_source(sources, &block["content"]["url"]);
    }
}

fn add_public_source<'a>(sources: &mut Vec<&'a str>, value: &'a Value) {
    let Some(link) = value.as_str() else {
        return;
    };
    if link.contains(char::is_whitespace)
        || link.contains(char::is_control)
        || sources.contains(&link)
    {
        return;
    }

    let Ok(url) = reqwest::Url::parse(link) else {
        return;
    };
    let public_domain = url.domain().is_some_and(|domain| {
        let domain = domain.trim_end_matches('.');
        domain.contains('.') && !domain.ends_with(".localhost") && !domain.ends_with(".local")
    });

    if matches!(url.scheme(), "http" | "https")
        && public_domain
        && url.username().is_empty()
        && url.password().is_none()
    {
        sources.push(link);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_claude_continuations_return_a_provider_error() {
        let context = TurnContext {
            prompt: "Read this public README",
            tools: &[],
            history: &[],
        };
        let profile = Profile::new(Provider::Anthropic);
        let pause = json!({
            "stop_reason": "pause_turn",
            "content": [{
                "type": "server_tool_use",
                "name": "web_search",
                "id": "srvtoolu_search",
                "input": {"query": "Rust license"}
            }]
        });

        for malformed in [json!([]), json!("unexpected body"), json!({})] {
            let mut request = ProviderRequest::new(&profile, &context);
            assert!(request.receive(pause.clone()).unwrap().is_none());

            let error = request.receive(malformed).unwrap_err();
            assert_eq!(
                error.to_string(),
                "Claude returned invalid response content"
            );
        }
    }

    #[test]
    fn claude_pauses_stop_at_the_continuation_limit() {
        let context = TurnContext {
            prompt: "Read this public README",
            tools: &[],
            history: &[],
        };
        let mut request = ProviderRequest::new(&Profile::new(Provider::Anthropic), &context);

        for index in 0..=MAX_ANTHROPIC_PAUSES {
            let pause = json!({
                "stop_reason": "pause_turn",
                "content": [{
                    "type": "server_tool_use",
                    "name": "web_search",
                    "id": format!("srvtoolu_{index}"),
                    "input": {"query": "Rust license"}
                }]
            });

            let result = request.receive(pause);
            if index == MAX_ANTHROPIC_PAUSES {
                assert!(
                    result
                        .unwrap_err()
                        .to_string()
                        .contains("narrower question")
                );
            } else {
                assert!(result.unwrap().is_none());
            }
        }
    }

    #[test]
    fn unknown_openai_models_keep_document_tools_and_get_web_switching_guidance() {
        let profile = Profile {
            model: "custom-model".into(),
            ..Profile::new(Provider::OpenAi)
        };
        let tools = [crate::tools::ToolDefinition {
            name: "read_resume".into(),
            description: "Read the draft in memory.".into(),
            parameters: json!({"type": "object"}),
        }];
        let context = TurnContext {
            prompt: "Read this public README and use the draft",
            tools: &tools,
            history: &[],
        };

        let request = ProviderRequest::new(&profile, &context);

        assert_eq!(request.body["tools"].as_array().unwrap().len(), 1);
        assert_eq!(request.body["tools"][0]["name"], "read_resume");
        assert!(request.body.get("max_tool_calls").is_none());
        assert!(
            request.body["instructions"]
                .as_str()
                .unwrap()
                .contains("/model")
        );
    }

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
            assert!(parse_turn(Provider::OpenAi, &response).is_err());
        }
        assert!(parse_turn(Provider::Anthropic, &json!({"stop_reason":"max_tokens"})).is_err());
        for text in [
            "not json",
            r#"{"reply":"ok","proposed_tex":""}"#,
            r#"{"reply":"","proposed_tex":null}"#,
        ] {
            assert!(parse_reply(text.as_bytes()).is_err());
        }
    }
}
