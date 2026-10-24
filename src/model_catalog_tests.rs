use super::*;
use serde_json::json;
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::Arc,
    thread,
};

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
                json!({
                    "data": [{
                        "id": "claude-sonnet-5-5",
                        "created_at": "2025-01-01T00:00:00Z"
                    }],
                    "has_more": true,
                    "last_id": "claude-sonnet-5-5"
                }),
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
