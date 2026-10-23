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
use serde::Deserialize;
use serde_json::{Value, json};

use crate::auth::{Profile, Provider};

pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(180);
pub const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(50);
const MAX_OUTPUT_TOKENS: u32 = 16_384;
const OPENAI_ENDPOINT: &str = "https://api.openai.com/v1/responses";
const ANTHROPIC_ENDPOINT: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";

#[derive(Debug, Deserialize)]
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

pub fn request(
    profile: &Profile,
    key: SecretString,
    prompt: &str,
    cancelled: &AtomicBool,
) -> Result<AssistantReply> {
    let endpoint = match profile.provider {
        Provider::OpenAi => OPENAI_ENDPOINT,
        Provider::Anthropic => ANTHROPIC_ENDPOINT,
        _ => bail!("Choose an API provider with /login"),
    };
    request_at(profile, key, prompt, cancelled, endpoint, REQUEST_TIMEOUT)
}

fn request_at(
    profile: &Profile,
    key: SecretString,
    prompt: &str,
    cancelled: &AtomicBool,
    endpoint: &str,
    timeout: Duration,
) -> Result<AssistantReply> {
    profile.validate()?;
    ensure!(!cancelled.load(Ordering::Relaxed), "Assistant cancelled");
    let client = Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(timeout)
        // A redirect must never forward credentials or a resume to a different endpoint.
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| anyhow!("Could not initialize the provider connection"))?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        tokio::select! {
            result = send_request(&client, profile, &key, prompt, endpoint) => result,
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
    const CATALOG_TIMEOUT: Duration = Duration::from_secs(30);
    const MAX_CATALOG_PAGES: usize = 100;
    const ANTHROPIC_PAGE_SIZE: &str = "1000";
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let client = Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| anyhow!("Could not initialize model discovery"))?;
    runtime.block_on(async {
        let fetch = async {
            let mut models = Vec::new();
            let mut cursor = String::new();
            for _ in 0..MAX_CATALOG_PAGES {
                let mut request = client.get(endpoint).headers(headers(provider, &key)?);
                if provider == Provider::Anthropic {
                    request = request.query(&[("limit", ANTHROPIC_PAGE_SIZE)]);
                    if !cursor.is_empty() {
                        request = request.query(&[("after_id", &cursor)]);
                    }
                }
                let mut response = request.send().await.map_err(connection_error)?;
                check_status(provider, response.status())?;
                let mut bytes = Vec::new();
                while let Some(chunk) = response.chunk().await.map_err(connection_error)? {
                    ensure!(
                        bytes.len().saturating_add(chunk.len()) <= MAX_RESPONSE_BYTES,
                        "Model catalog is too large"
                    );
                    bytes.extend_from_slice(&chunk);
                }
                let page: Value = serde_json::from_slice(&bytes)
                    .map_err(|_| anyhow!("The provider returned an invalid model catalog"))?;
                models.extend(catalog_entries(provider, &page)?);
                if provider != Provider::Anthropic || page["has_more"] != true {
                    models.sort_by(|a, b| {
                        b.created_seconds.cmp(&a.created_seconds)
                            .then_with(|| b.created_at.cmp(&a.created_at))
                            .then_with(|| a.id.cmp(&b.id))
                    });
                    let mut seen = std::collections::HashSet::new();
                    let ids: Vec<_> = models.into_iter()
                        .map(|entry| entry.id)
                        .filter(|id| seen.insert(id.clone()))
                        .collect();
                    ensure!(
                        !ids.is_empty(),
                        "No text models are available for this account. Check provider access and retry"
                    );
                    return Ok(ids);
                }
                let next = page["last_id"].as_str()
                    .filter(|id| !id.is_empty() && *id != cursor)
                    .ok_or_else(|| anyhow!("The provider returned invalid model pagination"))?;
                cursor = next.into();
            }
            bail!("The model catalog has too many pages")
        };
        tokio::select! {
            result = tokio::time::timeout(CATALOG_TIMEOUT, fetch) => {
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

struct CatalogModel {
    id: String,
    created_seconds: u64,
    created_at: String,
}

fn catalog_entries(provider: Provider, page: &Value) -> Result<Vec<CatalogModel>> {
    let data = page["data"]
        .as_array()
        .ok_or_else(|| anyhow!("Invalid model catalog"))?;
    let mut models = Vec::new();
    for item in data {
        let Some(id) = item["id"].as_str() else {
            continue;
        };
        // OpenAI's catalog includes image, audio, embedding, and other non-chat models.
        // It has no capability field, so restrict this picker to text model families.
        let text_model = match provider {
            Provider::OpenAi => {
                (id.starts_with("gpt-")
                    || id.starts_with("chatgpt-")
                    || id.starts_with("o1")
                    || id.starts_with("o3")
                    || id.starts_with("o4"))
                    && ![
                        "audio",
                        "realtime",
                        "transcribe",
                        "tts",
                        "image",
                        "search",
                        "deep-research",
                        "codex",
                        "-pro",
                    ]
                    .iter()
                    .any(|kind| id.contains(kind))
            }
            Provider::Anthropic => id.starts_with("claude-"),
            _ => false,
        };
        let profile = Profile {
            provider,
            model: id.into(),
            ..Profile::default()
        };
        if text_model && profile.validate().is_ok() {
            models.push(CatalogModel {
                id: id.into(),
                created_seconds: item["created"].as_u64().unwrap_or_default(),
                created_at: item["created_at"].as_str().unwrap_or_default().into(),
            });
        }
    }
    Ok(models)
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

fn payload(profile: &Profile, prompt: &str) -> Value {
    match profile.provider {
        Provider::OpenAi => json!({
            "model": profile.model, "input": prompt, "store": false,
            "max_output_tokens": MAX_OUTPUT_TOKENS,
            "text": {"format": {"type": "json_schema", "name": "resume_edit", "strict": true, "schema": reply_schema()}},
        }),
        Provider::Anthropic => json!({
            "model": profile.model, "max_tokens": MAX_OUTPUT_TOKENS,
            "messages": [{"role": "user", "content": prompt}],
            "output_config": {"format": {"type": "json_schema", "schema": reply_schema()}},
        }),
        _ => Value::Null,
    }
}

async fn send_request(
    client: &Client,
    profile: &Profile,
    key: &SecretString,
    prompt: &str,
    endpoint: &str,
) -> Result<AssistantReply> {
    let mut response = client
        .post(endpoint)
        .headers(headers(profile.provider, key)?)
        .json(&payload(profile, prompt))
        .send()
        .await
        .map_err(connection_error)?;
    check_status(profile.provider, response.status())?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(connection_error)? {
        ensure!(
            bytes.len().saturating_add(chunk.len()) <= MAX_RESPONSE_BYTES,
            "The provider response is too large"
        );
        bytes.extend_from_slice(&chunk);
    }
    let response: Value = serde_json::from_slice(&bytes)
        .map_err(|_| anyhow!("The provider returned an invalid response. Please retry"))?;
    parse_response(profile.provider, &response)
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
        let result = request_at(
            &Profile::new(provider),
            "test-api-secret".into(),
            "current unsaved resume",
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
    fn catalog_fetches_current_ids_sorts_and_filters_non_text_models() {
        let (models, requests) = mock_catalog(
            Provider::OpenAi,
            vec![(
                "200 OK",
                json!({"data":[
                    {"id":"gpt-future", "created":30}, {"id":"gpt-old", "created":10},
                    {"id":"gpt-new", "created":20}, {"id":"gpt-image-new", "created":40},
                    {"id":"text-embedding-new", "created":50}, {"id":"gpt-future", "created":30}
                ]}),
            )],
        );
        assert_eq!(models.unwrap(), ["gpt-future", "gpt-new", "gpt-old"]);
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
                    json!({"data":[{"id":"claude-old", "created_at":"2025-01-01T00:00:00Z"}], "has_more":true,"last_id":"claude-old"}),
                ),
                (
                    "200 OK",
                    json!({"data":[{"id":"claude-new", "created_at":"2026-01-01T00:00:00Z"}], "has_more":false}),
                ),
            ],
        );
        assert_eq!(models.unwrap(), ["claude-new", "claude-old"]);
        assert!(requests[1].contains("after_id=claude-old"));
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
