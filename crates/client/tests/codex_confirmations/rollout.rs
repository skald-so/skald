//! A Codex session's rollout, the session file its `SessionStart` names, grown entry by entry as
//! Codex writes it: one JSON object a line, each written whole and at once. The entries have the
//! shapes of Codex 0.162.1's, as the live checks of the delivery prototypes recorded them
//! (`prototypes/agents/live/tests/codex_permission/session-*.jsonl`, `transcript/src/lib.rs`) and
//! as a call of Skald's `inbox` on the model stub did (`recorded/inbox.jsonl`): a model's call of an
//! MCP tool is a `function_call` whose `namespace` is `mcp__<server>`, its result a
//! `function_call_output` whose `output` is an array of `input_text` items, the call's wall time,
//! then each text of the tool result's content; the model's own entries are an assistant
//! `message`, `reasoning` or a call; a failed call to the model writes no `response_item`, only the
//! turn's end with its error.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

use serde_json::{Value, json};
use skald_proto::MessageId;

use crate::codex::Thread;

/// The Codex version whose rollouts these entries follow.
const CODEX_VERSION: &str = "0.162.1";

/// The MCP server's name in the Codex config, as Skald's Codex setup names it.
const SERVER: &str = "skald";

/// A call of the model's, by its `call_id`.
#[derive(Debug, Clone)]
pub struct Call(String);

/// A session's rollout.
pub struct Rollout {
    path: PathBuf,
    /// The running turn's ID.
    turn: String,
    /// The next entry's `ordinal`.
    ordinal: u64,
    /// The calls of the model's so far.
    calls: u64,
}

impl Rollout {
    /// `of`'s rollout as Codex starts it: the session's meta, then a turn's start with the person's
    /// prompt.
    pub fn begin(of: &Thread) -> Self {
        let path = of.rollout();
        let dir = path.parent().expect("the rollout's directory");
        fs::create_dir_all(dir).expect("make the rollout's directory");
        fs::write(&path, "").expect("make the rollout");
        let mut rollout = Rollout {
            path,
            turn: MessageId::generate().to_string(),
            ordinal: 0,
            calls: 0,
        };
        let cwd = of.dir.to_string_lossy().into_owned();
        rollout.append(
            "session_meta",
            json!({ "id": of.id, "cwd": cwd, "cli_version": CODEX_VERSION }),
        );
        rollout.turn_starts("Run the tests of the web app, then reply.");
        rollout
    }

    /// `of`'s rollout, a file Skald cannot parse: lines that are no JSON, as a Codex that wrote its
    /// sessions in another format would.
    pub fn unparsable(of: &Thread) -> Self {
        let path = of.rollout();
        let dir = path.parent().expect("the rollout's directory");
        fs::create_dir_all(dir).expect("make the rollout's directory");
        let lines = [
            "\u{1f}codex rollout, format 9: binary frames follow",
            "frame 1 <session meta> 0x00 0x17 0x2a",
            "frame 2 <turn> 0x01 0x40",
        ];
        fs::write(&path, format!("{}\n", lines.join("\n"))).expect("write the rollout");
        Rollout {
            path,
            turn: MessageId::generate().to_string(),
            ordinal: 3,
            calls: 0,
        }
    }

    /// Appends the entry of `kind` with `payload`, a whole line in one write.
    fn append(&mut self, kind: &str, payload: Value) {
        let timestamp = format!("{:.3}", jiff::Timestamp::now());
        let entry = json!({
            "timestamp": timestamp,
            "ordinal": self.ordinal,
            "type": kind,
            "payload": payload,
        });
        self.ordinal += 1;
        let mut file = OpenOptions::new()
            .append(true)
            .open(&self.path)
            .expect("open the rollout");
        file.write_all(format!("{entry}\n").as_bytes())
            .expect("write to the rollout");
        file.flush().expect("write to the rollout");
    }

    fn next_call(&mut self) -> Call {
        self.calls += 1;
        Call(format!("call_{}_{}", self.calls, MessageId::generate()))
    }

    /// A turn starts with the person's `prompt`.
    pub fn turn_starts(&mut self, prompt: &str) {
        self.turn = MessageId::generate().to_string();
        let turn = self.turn.clone();
        self.append(
            "event_msg",
            json!({ "type": "task_started", "turn_id": turn }),
        );
        self.append(
            "turn_context",
            json!({
                "turn_id": turn, "approval_policy": "never",
                "sandbox_policy": { "type": "workspace-write" },
            }),
        );
        self.append(
            "response_item",
            json!({
                "type": "message", "role": "user",
                "content": [{ "type": "input_text", "text": prompt }],
            }),
        );
    }

    /// The model calls the `inbox` tool of Skald's MCP server. Gives the call.
    pub fn calls_inbox(&mut self) -> Call {
        let call = self.next_call();
        self.append(
            "response_item",
            json!({
                "type": "function_call", "id": format!("fc_{}", call.0), "call_id": call.0,
                "namespace": format!("mcp__{SERVER}"), "name": "inbox", "arguments": "{}",
            }),
        );
        call
    }

    /// The model runs `command` in the shell. Gives the call.
    pub fn calls_command(&mut self, command: &str) -> Call {
        let call = self.next_call();
        let arguments = json!({ "cmd": command, "yield_time_ms": 10_000 }).to_string();
        self.append(
            "response_item",
            json!({
                "type": "function_call", "id": format!("fc_{}", call.0), "call_id": call.0,
                "name": "exec_command", "arguments": arguments,
            }),
        );
        call
    }

    /// The MCP tool of `call` gave `text`: its result goes to the model.
    pub fn mcp_result(&mut self, call: &Call, text: &str) {
        let content = json!([
            { "type": "input_text", "text": "Wall time: 0.0059 seconds\nOutput:" },
            { "type": "input_text", "text": text },
        ]);
        self.append(
            "response_item",
            json!({
                "type": "function_call_output", "id": format!("fco_{}", call.0),
                "call_id": call.0, "output": content,
            }),
        );
    }

    /// The command of `call` printed `text`: its output goes to the model.
    pub fn command_output(&mut self, call: &Call, text: &str) {
        let output = format!(
            "Chunk ID: 0b21cf\nWall time: 0.0100 seconds\nProcess exited with code 0\nOutput:\n{text}"
        );
        self.append(
            "response_item",
            json!({
                "type": "function_call_output", "id": format!("fco_{}", call.0),
                "call_id": call.0, "output": output,
            }),
        );
    }

    /// The call to the model fails: the turn ends with the API's error, and no entry of the
    /// model's.
    pub fn api_error(&mut self) {
        let turn = self.turn.clone();
        self.append(
            "event_msg",
            json!({
                "type": "task_complete", "turn_id": turn, "last_agent_message": null,
                "error": { "message": "unexpected status 500 Internal Server Error" },
            }),
        );
    }

    /// The model replies `text`: its own entry.
    pub fn model_says(&mut self, text: &str) {
        let id = format!("msg_{}", MessageId::generate());
        self.append(
            "response_item",
            json!({
                "type": "message", "id": id, "role": "assistant",
                "content": [{ "type": "output_text", "text": text }],
            }),
        );
    }

    /// The model replies `Done.`.
    pub fn model_replies(&mut self) {
        self.model_says("Done.");
    }
}
