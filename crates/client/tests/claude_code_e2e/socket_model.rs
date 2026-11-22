//! A model stub for the sessions that get messages over their socket: a local server on the
//! Anthropic Messages API, as the model stub of `stub.rs`, that answers each turn by what the
//! conversation holds, not by a script's step, so a test can answer a turn that a message, not a
//! prompt, started.
//!
//! - A request that offers tools is a turn of the session, which counts as sent at once: the
//!   test's answer function picks its reply from its body, and may wait to. Any other request, such
//!   as one for a title, gets [`DONE`].
//! - It refuses a request whose credential headers hold anything but [`PLACEHOLDER`] and records
//!   the header's name, never its value: [`Model::assert_no_foreign_credential`] fails the test.

use std::net::TcpListener;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::Instant;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use serde_json::{Value, json};

use crate::stub::{DONE, PLACEHOLDER, skald_tool};

/// How long a test waits for the session to send a turn.
const REACH: std::time::Duration = std::time::Duration::from_secs(120);

/// How the stub answers a turn.
#[derive(Debug, Clone)]
pub enum Reply {
    /// The text [`DONE`], which ends the turn.
    Done,
    /// A call of `Bash` with this command.
    Bash(&'static str),
    /// A call of the tool of the MCP server `skald` with no arguments.
    Skald(&'static str),
    /// An error that Claude Code does not retry: `400 Bad Request`.
    Fail,
}

type Answer = Box<dyn Fn(&Value) -> Reply + Send + Sync>;

struct Shared {
    answer: Answer,
    /// The bodies of the session's turns, in order.
    turns: Mutex<Vec<Value>>,
    /// The names of the credential headers that held anything but the placeholder.
    foreign: Mutex<Vec<String>>,
    changed: Condvar,
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A running stub; it serves until the test binary exits.
pub struct Model {
    url: String,
    shared: Arc<Shared>,
}

impl Model {
    /// Starts a stub on a free local port that answers each turn as `answer` says.
    pub fn start(answer: impl Fn(&Value) -> Reply + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind the model stub");
        listener
            .set_nonblocking(true)
            .expect("a non-blocking listener");
        let url = format!("http://{}", listener.local_addr().expect("an address"));
        let shared = Arc::new(Shared {
            answer: Box::new(answer),
            turns: Mutex::new(Vec::new()),
            foreign: Mutex::new(Vec::new()),
            changed: Condvar::new(),
        });
        let serving = Arc::clone(&shared);
        thread::spawn(move || {
            let runtime = (tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build())
            .expect("a runtime for the model stub");
            runtime.block_on(async move {
                let listener =
                    tokio::net::TcpListener::from_std(listener).expect("a listener for tokio");
                let router = Router::new()
                    .route("/v1/messages", post(messages))
                    .route("/v1/messages/count_tokens", post(count_tokens))
                    .with_state(serving);
                let _ = axum::serve(listener, router).await;
            });
        });
        Model { url, shared }
    }

    /// The base URL Claude Code takes as `ANTHROPIC_BASE_URL`.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// The bodies of the session's turns so far.
    pub fn turns(&self) -> Vec<Value> {
        locked(&self.shared.turns).clone()
    }

    /// Waits until the session sent a turn whose conversation holds `text`; fails after a while,
    /// saying `what`. Gives the first such turn.
    #[track_caller]
    pub fn wait_turn_holding(&self, text: &str, what: &str) -> Value {
        self.wait_turn_showing(text, what, &String::new)
    }

    /// As [`Model::wait_turn_holding`]; the failure also shows what `shown` gives then.
    #[track_caller]
    pub fn wait_turn_showing(&self, text: &str, what: &str, shown: &dyn Fn() -> String) -> Value {
        let deadline = Instant::now() + REACH;
        let mut turns = locked(&self.shared.turns);
        loop {
            if let Some(turn) = turns.iter().find(|body| conversation(body).contains(text)) {
                return turn.clone();
            }
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(
                !left.is_zero(),
                "{what}: no turn held {text:?}\n{}",
                shown()
            );
            turns = (self.shared.changed.wait_timeout(turns, left))
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .0;
        }
    }

    /// Fails if the session sent a credential other than the placeholder, naming the header, never
    /// its value.
    #[track_caller]
    pub fn assert_no_foreign_credential(&self) {
        let foreign = locked(&self.shared.foreign).clone();
        assert!(
            foreign.is_empty(),
            "a credential other than the placeholder in {foreign:?}"
        );
    }
}

/// Every string of the conversation in a turn's `body`, a line apart.
pub fn conversation(body: &Value) -> String {
    fn walk(value: &Value, into: &mut String) {
        match value {
            Value::String(text) => {
                into.push_str(text);
                into.push('\n');
            }
            Value::Array(items) => items.iter().for_each(|item| walk(item, into)),
            Value::Object(fields) => fields.values().for_each(|item| walk(item, into)),
            _ => {}
        }
    }
    let mut all = String::new();
    walk(&body["messages"], &mut all);
    all
}

/// The text of the last user message of a turn's `body`, which started or goes on with the turn.
pub fn last_user_text(body: &Value) -> String {
    let messages = body["messages"].as_array().cloned().unwrap_or_default();
    let last = messages
        .iter()
        .rev()
        .find(|message| message["role"] == "user");
    last.map(|message| conversation(&json!({ "messages": [message] })))
        .unwrap_or_default()
}

/// Whether a credential header holds anything but the placeholder; records its name if so.
fn refused(shared: &Shared, headers: &HeaderMap) -> bool {
    let bearer = format!("Bearer {PLACEHOLDER}");
    let mut names = Vec::new();
    for (name, value) in headers {
        let name = name.as_str().to_ascii_lowercase();
        let credential = ["auth", "key", "token", "cookie", "secret"]
            .into_iter()
            .any(|part| name.contains(part));
        let value = value.to_str().unwrap_or_default();
        if credential && value != PLACEHOLDER && value != bearer {
            names.push(name);
        }
    }
    let any = !names.is_empty();
    locked(&shared.foreign).extend(names);
    any
}

async fn count_tokens(State(shared): State<Arc<Shared>>, headers: HeaderMap) -> Response {
    if refused(&shared, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    axum::Json(json!({ "input_tokens": 1 })).into_response()
}

async fn messages(State(shared): State<Arc<Shared>>, headers: HeaderMap, body: Bytes) -> Response {
    if refused(&shared, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let body: Value = serde_json::from_slice(&body).unwrap_or_default();
    let turn = body["tools"]
        .as_array()
        .is_some_and(|tools| !tools.is_empty());
    let reply = if turn {
        locked(&shared.turns).push(body.clone());
        shared.changed.notify_all();
        // The answer may wait for the test, as for a model that takes its time.
        let answering = Arc::clone(&shared);
        let asked = body.clone();
        tokio::task::spawn_blocking(move || (answering.answer)(&asked))
            .await
            .unwrap_or(Reply::Done)
    } else {
        Reply::Done
    };
    let count = locked(&shared.turns).len();
    let (block, delta, stop) = match reply {
        Reply::Fail => {
            let error = json!({
                "type": "error",
                "error": { "type": "invalid_request_error", "message": "stub failure" },
            });
            return (StatusCode::BAD_REQUEST, axum::Json(error)).into_response();
        }
        Reply::Done => (
            json!({ "type": "text", "text": "" }),
            json!({ "type": "text_delta", "text": DONE }),
            "end_turn",
        ),
        Reply::Bash(command) => tool_use(count, "Bash".to_owned(), json!({ "command": command })),
        Reply::Skald(tool) => tool_use(count, skald_tool(tool), json!({})),
    };
    let usage = json!({ "input_tokens": 1, "output_tokens": 1 });
    let message = json!({
        "id": format!("msg_socket_{count}"), "type": "message", "role": "assistant",
        "model": body["model"], "content": [], "stop_reason": null, "stop_sequence": null,
        "usage": usage,
    });
    let mut stream = String::new();
    for (kind, mut data) in [
        ("message_start", json!({ "message": message })),
        (
            "content_block_start",
            json!({ "index": 0, "content_block": block }),
        ),
        ("content_block_delta", json!({ "index": 0, "delta": delta })),
        ("content_block_stop", json!({ "index": 0 })),
        (
            "message_delta",
            json!({ "delta": { "stop_reason": stop, "stop_sequence": null }, "usage": usage }),
        ),
        ("message_stop", json!({})),
    ] {
        data["type"] = json!(kind);
        stream.push_str(&format!("event: {kind}\ndata: {data}\n\n"));
    }
    ([(header::CONTENT_TYPE, "text/event-stream")], stream).into_response()
}

/// A tool call's block, its input's delta and its stop reason.
fn tool_use(count: usize, name: String, input: Value) -> (Value, Value, &'static str) {
    (
        json!({ "type": "tool_use", "id": format!("toolu_socket_{count}"), "name": name, "input": {} }),
        json!({ "type": "input_json_delta", "partial_json": input.to_string() }),
        "tool_use",
    )
}
