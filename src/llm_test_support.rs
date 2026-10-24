use super::*;
use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
};

pub(super) fn mock_request(
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

pub(super) fn mock_turn(
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
        write!(
            stream,
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
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
