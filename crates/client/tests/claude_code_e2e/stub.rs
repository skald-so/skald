//! The model stub (`docs/dev/agent-delivery.md`, "Model stub"): a local HTTP server on the
//! Anthropic Messages API that plays a script of tool calls and records what the session sent.
//!
//! - A request of the session's turn, one that offers tools, gets the script's next step: the step
//!   whose index is the number of tool results the conversation holds; past the script, the reply
//!   `Done.`. Any other request, such as one for a title, gets `Done.`.
//! - The stub can hold the answers of one step until the test releases them, so the test acts while
//!   the turn is running.
//! - It answers a request whose credential headers hold anything but [`PLACEHOLDER`] with an error
//!   and records the header's name, never its value: [`Stub::foreign`] fails the test.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

/// The only credential the session may send.
pub const PLACEHOLDER: &str = "skald-e2e-placeholder-key";

/// What the stub says once its script ends.
pub const DONE: &str = "Done.";

/// The name Claude Code gives a tool of the MCP server `skald`.
pub fn skald_tool(tool: &str) -> String {
    format!("mcp__skald__{tool}")
}

/// Parts of a header's name that make it a credential header.
const CREDENTIALS: [&str; 5] = ["auth", "key", "token", "cookie", "secret"];

/// How long a test waits for the session to reach a step.
const REACH: Duration = Duration::from_secs(120);

/// A step of the stub's script: a tool call.
#[derive(Debug, Clone)]
pub enum Step {
    /// Runs a shell command with `Bash`.
    Bash(&'static str),
    /// Calls a tool of the MCP server `skald` with its arguments.
    Skald(&'static str, Value),
}

impl Step {
    /// The tool's name and input.
    fn call(&self) -> (String, Value) {
        match self {
            Self::Bash(command) => ("Bash".to_owned(), json!({ "command": command })),
            Self::Skald(tool, arguments) => (skald_tool(tool), arguments.clone()),
        }
    }
}

/// What the stub saw.
#[derive(Default)]
struct Seen {
    /// The bodies of the requests of the session's turns, in order.
    turns: Vec<Value>,
    /// The names of the credential headers that held anything but the placeholder.
    foreign: Vec<String>,
    /// Whether the held step's answers go out.
    released: bool,
}

struct Shared {
    script: Vec<Step>,
    /// The step whose answers wait for [`Stub::release`], if any.
    hold: Option<usize>,
    seen: Mutex<Seen>,
    changed: Condvar,
}

impl Shared {
    fn seen(&self) -> MutexGuard<'_, Seen> {
        self.seen
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// A running stub; it stops taking connections when dropped.
pub struct Stub {
    url: String,
    shared: Arc<Shared>,
}

impl Stub {
    /// Starts a stub on a free local port that plays `script`, holding the answers of step `hold`,
    /// if any, until released.
    pub fn start(script: Vec<Step>, hold: Option<usize>) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind the model stub");
        let url = format!(
            "http://{}",
            listener.local_addr().expect("the stub's address")
        );
        let shared = Arc::new(Shared {
            script,
            hold,
            seen: Mutex::new(Seen::default()),
            changed: Condvar::new(),
        });
        let serving = Arc::downgrade(&shared);
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let Some(shared) = serving.upgrade() else {
                    break;
                };
                thread::spawn(move || serve(stream, &shared));
            }
        });
        Stub { url, shared }
    }

    /// The base URL Claude Code takes as `ANTHROPIC_BASE_URL`.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Waits until a request of the turn reached the held step; fails after a while.
    #[track_caller]
    pub fn wait_held(&self) {
        let hold = self.shared.hold.expect("the stub holds a step");
        let deadline = Instant::now() + REACH;
        let mut seen = self.shared.seen();
        while !seen.turns.iter().any(|body| played(body) == hold) {
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(!left.is_zero(), "the session did not reach step {hold}");
            seen = (self.shared.changed.wait_timeout(seen, left))
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .0;
        }
    }

    /// Lets the held step's answers go out.
    pub fn release(&self) {
        self.shared.seen().released = true;
        self.shared.changed.notify_all();
    }

    /// The bodies of the requests of the session's turns so far, in order.
    pub fn turns(&self) -> Vec<Value> {
        self.shared.seen().turns.clone()
    }

    /// Fails if the session sent a credential other than the placeholder, naming the header, never
    /// its value.
    #[track_caller]
    pub fn assert_no_foreign_credential(&self) {
        let foreign = self.shared.seen().foreign.clone();
        assert!(
            foreign.is_empty(),
            "the stub got a credential other than its placeholder in {foreign:?}"
        );
    }
}

impl Drop for Stub {
    fn drop(&mut self) {
        self.release();
        // Wakes the listener, which then sees the stub is gone.
        let _ = TcpStream::connect(self.url.trim_start_matches("http://"));
    }
}

/// How many tool results the conversation in `body` holds: the steps the session has played.
pub fn played(body: &Value) -> usize {
    let messages = body["messages"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    (messages.iter())
        .filter_map(|message| message["content"].as_array())
        .flatten()
        .filter(|block| block["type"] == "tool_result")
        .count()
}

/// The ID of the stub's tool call of step `step`.
pub fn call_id(step: usize) -> String {
    format!("toolu_e2e_{step}")
}

/// Answers the requests of one connection until the client closes it.
fn serve(stream: TcpStream, shared: &Shared) {
    let Ok(mut writer) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(stream);
    while let Some((path, headers, body)) = request(&mut reader) {
        let (status, kind, answer) = answer(shared, &path, &headers, &body);
        let head = format!(
            "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\n\r\n",
            answer.len()
        );
        if writer.write_all(head.as_bytes()).is_err()
            || writer.write_all(answer.as_bytes()).is_err()
        {
            return;
        }
    }
}

/// An HTTP request's path, headers, their names lowercased, and body.
type Request = (String, Vec<(String, String)>, Vec<u8>);

/// One HTTP/1.1 request with a `Content-Length` body; `None` once the client closed the connection.
fn request(reader: &mut impl BufRead) -> Option<Request> {
    let mut line = String::new();
    if reader.read_line(&mut line).ok()? == 0 {
        return None;
    }
    let path = line.split(' ').nth(1).unwrap_or_default().to_owned();
    let mut headers = Vec::new();
    loop {
        let mut header = String::new();
        reader.read_line(&mut header).ok()?;
        match header.trim_end().split_once(':') {
            Some((name, value)) => {
                headers.push((name.to_ascii_lowercase(), value.trim().to_owned()))
            }
            None => break,
        }
    }
    let length: usize = (headers.iter())
        .find(|(name, _)| name == "content-length")
        .and_then(|(_, value)| value.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0; length];
    reader.read_exact(&mut body).ok()?;
    Some((path, headers, body))
}

/// The status, content type and body that answer a request.
fn answer(
    shared: &Shared,
    path: &str,
    headers: &[(String, String)],
    body: &[u8],
) -> (&'static str, &'static str, String) {
    let foreign: Vec<String> = (headers.iter())
        .filter(|(name, _)| CREDENTIALS.iter().any(|part| name.contains(part)))
        .filter(|(_, value)| value != PLACEHOLDER && *value != format!("Bearer {PLACEHOLDER}"))
        .map(|(name, _)| name.clone())
        .collect();
    if !foreign.is_empty() {
        shared.seen().foreign.extend(foreign);
        let error = json!({ "type": "error", "error": { "type": "authentication_error", "message": "not the placeholder" } });
        return ("401 Unauthorized", "application/json", error.to_string());
    }
    let body: Value = serde_json::from_slice(body).unwrap_or_default();
    match path.split('?').next().unwrap_or_default() {
        "/v1/messages/count_tokens" => (
            "200 OK",
            "application/json",
            json!({ "input_tokens": 1 }).to_string(),
        ),
        "/v1/messages" => ("200 OK", "text/event-stream", turn(shared, body)),
        _ => (
            "404 Not Found",
            "application/json",
            json!({ "type": "error" }).to_string(),
        ),
    }
}

/// The streamed answer to a request of `/v1/messages`: the next step if it is one of the session's
/// turns, else `Done.`.
fn turn(shared: &Shared, body: Value) -> String {
    let offers_tools = body["tools"]
        .as_array()
        .is_some_and(|tools| !tools.is_empty());
    let step = played(&body);
    let model = body["model"].clone();
    if offers_tools {
        let mut seen = shared.seen();
        seen.turns.push(body);
        shared.changed.notify_all();
        while shared.hold == Some(step) && !seen.released {
            seen = (shared.changed.wait(seen)).unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }
    let call = offers_tools.then(|| shared.script.get(step)).flatten();
    let (block, delta, stop) = match call.map(Step::call) {
        Some((name, input)) => (
            json!({ "type": "tool_use", "id": call_id(step), "name": name, "input": {} }),
            json!({ "type": "input_json_delta", "partial_json": input.to_string() }),
            "tool_use",
        ),
        None => (
            json!({ "type": "text", "text": "" }),
            json!({ "type": "text_delta", "text": DONE }),
            "end_turn",
        ),
    };
    let usage = json!({ "input_tokens": 1, "output_tokens": 1 });
    let start = json!({
        "type": "message_start",
        "message": {
            "id": format!("msg_e2e_{step}"), "type": "message", "role": "assistant", "model": model,
            "content": [], "stop_reason": null, "stop_sequence": null, "usage": usage,
        },
    });
    let events = [
        start,
        json!({ "type": "content_block_start", "index": 0, "content_block": block }),
        json!({ "type": "content_block_delta", "index": 0, "delta": delta }),
        json!({ "type": "content_block_stop", "index": 0 }),
        json!({ "type": "message_delta", "delta": { "stop_reason": stop, "stop_sequence": null }, "usage": usage }),
        json!({ "type": "message_stop" }),
    ];
    (events.iter())
        .map(|event| {
            format!(
                "event: {}\ndata: {event}\n\n",
                event["type"].as_str().unwrap_or_default()
            )
        })
        .collect()
}

/// Sends `stub` a request of a turn with the header `name: value`; gives its status line.
fn status_of(stub: &Stub, (name, value): (&str, &str)) -> String {
    let body = json!({ "model": "stub", "messages": [] }).to_string();
    let mut stream = TcpStream::connect(stub.url().trim_start_matches("http://")).expect("connect");
    let head = format!(
        "POST /v1/messages HTTP/1.1\r\nHost: stub\r\n{name}: {value}\r\nContent-Length: {}\r\n\r\n",
        body.len()
    );
    (stream.write_all(head.as_bytes())).expect("send the head");
    (stream.write_all(body.as_bytes())).expect("send the body");
    let mut status = String::new();
    (BufReader::new(stream).read_line(&mut status)).expect("read the status");
    status
}

/// The stub answers a request with its placeholder as the credential, in `x-api-key` or as a bearer
/// token; one with another credential in either header gets 401, and the stub keeps the header's
/// name, never its value, so the test then fails.
#[test]
fn stub_refuses_a_credential_other_than_its_placeholder() {
    let stub = Stub::start(Vec::new(), None);
    let other = "sk-ant-not-the-placeholder";
    let bearer = |key: &str| format!("Bearer {key}");

    let placeholder = [
        status_of(&stub, ("x-api-key", PLACEHOLDER)),
        status_of(&stub, ("Authorization", &bearer(PLACEHOLDER))),
    ];
    let foreign = [
        status_of(&stub, ("x-api-key", other)),
        status_of(&stub, ("Authorization", &bearer(other))),
    ];

    assert!(
        placeholder.iter().all(|status| status.contains(" 200 ")),
        "{placeholder:?}"
    );
    assert!(
        foreign.iter().all(|status| status.contains(" 401 ")),
        "{foreign:?}"
    );
    let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        stub.assert_no_foreign_credential()
    }));
    let said = (failed
        .expect_err("a foreign credential passes")
        .downcast::<String>())
    .expect("the failure says why");
    assert!(
        said.contains("x-api-key") && said.contains("authorization") && !said.contains(other),
        "{said}"
    );
}
