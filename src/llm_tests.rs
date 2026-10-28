use super::test_support::{mock_request, mock_turn};
use super::*;
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::Arc,
    thread,
    time::Instant,
};

#[test]
fn authorization_headers_are_redacted_in_debug_output() {
    for provider in [Provider::OpenAi, Provider::Anthropic] {
        let headers = headers(provider, &SecretString::from("synthetic-private-key")).unwrap();
        assert!(!format!("{headers:?}").contains("synthetic-private-key"));
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
            json!({
                "status": "completed",
                "output": [{
                    "type": "message",
                    "content": [{"type": "output_text", "text": reply}]
                }]
            })
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
            assert!(wire.contains("anthropic-version: 2023-06-01"));
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
        write!(
            stream,
            "HTTP/1.1 307 Temporary Redirect\r\nLocation: {redirect}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
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

#[test]
fn tool_turns_reuse_the_http_connection() {
    use std::io::{BufRead, BufReader};

    const REQUEST_TIMEOUT: Duration = Duration::from_secs(3);

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}/responses", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        stream.set_read_timeout(Some(REQUEST_TIMEOUT)).unwrap();
        let mut connection = BufReader::new(stream);

        // Both requests must arrive on the same socket, across separate block_on calls.
        for _ in 0..2 {
            let mut content_length = None;
            loop {
                let mut line = String::new();
                assert!(connection.read_line(&mut line).unwrap() > 0);
                if line == "\r\n" {
                    break;
                }
                if let Some(value) = line.to_lowercase().strip_prefix("content-length:") {
                    content_length = Some(value.trim().parse::<usize>().unwrap());
                }
            }

            let mut body = vec![0; content_length.unwrap()];
            connection.read_exact(&mut body).unwrap();
            let request: Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(request["reasoning"]["effort"], "low");

            let response = json!({
                "status": "completed",
                "output": [{
                    "type": "message",
                    "content": [{
                        "type": "output_text",
                        "text": r#"{"reply":"Advice","proposed_tex":null}"#
                    }]
                }]
            })
            .to_string();
            write!(
                connection.get_mut(),
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\n\r\n{response}",
                response.len()
            ).unwrap();
            connection.get_mut().flush().unwrap();
        }
    });

    let client = TurnClient::with_timeout(REQUEST_TIMEOUT).unwrap();
    let context = TurnContext {
        prompt: "synthetic resume",
        tools: &[],
        history: &[],
    };
    let profile = Profile::new(Provider::OpenAi);
    let key = SecretString::from("synthetic-key");

    for _ in 0..2 {
        let turn = client
            .request_turn_at(&profile, &key, &context, &AtomicBool::new(false), &endpoint)
            .unwrap();
        assert!(matches!(turn, AgentTurn::Complete(_)));
    }

    server.join().unwrap();
}
