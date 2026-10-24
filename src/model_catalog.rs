use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use anyhow::{Result, anyhow, bail, ensure};
use reqwest::Client;
use secrecy::SecretString;
use serde_json::Value;

use super::{
    CANCEL_POLL_INTERVAL, MAX_RESPONSE_BYTES,
    provider_http::{connection_error, headers, provider_client, read_provider_json},
};
use crate::auth::Provider;

const CATALOG_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_CATALOG_PAGES: usize = 100;
const ANTHROPIC_PAGE_SIZE: &str = "1000";

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

#[cfg(test)]
#[path = "model_catalog_tests.rs"]
mod tests;
