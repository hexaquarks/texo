use std::time::Duration;

use anyhow::{Result, anyhow, bail, ensure};
use reqwest::{
    Client, StatusCode,
    header::{AUTHORIZATION, HeaderMap, HeaderValue},
};
use secrecy::{ExposeSecret, SecretString};
use serde_json::Value;
use zeroize::Zeroizing;

use crate::auth::Provider;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const ANTHROPIC_VERSION: &str = "2023-06-01";

pub(super) fn headers(provider: Provider, key: &SecretString) -> Result<HeaderMap> {
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

pub(super) fn provider_client(timeout: Duration) -> Result<Client> {
    Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(timeout)
        // Never forward credentials or resume content to a redirected endpoint.
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| anyhow!("Could not initialize the provider connection"))
}

/// A shared budget bounds the entire catalog, including paginated responses.
pub(super) async fn read_provider_json(
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

pub(super) fn connection_error(error: reqwest::Error) -> anyhow::Error {
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
