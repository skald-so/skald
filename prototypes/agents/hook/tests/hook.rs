//! `agent-hook` runs as Claude Code runs it: a hook input recorded from the real Claude Code on
//! stdin, the handler's output on stdout.

use std::io::Write;
use std::path::Path;
use std::process::{Child, Command, Output, Stdio};

use agent_source::{Message, Source, State};
use serde_json::Value;

// Hook inputs recorded from Claude Code 2.1.322; the temporary directories' paths are replaced.
const POST_TOOL_USE: &str = include_str!("inputs/post-tool-use.json");
const STOP: &str = include_str!("inputs/stop.json");
/// The `Stop` input after a `Stop` hook kept the turn going.
const STOP_ACTIVE: &str = include_str!("inputs/stop-active.json");

/// Starts `agent-hook <source>` with piped stdio and no variable of the test's environment.
fn start(source: &Path) -> Child {
    Command::new(env!("CARGO_BIN_EXE_agent-hook"))
        .arg(source)
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

/// Runs `agent-hook <source>` with `input` on stdin.
fn hook(source: &Path, input: &str) -> Output {
    let mut child = start(source);
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Asserts that the handler failed with exit code 1 and an error that starts with `error`.
fn failed(output: &Output, error: &str) {
    assert_eq!(output.status.code(), Some(1), "{}", stderr(output));
    assert!(stderr(output).starts_with(error), "{}", stderr(output));
}

/// The `additionalContext` of a successful run's output for `event`.
fn context(output: &Output, event: &str) -> String {
    assert!(output.status.success(), "{}", stderr(output));
    let printed: Value = serde_json::from_slice(&output.stdout).unwrap();
    let specific = &printed["hookSpecificOutput"];
    assert_eq!(specific["hookEventName"], event);
    // Nothing that blocks a tool call or ends the session.
    assert_eq!(printed.as_object().unwrap().len(), 1, "{printed}");
    assert_eq!(specific.as_object().unwrap().len(), 2, "{printed}");
    specific["additionalContext"].as_str().unwrap().to_owned()
}

/// Asserts that `context` carries each of `messages` once, with its sender label and `msg_id`.
fn carries_once(context: &str, messages: &[Message]) {
    for message in messages {
        let element = format!(
            "<message msg_id=\"{}\" from=\"{}\">\n{}\n</message>",
            message.msg_id, message.sender, message.text
        );
        assert_eq!(context.matches(&element).count(), 1, "{context}");
        assert_eq!(context.matches(&message.msg_id).count(), 1, "{context}");
    }
}

fn quiet(output: &Output) {
    assert!(output.status.success(), "{}", stderr(output));
    assert!(output.stdout.is_empty(), "{:?}", output.stdout);
}

fn handed_over(source: &Source, messages: &[Message]) {
    for message in messages {
        assert!(matches!(
            source.state(&message.msg_id).unwrap(),
            Some(State::HandedOver { .. })
        ));
    }
}

fn waiting(source: &Source, message: &Message) {
    assert_eq!(source.state(&message.msg_id).unwrap(), Some(State::Waiting));
}

#[test]
fn post_tool_use_adds_each_waiting_message_to_the_context_once() {
    let dir = tempfile::tempdir().unwrap();
    let source = Source::open(dir.path()).unwrap();
    let messages = [
        source.add("alice of skald", "first").unwrap(),
        source.add("bob", "second").unwrap(),
    ];
    let output = hook(dir.path(), POST_TOOL_USE);
    carries_once(&context(&output, "PostToolUse"), &messages);
    handed_over(&source, &messages);
    // Handed over: the next tool call adds nothing.
    quiet(&hook(dir.path(), POST_TOOL_USE));

    let later = [source.add("carol", "third").unwrap()];
    let context = context(&hook(dir.path(), POST_TOOL_USE), "PostToolUse");
    carries_once(&context, &later);
    assert!(!context.contains(&messages[0].msg_id), "{context}");
}

#[test]
fn stop_keeps_the_turn_going_with_the_waiting_messages_and_then_lets_it_end() {
    let dir = tempfile::tempdir().unwrap();
    let source = Source::open(dir.path()).unwrap();
    let messages = [source.add("alice", "the token").unwrap()];
    // Context on Stop keeps the turn going: the agent gets the messages.
    carries_once(&context(&hook(dir.path(), STOP), "Stop"), &messages);
    handed_over(&source, &messages);
    // The turn went on; with none waiting, the next Stop lets it end.
    quiet(&hook(dir.path(), STOP_ACTIVE));

    // A message that arrives during the kept turn keeps it going again.
    let more = [source.add("bob", "more").unwrap()];
    carries_once(&context(&hook(dir.path(), STOP_ACTIVE), "Stop"), &more);
}

#[test]
fn with_none_waiting_it_adds_nothing_and_stop_lets_the_turn_end() {
    let dir = tempfile::tempdir().unwrap();
    for input in [POST_TOOL_USE, STOP, STOP_ACTIVE] {
        quiet(&hook(dir.path(), input));
    }
}

#[test]
fn another_event_takes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let source = Source::open(dir.path()).unwrap();
    let message = source.add("alice", "hi").unwrap();
    let input = POST_TOOL_USE.replace("\"PostToolUse\"", "\"PreToolUse\"");
    quiet(&hook(dir.path(), &input));
    waiting(&source, &message);
}

#[test]
fn a_closed_output_puts_the_messages_back() {
    let dir = tempfile::tempdir().unwrap();
    let source = Source::open(dir.path()).unwrap();
    let message = source.add("alice", "hi").unwrap();
    let mut child = start(dir.path());
    // The handler writes only after its input ends, so the output is closed by then.
    drop(child.stdout.take());
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(STOP.as_bytes()).unwrap();
    drop(stdin);
    failed(&child.wait_with_output().unwrap(), "agent-hook: ");
    waiting(&source, &message);
}

/// A failure exits with 1, never with 2, which on `Stop` would keep the turn going.
#[test]
fn fails_with_exit_code_1() {
    let dir = tempfile::tempdir().unwrap();
    let source = Source::open(dir.path()).unwrap();
    let message = source.add("alice", "hi").unwrap();
    let output = hook(dir.path(), "not JSON");
    failed(&output, "agent-hook: ");
    assert!(output.stdout.is_empty());
    waiting(&source, &message);

    let file = tempfile::NamedTempFile::new().unwrap();
    failed(
        &hook(file.path(), STOP),
        "agent-hook: cannot open the source ",
    );

    for args in [&[][..], &["a", "b"]] {
        let output = Command::new(env!("CARGO_BIN_EXE_agent-hook"))
            .args(args)
            .env_clear()
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "{args:?}");
        assert_eq!(stderr(&output), "usage: agent-hook <source-dir>\n");
    }
}
