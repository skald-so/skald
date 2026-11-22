//! What a check asks Codex's app-server, `codex app-server` on stdio, about a config home: the
//! models whose availability notice the terminal UI shows, and its hooks, which it trusts as a
//! person does in Review hooks, by writing each hook's current hash into `hooks.state` with
//! `config/batchWrite`.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{Map, Value, json};

/// A running `codex app-server`; killed on drop.
pub struct AppServer {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    next: u64,
}

impl AppServer {
    /// Starts `codex app-server` as `template`, a `codex` with its settings, and initializes it.
    pub fn start(mut template: Command) -> Self {
        let mut child = (template.arg("app-server"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("start codex app-server");
        let input = child.stdin.take().expect("stdin is piped");
        let output = BufReader::new(child.stdout.take().expect("stdout is piped"));
        let mut server = AppServer {
            child,
            input,
            output,
            next: 0,
        };
        let client = json!({ "name": "skald_live_check", "title": null, "version": "0" });
        server.call("initialize", json!({ "clientInfo": client }));
        server.send(&json!({ "method": "initialized" }));
        server
    }

    /// The result of the request `method` with `params`; fails on an error.
    #[track_caller]
    pub fn call(&mut self, method: &str, params: Value) -> Value {
        let id = self.next;
        self.next += 1;
        self.send(&json!({ "id": id, "method": method, "params": params }));
        loop {
            let mut line = String::new();
            let read = (self.output.read_line(&mut line)).expect("read the app-server");
            assert!(read > 0, "the app-server ended before answering {method}");
            let Ok(message) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if message["id"] == id {
                assert!(message["error"].is_null(), "{method}: {message}");
                return message["result"].clone();
            }
        }
    }

    fn send(&mut self, message: &Value) {
        writeln!(self.input, "{message}").expect("write to the app-server");
        self.input.flush().expect("write to the app-server");
    }

    /// Trusts every hook of sessions in `dir` that is not trusted yet, as Review hooks does; gives
    /// the hooks it listed before, as `hooks/list` gives them, and the commands it trusted.
    pub fn trust_hooks(&mut self, dir: &Path) -> (Value, Vec<String>) {
        let listed = self.call("hooks/list", json!({ "cwds": [dir] }));
        let (state, commands) = untrusted(&listed);
        if !state.is_empty() {
            let edit =
                json!({ "keyPath": "hooks.state", "mergeStrategy": "upsert", "value": state });
            self.call("config/batchWrite", json!({ "edits": [edit] }));
        }
        (listed, commands)
    }

    /// The commands of the hooks of sessions in `dir` that Codex does not trust.
    pub fn untrusted_hooks(&mut self, dir: &Path) -> Vec<String> {
        untrusted(&self.call("hooks/list", json!({ "cwds": [dir] }))).1
    }
}

impl Drop for AppServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The models of a result of `model/list` that come with an availability notice.
pub fn noticed_models(listed: &Value) -> Vec<String> {
    (listed["data"].as_array().map(Vec::as_slice))
        .unwrap_or_default()
        .iter()
        .filter(|model| model["availabilityNux"].is_object())
        .filter_map(|model| model["model"].as_str())
        .map(str::to_owned)
        .collect()
}

/// From a result of `hooks/list`: the `hooks.state` entries that trust each hook not trusted yet,
/// by its key, and their commands.
pub fn untrusted(listed: &Value) -> (Map<String, Value>, Vec<String>) {
    let mut state = Map::new();
    let mut commands = Vec::new();
    let entries = (listed["data"].as_array().map(Vec::as_slice)).unwrap_or_default();
    let hooks = (entries.iter())
        .filter_map(|entry| entry["hooks"].as_array())
        .flatten();
    for hook in hooks.filter(|hook| hook["trustStatus"] != "trusted") {
        let (Some(key), Some(hash)) = (hook["key"].as_str(), hook["currentHash"].as_str()) else {
            continue;
        };
        state.insert(key.to_owned(), json!({ "trusted_hash": hash }));
        commands.push(hook["command"].as_str().unwrap_or_default().to_owned());
    }
    (state, commands)
}

/// `model/list`, recorded from Codex 0.162.1: only its default model comes with a notice.
#[test]
fn the_noticed_models_are_those_with_an_availability_notice() {
    let listed: Value =
        serde_json::from_str(include_str!("recorded/model-list.json")).expect("JSON");
    assert_eq!(noticed_models(&listed), ["gpt-6.1-sol"]);
    assert!(noticed_models(&json!({ "data": [{ "model": "a" }] })).is_empty());
}

/// `hooks/list`, recorded from Codex 0.162.1 in a home Paseo and Skald wrote into, before any hook
/// was trusted: each gets its key's current hash; a trusted one gets none.
#[test]
fn trusting_takes_the_hooks_not_trusted_yet() {
    let listed: Value =
        serde_json::from_str(include_str!("recorded/hooks-list.json")).expect("JSON");
    let (state, commands) = untrusted(&listed);
    assert_eq!((state.len(), commands.len()), (9, 9));
    let skald = "/home/ada/.codex/config.toml:session_start:0:0";
    assert_eq!(
        state[skald]["trusted_hash"],
        "sha256:d60b2cc12f01512b5163791eb2c0074f953ccdc5fb8fcd4c83ddbeed324be847"
    );
    assert!(
        commands
            .iter()
            .any(|command| command.ends_with(" hook codex")),
        "{commands:?}"
    );
    assert!(
        commands
            .iter()
            .any(|command| command.contains("hooks codex Stop")),
        "{commands:?}"
    );
    let paseo = "/home/ada/.codex/hooks.json:stop:0:0";
    let trusted = listed.to_string().replace("\"untrusted\"", "\"trusted\"");
    let trusted: Value = serde_json::from_str(&trusted).expect("JSON");
    assert!(untrusted(&trusted).0.is_empty());
    assert!(state.contains_key(paseo), "{state:?}");
}
