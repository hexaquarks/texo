use super::*;
use std::{
    io::{BufRead, BufReader, ErrorKind, Read, Write},
    net::{TcpListener, TcpStream},
    thread,
    time::Instant,
};

const MOCK_REQUEST_TIMEOUT: Duration = Duration::from_secs(3);
const MOCK_ACCEPT_POLL_INTERVAL: Duration = Duration::from_millis(5);

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
    let (result, mut wire) = mock_turn_sequence(
        &Profile::new(provider),
        vec![(status.to_owned(), body)],
        context,
    );

    (result, wire.pop().unwrap())
}

pub(super) fn mock_turn_sequence(
    profile: &Profile,
    replies: Vec<(String, Value)>,
    context: &TurnContext<'_>,
) -> (Result<AgentTurn>, Vec<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}/test", listener.local_addr().unwrap());
    listener.set_nonblocking(true).unwrap();

    let server = thread::spawn(move || {
        replies
            .into_iter()
            .map(|(status, reply)| serve_request(&listener, &status, &reply))
            .collect()
    });

    let result = request_turn_at(
        profile,
        "test-api-secret".into(),
        context,
        &AtomicBool::new(false),
        &endpoint,
        MOCK_REQUEST_TIMEOUT,
    );

    (result, server.join().unwrap())
}

fn serve_request(listener: &TcpListener, status: &str, reply: &Value) -> String {
    let mut stream = accept_before_timeout(listener);
    stream.set_nonblocking(false).unwrap();
    stream.set_read_timeout(Some(MOCK_REQUEST_TIMEOUT)).unwrap();

    let request = read_request(&mut stream);
    let body = reply.to_string();
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .unwrap();

    request
}

fn accept_before_timeout(listener: &TcpListener) -> TcpStream {
    // A broken continuation should fail the test promptly.
    let start = Instant::now();

    loop {
        match listener.accept() {
            Ok((stream, _)) => return stream,
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                assert!(
                    start.elapsed() < MOCK_REQUEST_TIMEOUT,
                    "The mock provider received no request"
                );
                thread::sleep(MOCK_ACCEPT_POLL_INTERVAL);
            }
            Err(error) => panic!("Could not accept request: {error}"),
        }
    }
}

fn read_request(stream: &mut TcpStream) -> String {
    let mut reader = BufReader::new(stream);
    let mut header = String::new();

    loop {
        let mut line = String::new();
        assert!(reader.read_line(&mut line).unwrap() > 0);
        header.push_str(&line);

        if line == "\r\n" {
            break;
        }
    }

    let lower_header = header.to_lowercase();
    let length = lower_header
        .lines()
        .find_map(|line| line.strip_prefix("content-length: "))
        .unwrap()
        .parse::<usize>()
        .unwrap();

    let mut body = vec![0; length];
    reader.read_exact(&mut body).unwrap();
    header.push_str(&String::from_utf8(body).unwrap());

    header
}
