//! The agent CLI these tests play: an MCP client of `skald mcp` over its standard streams, MCP's
//! stdio transport, one JSON-RPC 2.0 message per line. It answers a request of the server's with
//! an empty result for `ping` and an error for anything else, and keeps the server's notifications
//! without acting on them.

use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::machine::{Machine, RUN_LIMIT};

/// The command of the MCP server, and its tool that reads the agent's messages.
pub const MCP: &str = "mcp";
pub const INBOX: &str = "inbox";

/// The send tool and the reply tool, and their arguments.
pub const SEND_TOOL: &str = "send";
pub const REPLY_TOOL: &str = "reply";
pub const TO_FIELD: &str = "to";
pub const TEXT_FIELD: &str = "text";
pub const CHANNEL_FIELD: &str = "channel";
pub const MESSAGE_FIELD: &str = "message";

/// What the send tool's `to` takes for every agent in the channel.
pub const EVERYONE: &str = "*";

/// The name the MCP client of most tests gives itself at initialization.
pub const CLIENT: &str = "claude-code";

/// How many bytes the result of `tools/list` has at most, as compact JSON: room for `inbox`, the
/// send tool and the reply tool, with short descriptions.
pub const TOOLS_BOUND: usize = 2048;

/// The MCP revision the client asks for.
const PROTOCOL: &str = "2025-06-18";

/// The MCP revision whose client opens the session with `server/discover`, not `initialize`, and
/// sends its version, name and capabilities in each request's `_meta`.
const STATELESS: &str = "2026-07-28";

/// How long the server is given to exit once its client is gone.
const EXIT: Duration = Duration::from_secs(30);

/// Starts `skald <args> mcp` on `machine` in `dir`, set up as the machine's runs are, and
/// initializes it as the MCP client named `client`.
pub fn session_in(machine: &Machine, dir: &Path, args: &[&str], client: &str) -> Session {
    let mut all = args.to_vec();
    all.push(MCP);
    Session::start(&mut machine.command_in(dir, &all), client)
}

/// The fields the input schema `schema` of a tool requires.
pub fn required(schema: &Value) -> Vec<&str> {
    (schema["required"].as_array().map(Vec::as_slice))
        .unwrap_or_default()
        .iter()
        .filter_map(Value::as_str)
        .collect()
}

/// What a tool call gave: its result's text, and whether it is an error, a JSON-RPC error's
/// message and data in place of the text.
#[derive(Debug)]
pub struct Called {
    pub text: String,
    pub error: bool,
}

/// What the server wrote to its standard output, which a thread reads in small pieces.
#[derive(Default)]
struct Output {
    bytes: Mutex<Vec<u8>>,
    /// Whether the output has ended, or the client has closed its reading end.
    ended: AtomicBool,
    /// Once set, the reading end closes as soon as the output holds this text, leaving the rest
    /// unread: the client is gone.
    close_at: Mutex<Option<String>>,
}

/// A running `skald mcp` and its client; the server is killed on drop.
pub struct Session {
    child: Child,
    input: Option<ChildStdin>,
    output: Arc<Output>,
    /// How many bytes of the output the client has taken as messages.
    taken: usize,
    errors: Arc<Output>,
    /// The ID of the next request.
    next: u64,
    /// What each request's `_meta` holds, as of [`STATELESS`].
    meta: Option<Value>,
    /// The result of `initialize`, once the server gave it.
    initialized: Value,
    /// The server's notifications so far, in order.
    notifications: Vec<Value>,
}

impl Session {
    /// Starts `command`, a run of `skald mcp`, and initializes it as the client named `client`;
    /// fails unless the server answers `initialize` with a result.
    #[track_caller]
    pub fn start(command: &mut Command, client: &str) -> Self {
        let mut session = Self::spawn(command);
        let params = json!({
            "protocolVersion": PROTOCOL,
            "capabilities": {},
            "clientInfo": { "name": client, "version": "1.0.0" },
        });
        match session.request("initialize", params) {
            Ok(result) => session.initialized = result,
            Err(error) => panic!("initialize failed: {error}: {}", session.errors()),
        }
        session.write(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));
        session
    }

    /// Starts `command`, a run of `skald mcp`, and opens the session as a client of [`STATELESS`]
    /// named `client` does, as Claude Code does: `server/discover`, and no `initialize`; fails
    /// unless the server answers it with a result, which [`Session::initialized`] then gives.
    #[track_caller]
    pub fn discovered(command: &mut Command, client: &str) -> Self {
        let mut session = Self::spawn(command);
        session.meta = Some(json!({
            "io.modelcontextprotocol/protocolVersion": STATELESS,
            "io.modelcontextprotocol/clientInfo": { "name": client, "version": "1.0.0" },
            "io.modelcontextprotocol/clientCapabilities": {},
        }));
        match session.request("server/discover", json!({})) {
            Ok(result) => session.initialized = result,
            Err(error) => panic!("server/discover failed: {error}: {}", session.errors()),
        }
        session
    }

    /// Starts `command`, a run of `skald mcp`, with its input and outputs piped.
    fn spawn(command: &mut Command) -> Self {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("run skald mcp");
        let output = Arc::new(Output::default());
        read(child.stdout.take().expect("stdout is piped"), &output);
        let errors = Arc::new(Output::default());
        read(child.stderr.take().expect("stderr is piped"), &errors);
        Session {
            input: child.stdin.take(),
            child,
            output,
            taken: 0,
            errors,
            next: 1,
            meta: None,
            initialized: Value::Null,
            notifications: Vec::new(),
        }
    }

    /// The server's process ID.
    pub fn id(&self) -> u32 {
        self.child.id()
    }

    /// What the server wrote to its standard error so far.
    pub fn errors(&self) -> String {
        String::from_utf8_lossy(&self.errors.bytes.lock().expect("the server's errors"))
            .into_owned()
    }

    /// The result the server gave `initialize`.
    pub fn initialized(&self) -> &Value {
        &self.initialized
    }

    /// The server's notifications of `method` so far, waiting for up to `wait` until it sent
    /// `count` of them; answers the server's requests meanwhile.
    #[track_caller]
    pub fn notifications(&mut self, method: &str, count: usize, wait: Duration) -> Vec<Value> {
        let deadline = Instant::now() + wait;
        loop {
            while let Some(message) = self.taken_message() {
                self.answer(&message);
            }
            let sent: Vec<Value> = (self.notifications.iter())
                .filter(|notification| notification["method"] == method)
                .cloned()
                .collect();
            if sent.len() >= count || Instant::now() >= deadline {
                return sent;
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    /// Calls `tool` with `arguments`.
    #[track_caller]
    pub fn call(&mut self, tool: &str, arguments: Value) -> Called {
        let params = json!({ "name": tool, "arguments": arguments });
        match self.request("tools/call", params) {
            Ok(result) => Called {
                text: text_of(&result),
                error: result["isError"].as_bool().unwrap_or(false),
            },
            Err(error) => Called {
                text: format!("{} {}", error["message"], error["data"]),
                error: true,
            },
        }
    }

    /// Calls `inbox`; fails if it is an error. Gives its text.
    #[track_caller]
    pub fn inbox(&mut self) -> String {
        let called = self.call(INBOX, json!({}));
        assert!(!called.error, "inbox failed: {called:?}: {}", self.errors());
        called.text
    }

    /// The result of `tools/list`; fails unless there is one.
    #[track_caller]
    pub fn tools(&mut self) -> Value {
        match self.request("tools/list", json!({})) {
            Ok(result) => result,
            Err(error) => panic!("tools/list failed: {error}: {}", self.errors()),
        }
    }

    /// Calls `inbox` and is gone once the server has written `head` of its result: the client
    /// closes the reading end of the server's output with the rest unread, then its input. Waits
    /// for up to [`EXIT`] for the server to exit, then kills it. Gives what the client read.
    #[track_caller]
    pub fn gone_in_the_middle_of_inbox(mut self, head: &str) -> String {
        *self.output.close_at.lock().expect("the server's output") = Some(head.to_owned());
        let id = self.next;
        self.next += 1;
        self.write(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": { "name": INBOX, "arguments": {} },
        }));
        let deadline = Instant::now() + RUN_LIMIT;
        while !self.output.ended.load(Ordering::SeqCst) {
            assert!(
                Instant::now() < deadline,
                "no {head:?} in the result: {}",
                self.errors()
            );
            thread::sleep(Duration::from_millis(10));
        }
        self.wait_for_exit();
        let read = self.output.bytes.lock().expect("the server's output");
        String::from_utf8_lossy(&read).into_owned()
    }

    /// Closes the input and captures both streams through the server's exit.
    #[allow(dead_code)]
    pub fn finish(mut self) -> (Vec<u8>, Vec<u8>) {
        assert!(self.wait_for_exit(), "skald mcp did not exit successfully");
        let deadline = Instant::now() + EXIT;
        let drained = |stream: &Output| {
            while !stream.ended.load(Ordering::SeqCst) {
                assert!(
                    Instant::now() < deadline,
                    "skald mcp's output did not close"
                );
                thread::sleep(Duration::from_millis(10));
            }
            stream.bytes.lock().expect("the server's output").clone()
        };
        (drained(&self.output), drained(&self.errors))
    }

    fn wait_for_exit(&mut self) -> bool {
        drop(self.input.take());
        let deadline = Instant::now() + EXIT;
        loop {
            if let Some(status) = self.child.try_wait().expect("wait for skald mcp") {
                return status.success();
            }
            if Instant::now() > deadline {
                let _ = self.child.kill();
                return false;
            }
            thread::sleep(Duration::from_millis(50));
        }
    }

    /// Sends the request `method` with `params`; gives its result, or its error. Answers the
    /// server's requests meanwhile.
    #[track_caller]
    fn request(&mut self, method: &str, mut params: Value) -> Result<Value, Value> {
        let id = self.next;
        self.next += 1;
        if let Some(meta) = &self.meta {
            params["_meta"] = meta.clone();
        }
        self.write(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        loop {
            let message = self.next_message();
            if message["id"] == json!(id) && message.get("method").is_none() {
                return match message.get("error") {
                    Some(error) => Err(error.clone()),
                    None => Ok(message["result"].clone()),
                };
            }
            self.answer(&message);
        }
    }

    /// Keeps `message` if it is a notification of the server's, and answers it if it is a request.
    #[track_caller]
    fn answer(&mut self, message: &Value) {
        let Some(method) = message["method"].as_str() else {
            return;
        };
        let Some(asked) = message.get("id") else {
            self.notifications.push(message.clone());
            return;
        };
        let answer = if method == "ping" {
            json!({ "jsonrpc": "2.0", "id": asked, "result": {} })
        } else {
            let error = json!({ "code": -32601, "message": "method not found" });
            json!({ "jsonrpc": "2.0", "id": asked, "error": error })
        };
        self.write(&answer);
    }

    /// Writes `message` to the server's input, on a line of its own.
    #[track_caller]
    fn write(&mut self, message: &Value) {
        let input = self.input.as_mut().expect("the server's input is open");
        let written = writeln!(input, "{message}").and_then(|()| input.flush());
        if let Err(err) = written {
            panic!("write to skald mcp: {err}: {}", self.errors());
        }
    }

    /// The next message on the server's output, waiting for up to [`RUN_LIMIT`]; fails if a line
    /// is no JSON object, or if the output ends first.
    #[track_caller]
    fn next_message(&mut self) -> Value {
        let deadline = Instant::now() + RUN_LIMIT;
        loop {
            if let Some(message) = self.taken_message() {
                return message;
            }
            if self.output.ended.load(Ordering::SeqCst) {
                panic!("skald mcp's output ended: {}", self.errors());
            }
            assert!(
                Instant::now() < deadline,
                "skald mcp did not answer in {RUN_LIMIT:?}: {}",
                self.errors()
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    /// The next message the server's output holds now, if it holds a whole one; fails if a line is
    /// no JSON object.
    #[track_caller]
    fn taken_message(&mut self) -> Option<Value> {
        loop {
            let (line, length) = {
                let bytes = self.output.bytes.lock().expect("the server's output");
                let end = bytes[self.taken..].iter().position(|&byte| byte == b'\n')?;
                let line = String::from_utf8_lossy(&bytes[self.taken..self.taken + end]);
                (line.into_owned(), end + 1)
            };
            self.taken += length;
            if line.trim().is_empty() {
                continue;
            }
            return match serde_json::from_str::<Value>(&line) {
                Ok(message) if message.is_object() => Some(message),
                _ => panic!("not a JSON-RPC message on its output: {line:?}"),
            };
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        drop(self.input.take());
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The text of a tool call's result: its text blocks, a line apart.
fn text_of(result: &Value) -> String {
    let blocks = result["content"].as_array().cloned().unwrap_or_default();
    let texts: Vec<&str> = (blocks.iter())
        .filter_map(|block| block["text"].as_str())
        .collect();
    texts.join("\n")
}

/// Reads `stdout` into `output` in small pieces until it ends, or, once `output` says where to
/// close, until it holds that text; then closes it and marks the output ended.
fn read(mut stdout: impl Read + Send + 'static, output: &Arc<Output>) {
    let output = Arc::clone(output);
    thread::spawn(move || {
        let mut piece = [0; 256];
        loop {
            let close_at = output.close_at.lock().expect("the server's output").clone();
            if let Some(head) = close_at {
                let bytes = output.bytes.lock().expect("the server's output");
                if String::from_utf8_lossy(&bytes).contains(&head) {
                    break;
                }
            }
            match stdout.read(&mut piece) {
                Ok(0) | Err(_) => break,
                Ok(count) => (output.bytes.lock())
                    .expect("the server's output")
                    .extend_from_slice(&piece[..count]),
            }
        }
        drop(stdout);
        output.ended.store(true, Ordering::SeqCst);
    });
}
