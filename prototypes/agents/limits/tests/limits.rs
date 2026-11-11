//! `agent-limits` on outputs recorded from the real CLIs, in `tests/outputs/`, given by stand-ins
//! for `claude` and `codex`:
//! - `claude-usage.jsonl`: `claude` with `agent_limits::CLAUDE_USAGE`, Claude Code 2.1.322 on a
//!   Claude subscription, without its init line, its usage breakdown and the credential's expiry;
//! - `claude-usage-api-key.jsonl`: the same on an API key, without its init line;
//! - `claude-status-line.json`: the input of Claude Code 2.1.322's status line command on a Claude
//!   subscription after the session's first response, and `claude-status-line-api-key.json` one
//!   on an API key; their paths replaced;
//! - `codex-api-key.jsonl`: `agent-limits codex`'s exchange with the Codex 0.162.1 app-server on an
//!   OpenAI API key, without the notifications about the user's config and machine, the config
//!   home replaced;
//! - `codex-rate-limits-updated.json`: the notification of the Codex 0.162.1 app-server after a
//!   turn on a model stub that sent rate limit headers.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use agent_limits::{Limit, claude_status_line};
use agent_testing::{Exchange, assert_failed, stderr, write_executable};
use serde_json::{Value, json};

fn output_file(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/outputs/{name}"))
}

fn recorded(name: &str) -> String {
    fs::read_to_string(output_file(name)).unwrap()
}

/// Runs `agent-limits <args>` with `stdin` and only `PATH`: `bin`, then the system's for the
/// stand-ins' shell.
fn agent_limits(args: &[&str], bin: &Path, stdin: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_agent-limits"))
        .args(args)
        .env_clear()
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

/// The JSON lines a successful run printed.
fn printed(output: &Output) -> Vec<Value> {
    assert!(output.status.success(), "{}", stderr(output));
    String::from_utf8(output.stdout.clone())
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn limit(name: &str, used_percent: f64, resets_at: i64) -> Value {
    json!({ "limit": name, "usedPercent": used_percent, "resetsAt": resets_at })
}

/// A directory with a stand-in `claude` in `bin` that writes its arguments, one per line, to
/// `args`, prints `output` and exits with `code`.
fn claude_stand_in(output: &str, code: u8) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("bin")).unwrap();
    fs::write(dir.path().join("output"), output).unwrap();
    let script = format!(
        "#!/bin/sh\n\
         printf '%s\\n' \"$@\" >'{dir}/args'\n\
         cat '{dir}/output'\n\
         exit {code}\n",
        dir = dir.path().display()
    );
    write_executable(&dir.path().join("bin/claude"), &script);
    dir
}

fn lines(path: &Path) -> Vec<String> {
    let text = fs::read_to_string(path).unwrap();
    text.lines().map(String::from).collect()
}

#[test]
fn claude_prints_each_limit_of_the_usage_report() {
    let stand_in = claude_stand_in(&recorded("claude-usage.jsonl"), 0);
    let output = agent_limits(&["claude"], &stand_in.path().join("bin"), "");
    assert_eq!(
        printed(&output),
        [
            limit("session", 29.0, 1_794_424_799),
            limit("weekly_all", 94.0, 1_794_754_799),
            limit("weekly_scoped Fable", 1.0, 1_794_754_799),
        ]
    );
    assert_eq!(
        lines(&stand_in.path().join("args")),
        [
            "-p",
            "/usage",
            "--output-format",
            "stream-json",
            "--verbose",
            "--setting-sources",
            "local",
            "--strict-mcp-config",
            "--no-session-persistence",
        ]
    );
}

#[test]
fn claude_prints_no_limit_on_an_api_key() {
    let stand_in = claude_stand_in(&recorded("claude-usage-api-key.jsonl"), 0);
    let output = agent_limits(&["claude"], &stand_in.path().join("bin"), "");
    assert_eq!(printed(&output), Vec::<Value>::new());
}

#[test]
fn claude_writes_the_cli_output_to_the_log() {
    let usage = recorded("claude-usage.jsonl");
    let stand_in = claude_stand_in(&usage, 0);
    let log = stand_in.path().join("usage.log");
    let output = agent_limits(
        &["claude", "--log", log.to_str().unwrap()],
        &stand_in.path().join("bin"),
        "",
    );
    assert_eq!(printed(&output).len(), 3);
    assert_eq!(fs::read_to_string(log).unwrap(), usage);
}

#[test]
fn claude_fails_when_the_cli_fails() {
    let stand_in = claude_stand_in(&recorded("claude-usage.jsonl"), 3);
    let output = agent_limits(&["claude"], &stand_in.path().join("bin"), "");
    assert_failed(&output, "agent-limits: claude /usage failed");
    assert!(output.stdout.is_empty());
}

/// A directory with a stand-in `codex` in `bin` that replays `exchange` as `codex app-server`
/// ([`Exchange::stand_in`]).
fn codex_stand_in(exchange: &Exchange) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    exchange.stand_in(dir.path(), ":", ":");
    dir
}

/// The lines the stand-in `codex` in `dir` received.
fn received(dir: &Path) -> Vec<Value> {
    let received = dir.join("received");
    (0..fs::read_dir(&received).unwrap().count())
        .map(|n| {
            serde_json::from_str(&fs::read_to_string(received.join(n.to_string())).unwrap())
                .unwrap()
        })
        .collect()
}

fn api_key_exchange() -> Exchange {
    Exchange::read(&output_file("codex-api-key.jsonl"))
}

/// The methods of `messages`, `<answer>` for an answer.
fn methods<'a>(messages: impl IntoIterator<Item = &'a Value>) -> Vec<String> {
    messages
        .into_iter()
        .map(|message| message["method"].as_str().unwrap_or("<answer>").to_owned())
        .collect()
}

#[test]
fn codex_prints_the_account_and_why_an_api_key_has_no_limits() {
    let exchange = api_key_exchange();
    let stand_in = codex_stand_in(&exchange);
    let output = agent_limits(
        &["codex", "--", "-c", "a=1"],
        &stand_in.path().join("bin"),
        "",
    );
    assert_eq!(
        printed(&output),
        [
            json!({ "account": "apiKey" }),
            json!({ "refused": "chatgpt authentication required to read rate limits" }),
        ]
    );
    assert_eq!(
        lines(&stand_in.path().join("args")),
        ["app-server", "-c", "a=1"]
    );
    assert_eq!(
        methods(&received(stand_in.path())),
        methods(exchange.client())
    );
}

#[test]
fn codex_prints_each_limit_the_app_server_reads() {
    let updated: Value = serde_json::from_str(&recorded("codex-rate-limits-updated.json")).unwrap();
    let mut exchange = api_key_exchange();
    let last = exchange.0.last_mut().unwrap();
    *last = json!({ "server": { "id": 2, "result": updated["params"] } });
    let stand_in = codex_stand_in(&exchange);
    let output = agent_limits(&["codex"], &stand_in.path().join("bin"), "");
    assert_eq!(
        printed(&output),
        [
            json!({ "account": "apiKey" }),
            limit("codex primary 300 min", 23.0, 1_794_432_801),
            limit("codex secondary 10080 min", 41.0, 1_794_767_601),
        ]
    );
    assert_eq!(lines(&stand_in.path().join("args")), ["app-server"]);
}

#[test]
fn codex_writes_the_exchange_to_the_log() {
    let exchange = api_key_exchange();
    let stand_in = codex_stand_in(&exchange);
    let log = stand_in.path().join("exchange.jsonl");
    let output = agent_limits(
        &["codex", "--log", log.to_str().unwrap()],
        &stand_in.path().join("bin"),
        "",
    );
    assert_eq!(printed(&output).len(), 2);
    assert_eq!(Exchange::read(&log).0, exchange.0);
}

#[test]
fn codex_fails_when_the_app_server_closes() {
    let bin = tempfile::tempdir().unwrap();
    // It reads the client's first line, so the client's write never meets a closed pipe.
    write_executable(
        &bin.path().join("codex"),
        "#!/bin/sh\nread -r line\nexit 0\n",
    );
    let output = agent_limits(&["codex"], bin.path(), "");
    assert_failed(
        &output,
        "agent-limits: the app-server closed the connection",
    );
    assert!(output.stdout.is_empty());
}

#[test]
fn the_status_line_shows_the_limits_of_its_input_and_logs_each_input() {
    let empty = tempfile::tempdir().unwrap();
    let log = empty.path().join("status-line.jsonl");
    let with = recorded("claude-status-line.json");
    let without = recorded("claude-status-line-api-key.json");
    let run = |input: &str, args: &[&str]| {
        let output = agent_limits(args, empty.path(), input);
        assert!(output.status.success(), "{}", stderr(&output));
        String::from_utf8(output.stdout).unwrap()
    };
    let args = ["status-line", "--log", log.to_str().unwrap()];
    assert_eq!(run(&with, &args), "five_hour 30% · seven_day 94%\n");
    assert_eq!(run(without.trim_end(), &args), "no limits\n");
    assert_eq!(
        run(&with, &["status-line"]),
        "five_hour 30% · seven_day 94%\n"
    );
    assert_eq!(lines(&log), [with.trim_end(), without.trim_end()]);
}

#[test]
fn the_status_line_input_gives_each_window_with_its_usage_and_reset_time() {
    let with = recorded("claude-status-line.json");
    assert_eq!(
        claude_status_line(&with),
        [
            Limit {
                name: "five_hour".to_owned(),
                used_percent: 30.0,
                resets_at: Some(1_794_424_800),
            },
            Limit {
                name: "seven_day".to_owned(),
                used_percent: 94.0,
                resets_at: Some(1_794_754_800),
            },
        ]
    );
    let without = recorded("claude-status-line-api-key.json");
    assert_eq!(claude_status_line(&without), []);
}

#[test]
fn wrong_arguments_print_the_usage() {
    let empty = tempfile::tempdir().unwrap();
    for args in [
        &[][..],
        &["gemini"],
        &["claude", "--log"],
        &["claude", "--verbose"],
        &["codex", "--log"],
        &["codex", "-c", "a=1"],
        &["status-line", "a"],
        &["status-line", "--log"],
        &["codex", "--log", "x", "-c"],
    ] {
        let output = agent_limits(args, empty.path(), "");
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert!(
            stderr(&output).starts_with("usage: agent-limits"),
            "{args:?}"
        );
    }
}
