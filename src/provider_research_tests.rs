use super::test_support::mock_turn_sequence;
use super::*;
use crate::tools::ToolResult;

const REPO: &str = "https://github.com/rust-lang/rust";
const README: &str = "https://github.com/rust-lang/rust/blob/main/README.md";
const PROMPT: &str = "Read the public Rust README and explain what it confirms about licensing.";

#[test]
fn openai_astra_replays_hosted_open_page_and_local_tool_output_then_shows_native_citations() {
    let profile = Profile {
        model: "gpt-6-astra".into(),
        ..Profile::new(Provider::OpenAi)
    };
    let hosted_items = json!([
        {
            "type": "reasoning",
            "id": "rs_1",
            "summary": [],
            "encrypted_content": "opaque-reasoning"
        },
        {
            "type": "web_search_call",
            "id": "ws_1",
            "status": "completed",
            "action": {"type": "open_page", "url": README}
        },
        {
            "type": "function_call",
            "id": "fc_1",
            "status": "completed",
            "call_id": "call_resume",
            "name": "read_resume",
            "arguments": "{}"
        }
    ]);
    let tools = [read_resume_tool()];
    let context = TurnContext {
        prompt: PROMPT,
        tools: &tools,
        history: &[],
    };

    let response = json!({"status": "completed", "output": hosted_items});
    let (turn, wire) = mock_turn_sequence(&profile, vec![ok(response)], &context);
    let initial = request_body(&wire[0]);

    assert_eq!(initial["model"], "gpt-6-astra");
    assert_eq!(initial["store"], false);
    assert_eq!(initial["include"], json!(["reasoning.encrypted_content"]));
    assert_eq!(initial["max_tool_calls"], 10);
    assert_eq!(initial["tools"][0]["name"], "read_resume");
    assert_eq!(
        initial["tools"][1],
        json!({"type": "web_search", "external_web_access": true})
    );

    let exchange = local_exchange(turn.unwrap());
    assert_eq!(exchange.continuation, hosted_items);
    let context = TurnContext {
        history: &[exchange],
        ..context
    };
    // Offsets refer to the raw JSON in each text block, including escapes and Unicode.
    let json_prefix = r#"{"reply":"Résumé \uD83D\uDE00: \"#;
    let cited_text = r#""dual licensing\" — MIT."#;
    let text = format!(
        r#"{cited_text} Personal contributions need separate evidence.","proposed_tex":null}}"#
    );
    let citation = json!({
        "type": "url_citation",
        "url": README,
        "title": "Rust README",
        "start_index": 1,
        "end_index": cited_text.chars().count() - 1,
    });
    let response = json!({
        "status": "completed",
        "output": [{
            "type": "message",
            "role": "assistant",
            "content": [
                {"type": "output_text", "text": json_prefix, "annotations": []},
                {
                    "type": "output_text",
                    "text": text,
                    "annotations": [
                        citation,
                        citation,
                        {"type": "url_citation", "url": "https://127.0.0.1/private"},
                        {"type": "url_citation", "url": "https://user:secret@example.com/"},
                        {"type": "url_citation", "url": REPO, "start_index": 0, "end_index": 900}
                    ]
                }
            ]
        }]
    });
    let (turn, wire) = mock_turn_sequence(&profile, vec![ok(response)], &context);
    let input = request_body(&wire[0])["input"].as_array().unwrap().clone();

    assert_eq!(&input[1..4], hosted_items.as_array().unwrap());
    assert_eq!(input[4]["type"], "function_call_output");
    assert_eq!(input[4]["call_id"], "call_resume");
    assert!(
        input[4]["output"]
            .as_str()
            .unwrap()
            .contains("synthetic draft")
    );

    let reply = completed_reply(turn.unwrap());
    assert_eq!(
        reply.reply,
        format!(
            "Résumé 😀: \"dual licensing\" — MIT. [1](<{README}>) \
             Personal contributions need separate evidence.\n\n\
             Sources\n- [2](<{REPO}>) github.com"
        )
    );
    assert!(!reply.reply.contains("127.0.0.1"));
    assert!(!reply.reply.contains("secret"));
    assert!(reply.proposed_tex.is_none());
}

#[test]
fn claude_replays_two_pauses_and_mixed_web_and_local_results_before_returning_sources() {
    let profile = Profile::new(Provider::Anthropic);
    let search = json!({
        "type": "server_tool_use",
        "id": "srvtoolu_search",
        "name": "web_search",
        "input": {"query": "Rust GitHub README license"}
    });
    let search_result = json!({
        "type": "web_search_tool_result",
        "tool_use_id": "srvtoolu_search",
        "content": [
            {
                "type": "web_search_result",
                "url": README,
                "title": "Rust README",
                "encrypted_content": "opaque-readme-search-result",
                "page_age": null
            },
            {
                "type": "web_search_result",
                "url": REPO,
                "title": "Rust repository",
                "encrypted_content": "opaque-repo-search-result",
                "page_age": null
            }
        ]
    });
    let fetch = json!({
        "type": "server_tool_use",
        "id": "srvtoolu_fetch",
        "name": "web_fetch",
        "input": {"url": README}
    });
    let local_read = json!({
        "type": "tool_use",
        "id": "call_resume",
        "name": "read_resume",
        "input": {}
    });
    let responses = vec![
        ok(json!({"stop_reason": "pause_turn", "content": [search]})),
        ok(json!({"stop_reason": "pause_turn", "content": [search_result]})),
        ok(json!({"stop_reason": "tool_use", "content": [fetch, local_read]})),
    ];
    let tools = [read_resume_tool()];
    let context = TurnContext {
        prompt: PROMPT,
        tools: &tools,
        history: &[],
    };

    let (turn, wire) = mock_turn_sequence(&profile, responses, &context);
    let initial = request_body(&wire[0]);
    let first_resume = request_body(&wire[1]);
    let second_resume = request_body(&wire[2]);

    assert!(initial.get("output_config").is_none());
    assert_eq!(initial["tools"][0]["type"], "web_search_20250305");
    assert_eq!(initial["tools"][1]["type"], "web_fetch_20250910");
    assert_eq!(initial["tools"][1]["citations"]["enabled"], true);
    assert_eq!(
        initial["tools"][1]["url_sources"]["client_tool_results"]["type"],
        "none"
    );
    assert_eq!(initial["tools"], first_resume["tools"]);
    assert_eq!(initial["tools"], second_resume["tools"]);
    assert_eq!(first_resume["messages"][1]["content"], json!([search]));
    assert_eq!(
        second_resume["messages"][1]["content"],
        json!([search, search_result])
    );

    let exchange = local_exchange(turn.unwrap());
    assert_eq!(
        exchange.continuation,
        json!([search, search_result, fetch, local_read])
    );
    let context = TurnContext {
        history: &[exchange],
        ..context
    };
    let response = json!({
        "stop_reason": "end_turn",
        "content": [
            {
                "type": "web_fetch_tool_result",
                "tool_use_id": "srvtoolu_fetch",
                "content": {
                    "type": "web_fetch_result",
                    "url": README,
                    "content": {
                        "type": "document",
                        "title": "Rust README",
                        "source": {"type": "text", "media_type": "text/plain", "data": "MIT and Apache-2.0"},
                        "citations": {"enabled": true}
                    },
                    "retrieved_at": "2026-11-21T12:00:00Z"
                }
            },
            {
                "type": "text",
                "text": "The README documents dual licensing.",
                "citations": [
                    {
                        "type": "char_location",
                        "document_index": 0,
                        "document_title": "Rust README",
                        "start_char_index": 0,
                        "end_char_index": 18,
                        "cited_text": "MIT and Apache-2.0"
                    },
                    {
                        "type": "web_search_result_location",
                        "url": REPO,
                        "title": "Rust repository",
                        "encrypted_index": "opaque-index",
                        "cited_text": "Licenses"
                    }
                ]
            },
            {"type": "text", "text": " Contributions need separate evidence."}
        ]
    });

    let (turn, wire) = mock_turn_sequence(&profile, vec![ok(response)], &context);
    let continued = request_body(&wire[0]);

    assert_eq!(continued["tools"], initial["tools"]);
    assert_eq!(
        continued["messages"][1]["content"],
        json!([search, search_result, fetch, local_read])
    );
    assert_eq!(continued["messages"][2]["role"], "user");
    assert_eq!(
        continued["messages"][2]["content"][0]["type"],
        "tool_result"
    );
    assert_eq!(
        continued["messages"][2]["content"][0]["tool_use_id"],
        "call_resume"
    );

    let reply = completed_reply(turn.unwrap());
    assert_eq!(
        reply.reply,
        format!(
            "The README documents dual licensing. [1](<{README}>) [2](<{REPO}>) \
             Contributions need separate evidence."
        )
    );
    assert!(reply.proposed_tex.is_none());
}

#[test]
fn claude_resolves_fetched_document_indexes_without_counting_paused_blocks_twice() {
    let profile = Profile::new(Provider::Anthropic);
    let context = TurnContext {
        prompt: PROMPT,
        tools: &[],
        history: &[],
    };
    let responses = vec![
        ok(json!({
            "stop_reason": "pause_turn",
            "content": [fetched_document(README, "Rust README", "srvtoolu_readme")]
        })),
        ok(json!({
            "stop_reason": "end_turn",
            "content": [
                fetched_document(REPO, "Rust project", "srvtoolu_repo"),
                {
                    "type": "text",
                    "text": "The project is public.",
                    "citations": [{
                        "type": "char_location",
                        "document_index": 1,
                        "document_title": "Rust project",
                        "start_char_index": 0,
                        "end_char_index": 10,
                        "cited_text": "A language"
                    }]
                }
            ]
        })),
    ];

    let (turn, _) = mock_turn_sequence(&profile, responses, &context);
    let reply = completed_reply(turn.unwrap());

    assert_eq!(
        reply.reply,
        format!("The project is public. [1](<{REPO}>)\n\nSources\n- [2](<{README}>) github.com")
    );
}

fn fetched_document(url: &str, title: &str, tool_use_id: &str) -> Value {
    json!({
        "type": "web_fetch_tool_result",
        "tool_use_id": tool_use_id,
        "content": {
            "type": "web_fetch_result",
            "url": url,
            "content": {
                "type": "document",
                "title": title,
                "source": {"type": "text", "media_type": "text/plain", "data": "A language"},
                "citations": {"enabled": true}
            },
            "retrieved_at": "2026-11-21T12:00:00Z"
        }
    })
}

fn ok(response: Value) -> (String, Value) {
    ("200 OK".into(), response)
}

fn read_resume_tool() -> ToolDefinition {
    ToolDefinition {
        name: "read_resume".into(),
        description: "Read the draft in memory.".into(),
        parameters: json!({
            "type": "object",
            "properties": {},
            "required": [],
            "additionalProperties": false
        }),
    }
}

fn request_body(wire: &str) -> Value {
    let (_, body) = wire.split_once("\r\n\r\n").unwrap();
    serde_json::from_str(body).unwrap()
}

fn local_exchange(turn: AgentTurn) -> ToolExchange {
    let AgentTurn::Tools {
        calls,
        continuation,
    } = turn
    else {
        panic!("expected a client tool call");
    };
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "read_resume");

    ToolExchange {
        results: vec![ToolResult {
            id: calls[0].id.clone(),
            is_error: false,
            output: json!({"tex": "synthetic draft"}),
        }],
        calls,
        continuation,
    }
}

fn completed_reply(turn: AgentTurn) -> AssistantReply {
    let AgentTurn::Complete(reply) = turn else {
        panic!("expected a final reply");
    };

    reply
}
