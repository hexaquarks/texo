use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use anyhow::{Result, anyhow, bail, ensure};
use reqwest::{
    Client, StatusCode,
    header::{AUTHORIZATION, HeaderMap, HeaderValue},
};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use zeroize::Zeroizing;

use crate::{
    agent::AgentTurn,
    auth::{Profile, Provider},
    tools::{ToolCall, ToolDefinition, ToolExchange},
};

pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(180);
pub const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const CATALOG_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_CATALOG_PAGES: usize = 100;
const ANTHROPIC_PAGE_SIZE: &str = "1000";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(50);
const MAX_OUTPUT_TOKENS: u32 = 16_384;
const OPENAI_ENDPOINT: &str = "https://api.openai.com/v1/responses";
const ANTHROPIC_ENDPOINT: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantReply {
    pub reply: String,
    pub proposed_tex: Option<String>,
}

pub fn reply_schema() -> Value {
    json!({
        "type": "object",
        "properties": {"reply": {"type": "string"}, "proposed_tex": {"type": ["string", "null"]}},
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

/// Fetch the account's current catalog; no generation request or resume content is sent.
pub fn list_models(
    provider: Provider,
    key: SecretString,
    cancelled: &AtomicBool,
) -> Result<Vec<String>> {
    let endpoint = match provider {
        Provider::OpenAi => "https://api.openai.com/v1/models",
        Provider::Anthropic => "https://api.anthropic.com/v1/models",
        _ => bail!("Model discovery requires an API provider"),
    };
    list_models_at(provider, key, cancelled, endpoint)
}

fn list_models_at(
    provider: Provider,
    key: SecretString,
    cancelled: &AtomicBool,
    endpoint: &str,
) -> Result<Vec<String>> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let client = provider_client(CATALOG_TIMEOUT)?;
    runtime.block_on(async {
        tokio::select! {
            result = tokio::time::timeout(CATALOG_TIMEOUT, fetch_models(&client, provider, &key, endpoint)) => {
                result.map_err(|_| anyhow!("Model discovery timed out. Press Enter to retry"))?
            },
            _ = async {
                while !cancelled.load(Ordering::Relaxed) {
                    tokio::time::sleep(CANCEL_POLL_INTERVAL).await;
                }
            } => Err(anyhow!("Model discovery cancelled")),
        }
    })
}

async fn fetch_models(
    client: &Client,
    provider: Provider,
    key: &SecretString,
    endpoint: &str,
) -> Result<Vec<String>> {
    let mut models = Vec::new();
    let mut cursor = String::new();
    let mut remaining_bytes = MAX_RESPONSE_BYTES;
    for _ in 0..MAX_CATALOG_PAGES {
        let mut request = client.get(endpoint).headers(headers(provider, key)?);
        if provider == Provider::Anthropic {
            request = request.query(&[("limit", ANTHROPIC_PAGE_SIZE)]);
            if !cursor.is_empty() {
                request = request.query(&[("after_id", &cursor)]);
            }
        }

        let response = request.send().await.map_err(connection_error)?;
        let page = read_provider_json(response, provider, &mut remaining_bytes).await?;
        models.extend(catalog_entries(&page)?);
        if provider != Provider::Anthropic || page["has_more"] != true {
            let ids = curated_models(provider, &models);
            ensure!(
                !ids.is_empty(),
                "None of Texo’s supported models are available for this account"
            );
            return Ok(ids);
        }

        let next = page["last_id"]
            .as_str()
            .filter(|id| !id.is_empty() && *id != cursor)
            .ok_or_else(|| anyhow!("The provider returned invalid model pagination"))?;
        cursor = next.into();
    }
    bail!("The model catalog has too many pages")
}

// Keep the picker focused on established general-purpose releases, not snapshots or aliases.
// Sources: developers.openai.com/api/docs/models/all and platform.claude.com/docs/en/models/overview.
const OPENAI_MODELS: &[&str] = &[
    "gpt-6.1-sol",
    "gpt-6-astra",
    "gpt-6-luna",
    "gpt-6-sol",
    "gpt-5.6-sol",
    "gpt-5.6-terra",
    "gpt-5.6-luna",
];
const ANTHROPIC_MODELS: &[&str] = &[
    "claude-fable-5-1",
    "claude-opus-5-5",
    "claude-sonnet-5-5",
    "claude-haiku-5-5",
];

pub fn supported_models(provider: Provider) -> &'static [&'static str] {
    match provider {
        Provider::OpenAi | Provider::Codex => OPENAI_MODELS,
        Provider::Anthropic => ANTHROPIC_MODELS,
        Provider::Disconnected => &[],
    }
}

fn curated_models(provider: Provider, available: &[String]) -> Vec<String> {
    supported_models(provider)
        .iter()
        .filter(|id| available.iter().any(|available_id| available_id == **id))
        .map(|id| (*id).to_owned())
        .collect()
}

fn catalog_entries(page: &Value) -> Result<Vec<String>> {
    let data = page["data"]
        .as_array()
        .ok_or_else(|| anyhow!("Invalid model catalog"))?;
    Ok(data
        .iter()
        .filter_map(|item| item["id"].as_str().map(str::to_owned))
        .collect())
}

fn headers(provider: Provider, key: &SecretString) -> Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    let (name, value) = match provider {
        Provider::OpenAi => (AUTHORIZATION, format!("Bearer {}", key.expose_secret())),
        Provider::Anthropic => (
            reqwest::header::HeaderName::from_static("x-api-key"),
            key.expose_secret().to_owned(),
        ),
        _ => bail!("Choose an API provider with /login"),
    };
    let value = Zeroizing::new(value);
    let mut value = HeaderValue::from_str(&value)
        .map_err(|_| anyhow!("Invalid API key. Use /login to replace it"))?;
    value.set_sensitive(true);
    headers.insert(name, value);
    if provider == Provider::Anthropic {
        headers.insert(
            "anthropic-version",
            HeaderValue::from_static(ANTHROPIC_VERSION),
        );
    }
    Ok(headers)
}

fn payload(profile: &Profile, context: &TurnContext<'_>) -> Value {
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

fn provider_client(timeout: Duration) -> Result<Client> {
    Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(timeout)
        // Never forward credentials or resume content to a redirected endpoint.
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| anyhow!("Could not initialize the provider connection"))
}

/// A shared budget bounds the entire catalog, including paginated responses.
async fn read_provider_json(
    mut response: reqwest::Response,
    provider: Provider,
    remaining_bytes: &mut usize,
) -> Result<Value> {
    check_status(provider, response.status())?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(connection_error)? {
        ensure!(
            chunk.len() <= *remaining_bytes,
            "The provider response is too large"
        );
        *remaining_bytes -= chunk.len();
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| anyhow!("The provider returned an invalid response. Please retry"))
}

fn connection_error(error: reqwest::Error) -> anyhow::Error {
    // Never surface raw HTTP errors, URLs, response bodies, or headers: they can contain secrets.
    if error.is_timeout() {
        anyhow!("The provider request timed out. Please retry")
    } else {
        anyhow!("Could not reach the provider. Check your network connection and retry")
    }
}

fn check_status(provider: Provider, status: StatusCode) -> Result<()> {
    if status.is_success() {
        return Ok(());
    }
    let label = provider.label();
    match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
            bail!("{label} rejected the credentials or access. Use /login to reconnect")
        }
        StatusCode::TOO_MANY_REQUESTS => {
            bail!("{label} rate limit or quota reached. Check your API billing and retry later")
        }
        StatusCode::BAD_REQUEST | StatusCode::NOT_FOUND => bail!(
            "{label} could not use the selected model or request. Check the model ID with /login"
        ),
        _ if status.is_server_error() => {
            bail!("{label} is temporarily unavailable. Please retry later")
        }
        _ => bail!(
            "{label} could not complete the request (HTTP {})",
            status.as_u16()
        ),
    }
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
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::Arc,
        thread,
        time::Instant,
    };

    fn mock_request(
        provider: Provider,
        status: &str,
        body: Value,
    ) -> (Result<AssistantReply>, String) {
        let context = TurnContext {
            prompt: "current unsaved resume",
            tools: &[],
            history: &[],
        };
        let (result, wire) = mock_turn(provider, status, body, &context);
        (
            result.and_then(|turn| match turn {
                AgentTurn::Complete(reply) => Ok(reply),
                _ => bail!("Unexpected tools"),
            }),
            wire,
        )
    }

    fn mock_turn(
        provider: Provider,
        status: &str,
        body: Value,
        context: &TurnContext<'_>,
    ) -> (Result<AgentTurn>, String) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/test", listener.local_addr().unwrap());
        let status = status.to_owned();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut buffer = [0; 4096];
            loop {
                let count = stream.read(&mut buffer).unwrap();
                bytes.extend_from_slice(&buffer[..count]);
                if let Some(end) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
                    let header = String::from_utf8_lossy(&bytes[..end]).to_lowercase();
                    let length: usize = header
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length: "))
                        .unwrap()
                        .parse()
                        .unwrap();
                    if bytes.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            let body = body.to_string();
            write!(stream, "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            String::from_utf8(bytes).unwrap()
        });
        let result = request_turn_at(
            &Profile::new(provider),
            "test-api-secret".into(),
            context,
            &AtomicBool::new(false),
            &endpoint,
            Duration::from_secs(3),
        );
        (result, server.join().unwrap())
    }

    fn mock_catalog(
        provider: Provider,
        pages: Vec<(&str, Value)>,
    ) -> (Result<Vec<String>>, Vec<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/v1/models", listener.local_addr().unwrap());
        let pages: Vec<_> = pages
            .into_iter()
            .map(|(status, body)| (status.to_owned(), body))
            .collect();
        let server = thread::spawn(move || {
            let mut requests = Vec::new();
            for (status, body) in pages {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut bytes = Vec::new();
                let mut buffer = [0; 4096];
                while !bytes.windows(4).any(|part| part == b"\r\n\r\n") {
                    let count = stream.read(&mut buffer).unwrap();
                    assert!(count > 0);
                    bytes.extend_from_slice(&buffer[..count]);
                }
                requests.push(String::from_utf8(bytes).unwrap());
                let body = body.to_string();
                write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
            requests
        });
        let result = list_models_at(
            provider,
            "catalog-secret".into(),
            &AtomicBool::new(false),
            &endpoint,
        );
        (result, server.join().unwrap())
    }

    #[test]
    fn catalog_fetches_only_available_curated_models_in_recommended_order() {
        let (models, requests) = mock_catalog(
            Provider::OpenAi,
            vec![(
                "200 OK",
                json!({"data":[
                    {"id":"gpt-6.1-sol", "created":30}, {"id":"gpt-5.6-sol", "created":10},
                    {"id":"gpt-6-luna", "created":20}, {"id":"gpt-image-new", "created":40},
                    {"id":"text-embedding-new", "created":50}, {"id":"gpt-6.1-sol", "created":30}
                ]}),
            )],
        );
        assert_eq!(
            models.unwrap(),
            ["gpt-6.1-sol", "gpt-6-luna", "gpt-5.6-sol"]
        );
        assert!(requests[0].starts_with("GET /v1/models "));
        assert!(requests[0].contains("authorization: Bearer catalog-secret"));
        assert!(!requests[0].contains("x-api-key"));
    }

    #[test]
    fn anthropic_catalog_follows_pagination_and_keeps_credentials_provider_specific() {
        let (models, requests) = mock_catalog(
            Provider::Anthropic,
            vec![
                (
                    "200 OK",
                    json!({"data":[{"id":"claude-sonnet-5-5", "created_at":"2025-01-01T00:00:00Z"}], "has_more":true,"last_id":"claude-sonnet-5-5"}),
                ),
                (
                    "200 OK",
                    json!({"data":[{"id":"claude-opus-5-5", "created_at":"2026-01-01T00:00:00Z"}], "has_more":false}),
                ),
            ],
        );
        assert_eq!(models.unwrap(), ["claude-opus-5-5", "claude-sonnet-5-5"]);
        assert!(requests[1].contains("after_id=claude-sonnet-5-5"));
        assert!(requests[0].contains("x-api-key: catalog-secret"));
        assert!(!requests[0].contains("authorization:"));
    }

    #[test]
    fn catalog_request_can_be_cancelled_while_waiting_for_the_provider() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = Arc::clone(&cancelled);
        let worker = thread::spawn(move || {
            list_models_at(
                Provider::OpenAi,
                "test-secret".into(),
                &worker_cancelled,
                &endpoint,
            )
        });
        let (_connection, _) = listener.accept().unwrap();
        cancelled.store(true, Ordering::Relaxed);
        assert_eq!(
            worker.join().unwrap().unwrap_err().to_string(),
            "Model discovery cancelled"
        );
    }

    #[test]
    fn curated_picker_excludes_aliases_snapshots_and_specialist_models() {
        let mut available: Vec<String> = OPENAI_MODELS.iter().map(|id| (*id).into()).collect();
        available.extend(
            [
                "gpt-6.1-sol-latest",
                "gpt-6.1-sol-2026-10-01",
                "gpt-image-new",
                "gpt-4o",
                "gpt-6.1-sol-preview",
                "gpt-6.1-sol",
            ]
            .map(str::to_owned),
        );
        let models = curated_models(Provider::OpenAi, &available);
        assert_eq!(models, OPENAI_MODELS);
        assert_eq!(models.len(), 7);
        assert_eq!(
            curated_models(Provider::OpenAi, &["gpt-6-luna".into()]),
            ["gpt-6-luna"]
        );
        assert!(curated_models(Provider::Anthropic, &available).is_empty());
    }

    #[test]
    fn authorization_headers_are_redacted_in_debug_output() {
        for provider in [Provider::OpenAi, Provider::Anthropic] {
            let headers = headers(provider, &SecretString::from("synthetic-private-key")).unwrap();
            assert!(!format!("{headers:?}").contains("synthetic-private-key"));
        }
    }

    #[test]
    fn catalog_size_limit_applies_across_pages() {
        let padding = "x".repeat(MAX_RESPONSE_BYTES / 2);
        let first = json!({"data":[], "padding":padding, "has_more":true, "last_id":"page-one"});
        let second = json!({"data":[], "padding":padding, "has_more":false});
        let (result, _) = mock_catalog(
            Provider::Anthropic,
            vec![("200 OK", first), ("200 OK", second)],
        );
        assert!(result.unwrap_err().to_string().contains("too large"));
    }

    #[test]
    fn catalog_errors_are_safe_and_empty_catalogs_fail_explicitly() {
        for (status, body) in [
            ("401 Unauthorized", json!({"error":"catalog-secret"})),
            ("200 OK", json!({"data":[]})),
            ("200 OK", json!({"invalid":"catalog-secret"})),
            ("302 Found", json!({})),
        ] {
            let (result, _) = mock_catalog(Provider::OpenAi, vec![(status, body)]);
            assert!(!result.unwrap_err().to_string().contains("catalog-secret"));
        }
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

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "requires Tectonic and cached LaTeX packages"]
    fn both_http_adapters_drive_real_tools_through_compile_failure_and_repair() {
        use crate::{agent, resume_tools::ResumeTools};

        let valid = r"\documentclass{article}\begin{document}Revised\end{document}";
        let broken = r"\documentclass{article}\begin{document}\undefinedcommand\end{document}";

        for provider in [Provider::OpenAi, Provider::Anthropic] {
            let folder = tempfile::tempdir().unwrap();
            let saved_path = folder.path().join("main.tex");
            std::fs::write(&saved_path, "saved source").unwrap();

            let mut tools = ResumeTools::new("main.tex".into(), "unsaved source".into())
                .in_directory(folder.path().to_owned());

            let result = agent::run(
                "Edit this resume",
                &mut tools,
                &AtomicBool::new(false),
                |prompt, definitions, history, cancelled| {
                    let step = history.len();
                    let next_call = match step {
                        0 => Some(("read_resume", json!({}))),
                        1 => {
                            assert_eq!(history[0].results[0].output["tex"], "unsaved source");
                            Some(("propose_edit", json!({"tex": broken})))
                        }
                        2 => Some(("compile_resume", json!({}))),
                        3 => {
                            assert!(history[2].results[0].is_error);
                            Some(("propose_edit", json!({"tex": valid})))
                        }
                        4 => Some(("compile_resume", json!({}))),
                        _ => {
                            assert!(!history[4].results[0].is_error);
                            None
                        }
                    };

                    let response = if let Some((name, arguments)) = next_call {
                        let id = format!("call-{step}");

                        if provider == Provider::OpenAi {
                            json!({
                                "status": "completed",
                                "output": [{
                                    "type": "function_call",
                                    "call_id": id,
                                    "name": name,
                                    "arguments": arguments.to_string()
                                }]
                            })
                        } else {
                            json!({
                                "stop_reason": "tool_use",
                                "content": [{
                                    "type": "tool_use",
                                    "id": id,
                                    "name": name,
                                    "input": arguments
                                }]
                            })
                        }
                    } else {
                        let final_text =
                            json!({"reply": "Updated draft", "proposed_tex": null}).to_string();

                        if provider == Provider::OpenAi {
                            json!({
                                "status": "completed",
                                "output": [{
                                    "type": "message",
                                    "content": [{"type": "output_text", "text": final_text}]
                                }]
                            })
                        } else {
                            json!({
                                "stop_reason": "end_turn",
                                "content": [{"type": "text", "text": final_text}]
                            })
                        }
                    };

                    let context = TurnContext {
                        prompt,
                        tools: definitions,
                        history,
                    };
                    let (turn, wire) = mock_turn(provider, "200 OK", response, &context);

                    assert!(!cancelled.load(Ordering::Relaxed));
                    assert!(wire.contains("compile_resume"));

                    if step > 0 {
                        assert!(wire.contains(&format!("call-{}", step - 1)));
                    }

                    turn
                },
            )
            .unwrap();

            assert_eq!(result.proposed_tex.as_deref(), Some(valid));
            assert_eq!(std::fs::read_to_string(saved_path).unwrap(), "saved source");
        }
    }

    #[test]
    fn native_tool_calls_and_results_round_trip_for_both_providers() {
        let tools = vec![ToolDefinition {
            name: "read_resume".into(),
            description: "Read current buffer".into(),
            parameters: json!({
                "type": "object",
                "properties": {},
                "required": [],
                "additionalProperties": false
            }),
        }];

        for provider in [Provider::OpenAi, Provider::Anthropic] {
            let tool_response = if provider == Provider::OpenAi {
                json!({
                    "status": "completed",
                    "output": [
                        {
                            "type": "reasoning",
                            "id": "r1",
                            "encrypted_content": "opaque",
                            "summary": []
                        },
                        {
                            "type": "function_call",
                            "call_id": "call1",
                            "name": "read_resume",
                            "arguments": "{}"
                        }
                    ]
                })
            } else {
                json!({
                    "stop_reason": "tool_use",
                    "content": [{
                        "type": "tool_use",
                        "id": "call1",
                        "name": "read_resume",
                        "input": {}
                    }]
                })
            };

            let context = TurnContext {
                prompt: "Edit current resume",
                tools: &tools,
                history: &[],
            };
            let (turn, wire) = mock_turn(provider, "200 OK", tool_response, &context);

            assert!(wire.contains("read_resume"));
            assert!(wire.contains(if provider == Provider::OpenAi {
                "parameters"
            } else {
                "input_schema"
            }));

            let AgentTurn::Tools {
                calls,
                continuation,
            } = turn.unwrap()
            else {
                panic!("expected tool call")
            };
            assert_eq!(calls[0].id, "call1");

            let history = vec![ToolExchange {
                calls,
                continuation,
                results: vec![crate::tools::ToolResult {
                    id: "call1".into(),
                    is_error: false,
                    output: json!({"tex":"unsaved source"}),
                }],
            }];

            let final_text = json!({"reply":"Done", "proposed_tex":null}).to_string();
            let final_response = if provider == Provider::OpenAi {
                json!({
                    "status": "completed",
                    "output": [{
                        "type": "message",
                        "content": [{"type": "output_text", "text": final_text}]
                    }]
                })
            } else {
                json!({"stop_reason":"end_turn", "content":[{"type":"text", "text":final_text}]})
            };

            let context = TurnContext {
                prompt: "Edit current resume",
                tools: &tools,
                history: &history,
            };
            let (turn, wire) = mock_turn(provider, "200 OK", final_response, &context);

            assert!(matches!(turn.unwrap(), AgentTurn::Complete(_)));
            assert!(wire.contains("unsaved source"));
            assert!(wire.contains("call1"));
            assert!(wire.contains(if provider == Provider::OpenAi {
                "function_call_output"
            } else {
                "tool_result"
            }));
            if provider == Provider::OpenAi {
                assert!(wire.contains("encrypted_content"));
            }
        }
    }

    #[test]
    fn codex_adapter_rejects_mixed_edits_and_tools_and_parses_calls() {
        let mut response = json!({
            "reply": "",
            "proposed_tex": null,
            "tool_calls": [{
                "id": "one",
                "name": "read_resume",
                "arguments_json": "{}"
            }]
        });

        assert!(matches!(
            parse_codex_turn(&serde_json::to_vec(&response).unwrap()).unwrap(),
            AgentTurn::Tools { .. }
        ));

        response["proposed_tex"] = json!("partial edit");
        assert!(parse_codex_turn(&serde_json::to_vec(&response).unwrap()).is_err());

        response["proposed_tex"] = Value::Null;
        response["tool_calls"][0]["arguments_json"] = json!("invalid json");
        assert!(parse_codex_turn(&serde_json::to_vec(&response).unwrap()).is_err());
    }

    #[test]
    fn both_providers_send_only_their_own_auth_header_and_parse_edits() {
        let reply = json!({"reply":"Updated wording", "proposed_tex":"new document"}).to_string();
        for provider in [Provider::OpenAi, Provider::Anthropic] {
            let body = if provider == Provider::OpenAi {
                json!({"status":"completed", "output":[{"type":"message", "content":[{"type":"output_text", "text":reply}]}]})
            } else {
                json!({"stop_reason":"end_turn", "content":[{"type":"text", "text":reply}]})
            };
            let (response, wire) = mock_request(provider, "200 OK", body);
            assert_eq!(
                response.unwrap().proposed_tex.as_deref(),
                Some("new document")
            );
            assert!(wire.contains("current unsaved resume"));
            if provider == Provider::OpenAi {
                assert!(wire.contains("authorization: Bearer test-api-secret"));
                assert!(!wire.contains("x-api-key"));
                assert!(wire.contains("\"store\":false"));
            } else {
                assert!(wire.contains("x-api-key: test-api-secret"));
                assert!(!wire.contains("authorization:"));
                assert!(wire.contains(ANTHROPIC_VERSION));
            }
        }
    }

    #[test]
    fn provider_errors_do_not_echo_secrets_or_response_bodies() {
        for status in [
            "401 Unauthorized",
            "403 Forbidden",
            "429 Too Many Requests",
            "500 Server Error",
            "302 Found",
        ] {
            let (result, _) = mock_request(
                Provider::OpenAi,
                status,
                json!({"error":"test-api-secret private resume"}),
            );
            let error = format!("{:#}", result.unwrap_err());
            assert!(!error.contains("test-api-secret"));
            assert!(!error.contains("private resume"));
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

    #[test]
    fn timeouts_and_oversized_responses_fail_without_an_edit() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let worker = thread::spawn(move || {
            request_at(
                &Profile::new(Provider::Anthropic),
                "test-secret".into(),
                "hello",
                &AtomicBool::new(false),
                &endpoint,
                Duration::from_millis(100),
            )
        });
        let (_connection, _) = listener.accept().unwrap();
        let error = worker.join().unwrap().unwrap_err().to_string();
        assert!(error.contains("timed out"));
        assert!(parse_reply(&vec![b' '; MAX_RESPONSE_BYTES + 1]).is_err());
    }

    #[test]
    fn redirects_never_forward_credentials_to_another_endpoint() {
        let source = TcpListener::bind("127.0.0.1:0").unwrap();
        let destination = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", source.local_addr().unwrap());
        let redirect = format!("http://{}", destination.local_addr().unwrap());
        destination.set_nonblocking(true).unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = source.accept().unwrap();
            let mut bytes = [0; 4096];
            let count = stream.read(&mut bytes).unwrap();
            assert!(count > 0);
            write!(stream, "HTTP/1.1 307 Temporary Redirect\r\nLocation: {redirect}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
        });
        let result = request_at(
            &Profile::new(Provider::OpenAi),
            "test-secret".into(),
            "hello",
            &AtomicBool::new(false),
            &endpoint,
            Duration::from_secs(1),
        );
        assert!(result.is_err());
        server.join().unwrap();
        assert!(destination.accept().is_err());
    }

    #[test]
    fn cancelling_a_pending_network_request_returns_promptly() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = Arc::clone(&cancelled);
        let started = Instant::now();
        let worker = thread::spawn(move || {
            request_at(
                &Profile::new(Provider::OpenAi),
                "test-secret".into(),
                "hello",
                &worker_cancelled,
                &endpoint,
                REQUEST_TIMEOUT,
            )
        });
        let (_connection, _) = listener.accept().unwrap();
        cancelled.store(true, Ordering::Relaxed);
        assert_eq!(
            worker.join().unwrap().unwrap_err().to_string(),
            "Assistant cancelled"
        );
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
