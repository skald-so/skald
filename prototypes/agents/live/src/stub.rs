//! A local stand-in for the model: an HTTP server on the Anthropic Messages API (`/v1/messages`)
//! and the OpenAI Responses API (`/v1/responses`) that answers each request with a scripted turn and
//! records the requests. It refuses a request whose credential headers hold anything but
//! `PLACEHOLDER`, and records the header's name, never its value.

use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;

use agent_inbox::INBOX;
use serde_json::{Value, json};

use crate::SERVER;

/// The only credential the CLI may send to the stub.
pub const PLACEHOLDER: &str = "skald-live-check-placeholder";

/// The shell command of the busy session.
pub const SHELL: &str = "sleep 20";

/// A shell command that ends at once.
pub const QUICK: &str = "true";

/// The stub's reply once its script ends.
pub const DONE: &str = "Done.";

/// Parts of a header name that mark a credential header.
const CREDENTIAL: [&str; 5] = ["auth", "key", "token", "cookie", "secret"];

/// A tool call the stub makes in a scripted turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Call {
    /// Calls the `inbox` tool of the MCP server `SERVER`.
    Inbox,
    /// Runs `SHELL`.
    Shell,
    /// Runs `QUICK`.
    Quick,
}

impl Call {
    /// The shell command the call runs, if it runs one.
    fn command(self) -> Option<&'static str> {
        match self {
            Self::Inbox => None,
            Self::Shell => Some(SHELL),
            Self::Quick => Some(QUICK),
        }
    }
}

/// What the stub saw of a session.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct StubLog {
    /// The bodies of the requests the stub answered.
    pub requests: Vec<String>,
    /// The credential headers that held anything but the placeholder, each once.
    pub credentials: Vec<String>,
}

impl StubLog {
    /// Whether `token` reached a request the stub answered.
    pub fn saw(&self, token: &str) -> bool {
        self.requests.iter().any(|body| body.contains(token))
    }

    /// Why the session fails whatever the CLI did: a credential other than the placeholder.
    pub fn failure(&self) -> Option<String> {
        (!self.credentials.is_empty()).then(|| {
            format!(
                "the stub got a credential other than the placeholder in {}",
                self.credentials.join(", ")
            )
        })
    }

    /// Writes the requests to `path` as a JSON array: they hold the session's context; the stub
    /// keeps no credential's value.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        fs::write(path, serde_json::to_string_pretty(&self.requests)?)
    }
}

/// A running stub; it stops when dropped.
#[derive(Debug)]
pub struct Stub {
    addr: SocketAddr,
    log: Arc<Mutex<StubLog>>,
    stop: Arc<AtomicBool>,
}

impl Stub {
    /// Starts a stub on a free local port that answers with `script`'s calls, then with `DONE`.
    pub fn start(script: &'static [Call]) -> io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let addr = listener.local_addr()?;
        let log = Arc::new(Mutex::new(StubLog::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let (accepted_log, accepted_stop) = (Arc::clone(&log), Arc::clone(&stop));
        thread::spawn(move || {
            for stream in listener.incoming() {
                if accepted_stop.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(stream) = stream else { continue };
                let log = Arc::clone(&accepted_log);
                thread::spawn(move || serve(stream, script, &log));
            }
        });
        Ok(Self { addr, log, stop })
    }

    /// The base URL of the stub.
    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// What the stub saw so far.
    pub fn log(&self) -> StubLog {
        lock(&self.log).clone()
    }
}

impl Drop for Stub {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wakes the listener so it sees the stop.
        let _ = TcpStream::connect(self.addr);
    }
}

fn lock(log: &Mutex<StubLog>) -> MutexGuard<'_, StubLog> {
    log.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Answers the requests of one connection until the client closes it.
fn serve(stream: TcpStream, script: &[Call], log: &Mutex<StubLog>) -> io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut writer = stream;
    while let Some(request) = read_request(&mut reader)? {
        let response = answer(script, &request, &mut lock(log));
        writer.write_all(&response.to_bytes())?;
    }
    Ok(())
}

/// An HTTP request.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Request {
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// Reads one HTTP/1.1 request with a `Content-Length` body; `None` once the client closed the
/// connection.
pub fn read_request(reader: &mut impl BufRead) -> io::Result<Option<Request>> {
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    let path = line
        .split_whitespace()
        .nth(1)
        .unwrap_or_default()
        .to_owned();
    let mut headers = Vec::new();
    loop {
        line.clear();
        reader.read_line(&mut line)?;
        let Some((name, value)) = line.split_once(':') else {
            break;
        };
        headers.push((name.trim().to_owned(), value.trim().to_owned()));
    }
    let length = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    Ok(Some(Request {
        path,
        headers,
        body,
    }))
}

/// An HTTP response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub content_type: &'static str,
    pub body: String,
}

impl Response {
    fn json(status: u16, body: &Value) -> Self {
        Self {
            status,
            content_type: "application/json",
            body: body.to_string(),
        }
    }

    /// Server-sent events, each `(type, data)`.
    fn events(events: &[(&str, Value)]) -> Self {
        let body = events
            .iter()
            .map(|(event, data)| format!("event: {event}\ndata: {data}\n\n"))
            .collect();
        Self {
            status: 200,
            content_type: "text/event-stream",
            body,
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        format!(
            "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\n\r\n{}",
            self.status,
            self.content_type,
            self.body.len(),
            self.body
        )
        .into_bytes()
    }
}

/// The names of the credential headers in `headers` that hold anything but the placeholder.
pub fn foreign_credentials(headers: &[(String, String)]) -> Vec<String> {
    headers
        .iter()
        .map(|(name, value)| (name.to_ascii_lowercase(), value))
        .filter(|(name, value)| {
            CREDENTIAL.iter().any(|part| name.contains(part))
                && *value != PLACEHOLDER
                && **value != format!("Bearer {PLACEHOLDER}")
        })
        .map(|(name, _)| name)
        .collect()
}

/// Answers `request` with the next turn of `script` and records it in `log`.
pub fn answer(script: &[Call], request: &Request, log: &mut StubLog) -> Response {
    let foreign = foreign_credentials(&request.headers);
    if !foreign.is_empty() {
        for name in foreign {
            if !log.credentials.contains(&name) {
                log.credentials.push(name);
            }
        }
        return Response::json(401, &error("a credential other than the placeholder"));
    }
    log.requests
        .push(String::from_utf8_lossy(&request.body).into_owned());
    let body: Value = serde_json::from_slice(&request.body).unwrap_or_default();
    match request.path.split('?').next().unwrap_or_default() {
        "/v1/messages" => messages(script, &body),
        "/v1/messages/count_tokens" => Response::json(200, &json!({ "input_tokens": 0 })),
        "/v1/responses" => responses(script, &body),
        _ => Response::json(404, &error("not found")),
    }
}

fn error(message: &str) -> Value {
    json!({ "type": "error", "error": { "type": "stub_error", "message": message } })
}

/// How many tool results the conversation in `items` holds: the turns of the script it has played.
fn played(items: &Value, results: impl Fn(&Value) -> usize) -> usize {
    items
        .as_array()
        .map_or(0, |items| items.iter().map(results).sum())
}

/// An Anthropic Messages API answer.
fn messages(script: &[Call], body: &Value) -> Response {
    let step = played(&body["messages"], |message| {
        message["content"].as_array().map_or(0, |content| {
            content
                .iter()
                .filter(|block| block["type"] == "tool_result")
                .count()
        })
    });
    let (block, delta, stop_reason) = match script.get(step) {
        Some(call) => {
            let (name, input) = match call.command() {
                None => (format!("mcp__{SERVER}__{INBOX}"), json!({})),
                Some(command) => ("Bash".to_owned(), json!({ "command": command })),
            };
            let id = format!("toolu_stub_{step}");
            (
                json!({ "type": "tool_use", "id": id, "name": name, "input": input }),
                json!({ "type": "input_json_delta", "partial_json": input.to_string() }),
                "tool_use",
            )
        }
        None => (
            json!({ "type": "text", "text": DONE }),
            json!({ "type": "text_delta", "text": DONE }),
            "end_turn",
        ),
    };
    let message = |content: Value, stop_reason: Value| {
        json!({
            "id": format!("msg_stub_{step}"),
            "type": "message",
            "role": "assistant",
            "model": body["model"],
            "content": content,
            "stop_reason": stop_reason,
            "stop_sequence": null,
            "usage": { "input_tokens": 0, "output_tokens": 0 },
        })
    };
    if body["stream"] != true {
        return Response::json(200, &message(json!([block]), json!(stop_reason)));
    }
    let mut start = block;
    match start["type"].as_str() {
        Some("tool_use") => start["input"] = json!({}),
        _ => start["text"] = json!(""),
    }
    Response::events(&[
        (
            "message_start",
            json!({ "type": "message_start", "message": message(json!([]), Value::Null) }),
        ),
        (
            "content_block_start",
            json!({ "type": "content_block_start", "index": 0, "content_block": start }),
        ),
        (
            "content_block_delta",
            json!({ "type": "content_block_delta", "index": 0, "delta": delta }),
        ),
        (
            "content_block_stop",
            json!({ "type": "content_block_stop", "index": 0 }),
        ),
        (
            "message_delta",
            json!({
                "type": "message_delta",
                "delta": { "stop_reason": stop_reason, "stop_sequence": null },
                "usage": { "output_tokens": 0 },
            }),
        ),
        ("message_stop", json!({ "type": "message_stop" })),
    ])
}

/// An OpenAI Responses API answer, always streamed.
fn responses(script: &[Call], body: &Value) -> Response {
    let step = played(&body["input"], |item| {
        usize::from(
            item["type"]
                .as_str()
                .is_some_and(|kind| kind.ends_with("_output")),
        )
    });
    let item = match script.get(step) {
        Some(call) => {
            let call_id = format!("call_stub_{step}");
            let id = format!("fc_stub_{step}");
            match call.command() {
                None => json!({
                    "type": "function_call", "id": id, "call_id": call_id,
                    "namespace": format!("mcp__{SERVER}"), "name": INBOX, "arguments": "{}",
                }),
                // Codex waits for the command up to `yield_time_ms`, 10 s by default.
                Some(command) => json!({
                    "type": "function_call", "id": id, "call_id": call_id, "name": "exec_command",
                    "arguments": json!({ "cmd": command, "yield_time_ms": 30_000 }).to_string(),
                }),
            }
        }
        None => json!({
            "type": "message", "id": format!("msg_stub_{step}"), "role": "assistant",
            "content": [{ "type": "output_text", "text": DONE }],
        }),
    };
    let id = format!("resp_stub_{step}");
    let usage = json!({
        "input_tokens": 0, "input_tokens_details": null,
        "output_tokens": 0, "output_tokens_details": null, "total_tokens": 0,
    });
    Response::events(&[
        (
            "response.created",
            json!({ "type": "response.created", "response": { "id": id } }),
        ),
        (
            "response.output_item.done",
            json!({ "type": "response.output_item.done", "item": item }),
        ),
        (
            "response.completed",
            json!({ "type": "response.completed", "response": { "id": id, "usage": usage } }),
        ),
    ])
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use super::*;

    const BUSY: &[Call] = &[Call::Shell, Call::Inbox];

    fn request(path: &str, headers: &[(&str, &str)], body: &Value) -> Request {
        Request {
            path: path.to_owned(),
            headers: headers
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect(),
            body: body.to_string().into_bytes(),
        }
    }

    /// The data of each server-sent event in `response`.
    fn events(response: &Response) -> Vec<Value> {
        assert_eq!(response.status, 200);
        assert_eq!(response.content_type, "text/event-stream");
        response
            .body
            .split("\n\n")
            .filter(|event| !event.is_empty())
            .map(|event| {
                let (kind, data) = event.split_once('\n').unwrap();
                let data: Value =
                    serde_json::from_str(data.strip_prefix("data: ").unwrap()).unwrap();
                assert_eq!(kind, format!("event: {}", data["type"].as_str().unwrap()));
                data
            })
            .collect()
    }

    /// An Anthropic request after `results` tool results.
    fn messages_request(results: usize, stream: bool) -> Request {
        let mut messages =
            vec![json!({ "role": "user", "content": [{ "type": "text", "text": "Call" }] })];
        for i in 0..results {
            messages
                .push(json!({ "role": "assistant", "content": [{ "type": "tool_use", "id": i }] }));
            messages.push(json!({ "role": "user", "content": [
                { "type": "tool_result", "tool_use_id": i, "content": "x" },
                { "type": "text", "text": "a reminder" },
            ] }));
        }
        request(
            "/v1/messages?beta=true",
            &[("x-api-key", PLACEHOLDER)],
            &json!({ "model": "m", "stream": stream, "messages": messages }),
        )
    }

    /// The content block a streamed Anthropic answer makes, with its stop reason.
    fn streamed_block(response: &Response) -> (Value, Value) {
        let events = events(response);
        let kinds: Vec<_> = events.iter().map(|event| event["type"].clone()).collect();
        assert_eq!(
            kinds,
            [
                "message_start",
                "content_block_start",
                "content_block_delta",
                "content_block_stop",
                "message_delta",
                "message_stop"
            ]
        );
        assert_eq!(events[0]["message"]["content"], json!([]));
        assert_eq!(events[0]["message"]["model"], "m");
        assert_eq!(events[0]["message"]["role"], "assistant");
        let mut block = events[1]["content_block"].clone();
        let delta = &events[2]["delta"];
        match delta["type"].as_str().unwrap() {
            "input_json_delta" => {
                assert_eq!(block["input"], json!({}));
                block["input"] =
                    serde_json::from_str(delta["partial_json"].as_str().unwrap()).unwrap();
            }
            "text_delta" => {
                assert_eq!(block["text"], "");
                block["text"] = delta["text"].clone();
            }
            other => panic!("delta {other}"),
        }
        (block, events[4]["delta"]["stop_reason"].clone())
    }

    #[test]
    fn plays_the_script_on_the_messages_api() {
        let mut log = StubLog::default();
        let (block, stop) = streamed_block(&answer(BUSY, &messages_request(0, true), &mut log));
        assert_eq!(
            block,
            json!({ "type": "tool_use", "id": "toolu_stub_0", "name": "Bash", "input": { "command": "sleep 20" } })
        );
        assert_eq!(stop, "tool_use");
        let (block, stop) = streamed_block(&answer(BUSY, &messages_request(1, true), &mut log));
        assert_eq!(
            block,
            json!({ "type": "tool_use", "id": "toolu_stub_1", "name": "mcp__skald__inbox", "input": {} })
        );
        assert_eq!(stop, "tool_use");
        let (block, stop) = streamed_block(&answer(BUSY, &messages_request(2, true), &mut log));
        assert_eq!(block, json!({ "type": "text", "text": "Done." }));
        assert_eq!(stop, "end_turn");
        assert_eq!(log.requests.len(), 3);
        assert!(log.credentials.is_empty());
    }

    #[test]
    fn answers_unstreamed_messages() {
        let response = answer(
            &[Call::Inbox],
            &messages_request(0, false),
            &mut StubLog::default(),
        );
        assert_eq!(response.status, 200);
        assert_eq!(response.content_type, "application/json");
        let message: Value = serde_json::from_str(&response.body).unwrap();
        assert_eq!(
            message["content"],
            json!([{ "type": "tool_use", "id": "toolu_stub_0", "name": "mcp__skald__inbox", "input": {} }])
        );
        assert_eq!(message["stop_reason"], "tool_use");
        assert_eq!(message["model"], "m");
        let response = answer(
            &[Call::Inbox],
            &messages_request(1, false),
            &mut StubLog::default(),
        );
        let message: Value = serde_json::from_str(&response.body).unwrap();
        assert_eq!(
            message["content"],
            json!([{ "type": "text", "text": "Done." }])
        );
        assert_eq!(message["stop_reason"], "end_turn");
    }

    /// A Responses request after `results` tool outputs.
    fn responses_request(results: usize) -> Request {
        let mut input = vec![json!({ "type": "message", "role": "user", "content": [] })];
        for i in 0..results {
            input.push(json!({ "type": "function_call", "call_id": i }));
            input.push(json!({ "type": "function_call_output", "call_id": i, "output": "x" }));
        }
        request(
            "/v1/responses",
            &[("authorization", &format!("Bearer {PLACEHOLDER}"))],
            &json!({ "model": "m", "stream": true, "input": input }),
        )
    }

    /// The item a Responses answer makes.
    fn responded_item(response: &Response) -> Value {
        let events = events(response);
        let kinds: Vec<_> = events.iter().map(|event| event["type"].clone()).collect();
        assert_eq!(
            kinds,
            [
                "response.created",
                "response.output_item.done",
                "response.completed"
            ]
        );
        assert_eq!(events[0]["response"]["id"], events[2]["response"]["id"]);
        assert_eq!(events[2]["response"]["usage"]["total_tokens"], 0);
        events[1]["item"].clone()
    }

    #[test]
    fn plays_the_script_on_the_responses_api() {
        let mut log = StubLog::default();
        assert_eq!(
            responded_item(&answer(BUSY, &responses_request(0), &mut log)),
            json!({
                "type": "function_call", "id": "fc_stub_0", "call_id": "call_stub_0",
                "name": "exec_command", "arguments": r#"{"cmd":"sleep 20","yield_time_ms":30000}"#,
            })
        );
        assert_eq!(
            responded_item(&answer(BUSY, &responses_request(1), &mut log)),
            json!({
                "type": "function_call", "id": "fc_stub_1", "call_id": "call_stub_1",
                "namespace": "mcp__skald", "name": "inbox", "arguments": "{}",
            })
        );
        assert_eq!(
            responded_item(&answer(BUSY, &responses_request(2), &mut log)),
            json!({
                "type": "message", "id": "msg_stub_2", "role": "assistant",
                "content": [{ "type": "output_text", "text": "Done." }],
            })
        );
        assert_eq!(log.requests.len(), 3);
    }

    #[test]
    fn runs_the_quick_command_on_both_apis() {
        let mut log = StubLog::default();
        let (block, stop) = streamed_block(&answer(
            &[Call::Quick],
            &messages_request(0, true),
            &mut log,
        ));
        assert_eq!(
            block,
            json!({ "type": "tool_use", "id": "toolu_stub_0", "name": "Bash", "input": { "command": "true" } })
        );
        assert_eq!(stop, "tool_use");
        let (block, _) = streamed_block(&answer(
            &[Call::Quick],
            &messages_request(1, true),
            &mut log,
        ));
        assert_eq!(block, json!({ "type": "text", "text": "Done." }));
        assert_eq!(
            responded_item(&answer(&[Call::Quick], &responses_request(0), &mut log)),
            json!({
                "type": "function_call", "id": "fc_stub_0", "call_id": "call_stub_0",
                "name": "exec_command", "arguments": r#"{"cmd":"true","yield_time_ms":30000}"#,
            })
        );
    }

    #[test]
    fn answers_other_paths() {
        let mut log = StubLog::default();
        let tokens = answer(
            &[],
            &request("/v1/messages/count_tokens", &[], &json!({})),
            &mut log,
        );
        assert_eq!(tokens, Response::json(200, &json!({ "input_tokens": 0 })));
        let other = answer(&[], &request("/v1/models", &[], &json!({})), &mut log);
        assert_eq!(
            other,
            Response::json(
                404,
                &json!({ "type": "error", "error": { "type": "stub_error", "message": "not found" } })
            )
        );
        assert_eq!(log.requests.len(), 2);
    }

    #[test]
    fn finds_credentials_other_than_the_placeholder() {
        let headers = |pairs: &[(&str, &str)]| -> Vec<(String, String)> {
            pairs
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect()
        };
        assert!(
            foreign_credentials(&headers(&[
                ("X-Api-Key", PLACEHOLDER),
                ("Authorization", &format!("Bearer {PLACEHOLDER}")),
                ("Content-Type", "application/json"),
                ("anthropic-version", "2023-06-01"),
            ]))
            .is_empty()
        );
        assert_eq!(
            foreign_credentials(&headers(&[
                ("X-Api-Key", "sk-ant-secret"),
                ("Authorization", "Bearer sk-secret"),
                ("Proxy-Authorization", PLACEHOLDER),
                ("Cookie", "session=secret"),
                ("x-goog-api-key", ""),
                ("x-auth-token", "Bearer secret"),
                ("x-client-secret", "secret"),
                ("user-agent", "secret"),
            ])),
            [
                "x-api-key",
                "authorization",
                "cookie",
                "x-goog-api-key",
                "x-auth-token",
                "x-client-secret"
            ]
        );
    }

    #[test]
    fn refuses_a_request_with_a_foreign_credential_without_keeping_its_value() {
        let mut log = StubLog::default();
        let secret = "sk-ant-oat01-secret";
        let response = answer(
            &[Call::Inbox],
            &request(
                "/v1/messages",
                &[
                    ("Authorization", &format!("Bearer {secret}")),
                    ("x-api-key", secret),
                ],
                &json!({ "stream": true, "messages": [] }),
            ),
            &mut log,
        );
        assert_eq!(
            response,
            Response::json(
                401,
                &json!({ "type": "error", "error": {
                    "type": "stub_error", "message": "a credential other than the placeholder",
                } })
            )
        );
        // A second request with the same header adds no name.
        answer(
            &[Call::Inbox],
            &request("/v1/responses", &[("authorization", secret)], &json!({})),
            &mut log,
        );
        assert_eq!(
            log,
            StubLog {
                requests: Vec::new(),
                credentials: vec!["authorization".to_owned(), "x-api-key".to_owned()],
            }
        );
        let failure = log.failure().unwrap();
        assert_eq!(
            failure,
            "the stub got a credential other than the placeholder in authorization, x-api-key"
        );
        assert!(!format!("{log:?}").contains(secret));
        assert_eq!(StubLog::default().failure(), None);
    }

    #[test]
    fn sees_a_token_in_a_request() {
        let log = StubLog {
            requests: vec![
                "{}".to_owned(),
                r#"{"text":"The check token is tok-1."}"#.to_owned(),
            ],
            credentials: Vec::new(),
        };
        assert!(log.saw("tok-1"));
        assert!(!log.saw("tok-2"));
        assert!(!StubLog::default().saw("tok-1"));
    }

    #[test]
    fn saves_the_requests_as_a_json_array() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("stub.json");
        let log = StubLog {
            requests: vec!["{}".to_owned(), "a\"b".to_owned()],
            credentials: vec!["x-api-key".to_owned()],
        };
        log.save(&path).unwrap();
        let saved: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(saved, json!(["{}", "a\"b"]));
        assert!(log.save(&dir.path().join("missing/stub.json")).is_err());
    }

    #[test]
    fn reads_requests_with_their_bodies() {
        let raw = "POST /v1/messages?beta=true HTTP/1.1\r\nHost: x\r\ncontent-length: 2\r\nX-Api-Key: k\r\n\r\n{}\
                   GET /v1/models HTTP/1.1\r\nCONTENT-LENGTH: 3\r\n\r\nabc";
        let mut reader = BufReader::new(raw.as_bytes());
        assert_eq!(
            read_request(&mut reader).unwrap(),
            Some(Request {
                path: "/v1/messages?beta=true".to_owned(),
                headers: vec![
                    ("Host".to_owned(), "x".to_owned()),
                    ("content-length".to_owned(), "2".to_owned()),
                    ("X-Api-Key".to_owned(), "k".to_owned()),
                ],
                body: b"{}".to_vec(),
            })
        );
        let second = read_request(&mut reader).unwrap().unwrap();
        assert_eq!(second.path, "/v1/models");
        assert_eq!(second.body, b"abc");
        assert_eq!(read_request(&mut reader).unwrap(), None);
        // Without a length, no body.
        let mut reader = BufReader::new("GET / HTTP/1.1\r\n\r\n".as_bytes());
        assert_eq!(read_request(&mut reader).unwrap().unwrap().body, b"");
    }

    #[test]
    fn writes_responses() {
        let response = Response {
            status: 401,
            content_type: "application/json",
            body: "{}".to_owned(),
        };
        assert_eq!(
            response.to_bytes(),
            b"HTTP/1.1 401\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n{}"
        );
    }

    /// Sends `request` to `stub` on one connection, twice, and returns the responses' heads.
    fn exchange(stub: &Stub, request: &str) -> String {
        let url = stub.url();
        let stream = TcpStream::connect(url.strip_prefix("http://").unwrap()).unwrap();
        send(stream, &request.repeat(2)).unwrap()
    }

    /// Sends `requests` on `stream`, closes its write side and reads the answers, at most 1 MiB.
    fn send(mut stream: TcpStream, requests: &str) -> io::Result<String> {
        stream.set_read_timeout(Some(std::time::Duration::from_secs(5)))?;
        stream.write_all(requests.as_bytes())?;
        stream.shutdown(std::net::Shutdown::Write)?;
        let mut response = String::new();
        stream.take(1 << 20).read_to_string(&mut response)?;
        Ok(response)
    }

    #[test]
    fn serves_requests_over_http_until_dropped() {
        let stub = Stub::start(&[Call::Inbox]).unwrap();
        assert!(stub.url().starts_with("http://127.0.0.1:"));
        let body = r#"{"messages":[],"note":"tok-9"}"#;
        let request = format!(
            "POST /v1/messages HTTP/1.1\r\nx-api-key: {PLACEHOLDER}\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        let response = exchange(&stub, &request);
        assert_eq!(
            response.matches("HTTP/1.1 200\r\n").count(),
            2,
            "{response}"
        );
        assert!(response.contains("mcp__skald__inbox"), "{response}");
        let request =
            "POST /v1/messages HTTP/1.1\r\nx-api-key: secret\r\nContent-Length: 0\r\n\r\n";
        assert!(exchange(&stub, request).starts_with("HTTP/1.1 401\r\n"));
        assert_eq!(
            stub.log(),
            StubLog {
                requests: vec![body.to_owned(), body.to_owned()],
                credentials: vec!["x-api-key".to_owned()],
            }
        );
        let url = stub.url();
        drop(stub);
        // The listener stops: a connection is refused or closed without an answer.
        let addr = url.strip_prefix("http://").unwrap();
        thread::sleep(std::time::Duration::from_millis(100));
        if let Ok(stream) = TcpStream::connect(addr) {
            assert_eq!(send(stream, request).unwrap_or_default(), "");
        }
    }
}
