//! `agent-hook` runs as each CLI runs it: a hook input recorded from the real CLI on stdin, the
//! handler's output on stdout.

use std::io::Write;
use std::path::Path;
use std::process::{Child, Command, Output, Stdio};

use agent_source::Source;
use agent_testing::{
    assert_carries_once, assert_failed, assert_handed_over, assert_waiting, stderr,
};
use serde_json::Value;

/// A CLI's hook inputs, recorded from it with the temporary directories' paths replaced, and the
/// handler's arguments before the source for it.
struct Cli {
    flags: &'static [&'static str],
    post_tool_use: &'static str,
    stop: &'static str,
    /// The `Stop` input after a `Stop` hook kept the turn going.
    stop_active: &'static str,
    /// Codex's only.
    user_prompt_submit: Option<&'static str>,
}

/// Claude Code 2.1.322.
const CLAUDE: Cli = Cli {
    flags: &[],
    post_tool_use: include_str!("inputs/claude-post-tool-use.json"),
    stop: include_str!("inputs/claude-stop.json"),
    stop_active: include_str!("inputs/claude-stop-active.json"),
    user_prompt_submit: None,
};

/// Codex 0.162.1.
const CODEX: Cli = Cli {
    flags: &["--codex"],
    post_tool_use: include_str!("inputs/codex-post-tool-use.json"),
    stop: include_str!("inputs/codex-stop.json"),
    stop_active: include_str!("inputs/codex-stop-active.json"),
    user_prompt_submit: Some(include_str!("inputs/codex-user-prompt-submit.json")),
};

impl Cli {
    fn is_codex(&self) -> bool {
        !self.flags.is_empty()
    }

    /// Every recorded input.
    fn inputs(&self) -> Vec<&'static str> {
        [self.post_tool_use, self.stop, self.stop_active]
            .into_iter()
            .chain(self.user_prompt_submit)
            .collect()
    }
}

/// Starts `agent-hook <flags> <source>` with piped stdio and no variable of the test's environment.
fn start(cli: &Cli, source: &Path) -> Child {
    Command::new(env!("CARGO_BIN_EXE_agent-hook"))
        .args(cli.flags)
        .arg(source)
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

/// Runs the handler for `cli` on `source` with `input` on stdin.
fn hook(cli: &Cli, source: &Path, input: &str) -> Output {
    let mut child = start(cli, source);
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

/// What a successful run of `cli`'s `event` hook gave the agent: the `additionalContext`, or on
/// Codex's `Stop` the reason of the decision that keeps the turn going.
fn given(cli: &Cli, output: &Output, event: &str) -> String {
    assert!(output.status.success(), "{}", stderr(output));
    let printed: Value = serde_json::from_slice(&output.stdout).unwrap();
    // Nothing that blocks a tool call or a prompt, or ends the session.
    assert_eq!(
        printed.as_object().unwrap().len(),
        1 + usize::from(cli.is_codex() && event == "Stop")
    );
    if cli.is_codex() && event == "Stop" {
        assert_eq!(printed["decision"], "block", "{printed}");
        return printed["reason"].as_str().unwrap().to_owned();
    }
    let specific = &printed["hookSpecificOutput"];
    assert_eq!(specific["hookEventName"], event);
    assert_eq!(specific.as_object().unwrap().len(), 2, "{printed}");
    specific["additionalContext"].as_str().unwrap().to_owned()
}

fn quiet(output: &Output) {
    assert!(output.status.success(), "{}", stderr(output));
    assert!(output.stdout.is_empty(), "{:?}", output.stdout);
}

/// Asserts that `cli`'s `event` hook, with `input`, adds each waiting message to the context once
/// and nothing once they are handed over.
fn adds_each_waiting_message_once(cli: &Cli, input: &str, event: &str) {
    let dir = tempfile::tempdir().unwrap();
    let source = Source::open(dir.path()).unwrap();
    let messages = [
        source.add("alice of skald", "first").unwrap(),
        source.add("bob", "second").unwrap(),
    ];
    assert_carries_once(&given(cli, &hook(cli, dir.path(), input), event), &messages);
    assert_handed_over(&source, &messages);
    // Handed over: the next run adds nothing.
    quiet(&hook(cli, dir.path(), input));

    let later = [source.add("carol", "third").unwrap()];
    let context = given(cli, &hook(cli, dir.path(), input), event);
    assert_carries_once(&context, &later);
    assert!(!context.contains(&messages[0].msg_id), "{context}");
}

#[test]
fn post_tool_use_adds_each_waiting_message_to_the_context_once() {
    for cli in [&CLAUDE, &CODEX] {
        adds_each_waiting_message_once(cli, cli.post_tool_use, "PostToolUse");
    }
}

#[test]
fn user_prompt_submit_on_codex_adds_each_waiting_message_to_the_context_once() {
    let input = CODEX.user_prompt_submit.unwrap();
    adds_each_waiting_message_once(&CODEX, input, "UserPromptSubmit");
}

#[test]
fn stop_keeps_the_turn_going_with_the_waiting_messages_and_then_lets_it_end() {
    for cli in [&CLAUDE, &CODEX] {
        let dir = tempfile::tempdir().unwrap();
        let source = Source::open(dir.path()).unwrap();
        let messages = [source.add("alice", "the token").unwrap()];
        // The output keeps the turn going: the agent gets the messages.
        assert_carries_once(
            &given(cli, &hook(cli, dir.path(), cli.stop), "Stop"),
            &messages,
        );
        assert_handed_over(&source, &messages);
        // The turn went on; with none waiting, the next Stop lets it end.
        quiet(&hook(cli, dir.path(), cli.stop_active));

        // A message that arrives during the kept turn keeps it going again.
        let more = [source.add("bob", "more").unwrap()];
        let output = hook(cli, dir.path(), cli.stop_active);
        assert_carries_once(&given(cli, &output, "Stop"), &more);
    }
}

#[test]
fn with_none_waiting_it_adds_nothing_and_stop_lets_the_turn_end() {
    for cli in [&CLAUDE, &CODEX] {
        let dir = tempfile::tempdir().unwrap();
        for input in cli.inputs() {
            quiet(&hook(cli, dir.path(), input));
        }
    }
}

#[test]
fn another_event_takes_nothing() {
    let pre_tool_use = |cli: &Cli| {
        cli.post_tool_use
            .replace("\"PostToolUse\"", "\"PreToolUse\"")
    };
    // Claude Code's handler does not answer UserPromptSubmit.
    let user_prompt_submit = CODEX.user_prompt_submit.unwrap().to_owned();
    for (cli, input) in [
        (&CLAUDE, pre_tool_use(&CLAUDE)),
        (&CODEX, pre_tool_use(&CODEX)),
        (&CLAUDE, user_prompt_submit),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let source = Source::open(dir.path()).unwrap();
        let message = source.add("alice", "hi").unwrap();
        quiet(&hook(cli, dir.path(), &input));
        assert_waiting(&source, &[message]);
    }
}

#[test]
fn a_closed_output_puts_the_messages_back() {
    for cli in [&CLAUDE, &CODEX] {
        let dir = tempfile::tempdir().unwrap();
        let source = Source::open(dir.path()).unwrap();
        let message = source.add("alice", "hi").unwrap();
        let mut child = start(cli, dir.path());
        // The handler writes only after its input ends, so the output is closed by then.
        drop(child.stdout.take());
        let mut stdin = child.stdin.take().unwrap();
        stdin.write_all(cli.stop.as_bytes()).unwrap();
        drop(stdin);
        assert_failed(&child.wait_with_output().unwrap(), "agent-hook: ");
        assert_waiting(&source, &[message]);
    }
}

/// A failure exits with 1, never with 2, which on `Stop` would keep the turn going.
#[test]
fn fails_with_exit_code_1() {
    for cli in [&CLAUDE, &CODEX] {
        let dir = tempfile::tempdir().unwrap();
        let source = Source::open(dir.path()).unwrap();
        let message = source.add("alice", "hi").unwrap();
        let output = hook(cli, dir.path(), "not JSON");
        assert_failed(&output, "agent-hook: ");
        assert!(output.stdout.is_empty());
        assert_waiting(&source, &[message]);

        let file = tempfile::NamedTempFile::new().unwrap();
        assert_failed(
            &hook(cli, file.path(), cli.stop),
            "agent-hook: cannot open the source ",
        );
    }

    for args in [
        &[][..],
        &["a", "b"],
        &["--codex"],
        &["--codex", "a", "b"],
        &["--claude", "a"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_agent-hook"))
            .args(args)
            .env_clear()
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "{args:?}");
        assert_eq!(
            stderr(&output),
            "usage: agent-hook [--codex] <source-dir>\n"
        );
    }
}
