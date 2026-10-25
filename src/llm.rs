use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use anyhow::{Result, anyhow, bail, ensure};
use reqwest::Client;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    agent::AgentTurn,
    auth::{Profile, Provider},
    tools::{ToolCall, ToolDefinition, ToolExchange},
};

pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(180);
pub const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(50);
const OPENAI_ENDPOINT: &str = "https://api.openai.com/v1/responses";
const ANTHROPIC_ENDPOINT: &str = "https://api.anthropic.com/v1/messages";

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantReply {
    pub reply: String,
    pub proposed_tex: Option<String>,
}

pub fn reply_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "reply": {"type": "string"},
            "proposed_tex": {"type": ["string", "null"]},
        },
        "required": ["reply", "proposed_tex"],
        "additionalProperties": false,
    })
}

pub fn parse_reply(bytes: &[u8]) -> Result<AssistantReply> {
    ensure!(
        bytes.len() <= MAX_RESPONSE_BYTES,
        "The assistant response is too large"
    );

    let reply: AssistantReply = serde_json::from_slice(bytes).map_err(|_| {
        anyhow!("The assistant returned an invalid structured response. Please retry")
    })?;
    ensure!(
        !reply.reply.trim().is_empty(),
        "The assistant returned an empty reply"
    );
    ensure!(
        reply
            .proposed_tex
            .as_ref()
            .is_none_or(|text| !text.trim().is_empty()),
        "The assistant proposed an empty document"
    );
    Ok(reply)
}

pub struct TurnContext<'a> {
    pub prompt: &'a str,
    pub tools: &'a [ToolDefinition],
    pub history: &'a [ToolExchange],
}

pub fn request_turn(
    profile: &Profile,
    key: SecretString,
    context: &TurnContext<'_>,
    cancelled: &AtomicBool,
) -> Result<AgentTurn> {
    let endpoint = match profile.provider {
        Provider::OpenAi => OPENAI_ENDPOINT,
        Provider::Anthropic => ANTHROPIC_ENDPOINT,
        _ => bail!("Choose an API provider with /login"),
    };
    request_turn_at(profile, key, context, cancelled, endpoint, REQUEST_TIMEOUT)
}

#[cfg(test)]
fn request_at(
    profile: &Profile,
    key: SecretString,
    prompt: &str,
    cancelled: &AtomicBool,
    endpoint: &str,
    timeout: Duration,
) -> Result<AssistantReply> {
    let context = TurnContext {
        prompt,
        tools: &[],
        history: &[],
    };
    match request_turn_at(profile, key, &context, cancelled, endpoint, timeout)? {
        AgentTurn::Complete(reply) => Ok(reply),
        AgentTurn::Tools { .. } => bail!("Unexpected tool call"),
    }
}

fn request_turn_at(
    profile: &Profile,
    key: SecretString,
    context: &TurnContext<'_>,
    cancelled: &AtomicBool,
    endpoint: &str,
    timeout: Duration,
) -> Result<AgentTurn> {
    profile.validate()?;
    ensure!(!cancelled.load(Ordering::Relaxed), "Assistant cancelled");

    let client = provider_client(timeout)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    runtime.block_on(async {
        tokio::select! {
            result = send_request(&client, profile, &key, context, endpoint) => result,
            _ = async {
                while !cancelled.load(Ordering::Relaxed) {
                    tokio::time::sleep(CANCEL_POLL_INTERVAL).await;
                }
            } => Err(anyhow!("Assistant cancelled")),
        }
    })
}

async fn send_request(
    client: &Client,
    profile: &Profile,
    key: &SecretString,
    context: &TurnContext<'_>,
    endpoint: &str,
) -> Result<AgentTurn> {
    let response = client
        .post(endpoint)
        .headers(headers(profile.provider, key)?)
        .json(&payload(profile, context))
        .send()
        .await
        .map_err(connection_error)?;
    let mut remaining_bytes = MAX_RESPONSE_BYTES;
    let response = read_provider_json(response, profile.provider, &mut remaining_bytes).await?;
    parse_turn(profile.provider, &response)
}

/// Codex CLI has no host tool callback here, so it emits the same tool protocol as JSON.
pub fn codex_turn_schema() -> Value {
    let mut schema = reply_schema();
    schema["properties"]["tool_calls"] = json!({
        "type": "array",
        "items": {
            "type": "object",
            "properties": {
                "id": {"type": "string"},
                "name": {"type": "string"},
                "arguments_json": {"type": "string"}
            },
            "required": ["id", "name", "arguments_json"],
            "additionalProperties": false
        }
    });
    schema["required"] = json!(["reply", "proposed_tex", "tool_calls"]);

    schema
}

pub fn parse_codex_turn(bytes: &[u8]) -> Result<AgentTurn> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Call {
        id: String,
        name: String,
        arguments_json: String,
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Response {
        reply: String,
        proposed_tex: Option<String>,
        tool_calls: Vec<Call>,
    }

    ensure!(
        bytes.len() <= MAX_RESPONSE_BYTES,
        "The assistant response is too large"
    );

    let response: Response =
        serde_json::from_slice(bytes).map_err(|_| anyhow!("Invalid Codex tool response"))?;

    if response.tool_calls.is_empty() {
        let reply = parse_reply(&serde_json::to_vec(&AssistantReply {
            reply: response.reply,
            proposed_tex: response.proposed_tex,
        })?)?;
        return Ok(AgentTurn::Complete(reply));
    }

    ensure!(
        response.proposed_tex.is_none(),
        "Codex returned tools and a final edit in the same turn"
    );

    let calls = response
        .tool_calls
        .into_iter()
        .map(|call| {
            Ok(ToolCall {
                id: call.id,
                name: call.name,
                arguments: serde_json::from_str(&call.arguments_json)
                    .map_err(|_| anyhow!("Invalid tool arguments"))?,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(AgentTurn::Tools {
        calls,
        continuation: Value::Null,
    })
}

#[path = "model_catalog.rs"]
mod model_catalog;
#[path = "provider_http.rs"]
mod provider_http;
#[path = "provider_protocol.rs"]
mod provider_protocol;

pub use model_catalog::{list_models, supported_models};
use provider_http::{connection_error, headers, provider_client, read_provider_json};
use provider_protocol::{parse_turn, payload};

#[cfg(test)]
#[path = "llm_test_support.rs"]
mod test_support;
#[cfg(test)]
#[path = "llm_tests.rs"]
mod tests;
