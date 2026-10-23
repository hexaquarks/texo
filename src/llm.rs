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
