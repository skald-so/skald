//! `agent-hook` runs as each CLI runs it: a hook input recorded from the real CLI on stdin, the
//! handler's output on stdout.

use std::ffi::OsStr;
use std::fs;
use std::io::{ErrorKind, Write};
use std::path::Path;
use std::process::{Child, Command, Output, Stdio};

use agent_source::Source;
use agent_testing::{
    assert_carries_once, assert_failed, assert_handed_over, assert_waiting, one_waiting, stderr,
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

/// Each CLI's `SessionStart` input, recorded as the others.
const CLAUDE_SESSION_START: &str = include_str!("inputs/claude-session-start.json");
const CODEX_SESSION_START: &str = include_str!("inputs/codex-session-start.json");

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

/// Starts `agent-hook <args>` with piped stdio and only `env` of the test's environment.
fn spawn(args: &[&OsStr], env: &[(&str, &str)]) -> Child {
    Command::new(env!("CARGO_BIN_EXE_agent-hook"))
        .args(args)
        .env_clear()
        .envs(env.iter().copied())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

/// Starts `agent-hook <flags> <source>` with piped stdio and no variable of the test's environment.
fn start(cli: &Cli, source: &Path) -> Child {
    let mut args: Vec<&OsStr> = cli.flags.iter().map(OsStr::new).collect();
    args.push(source.as_os_str());
    spawn(&args, &[])
}

/// Writes `input` to `child`'s stdin and waits for its output.
fn finish(mut child: Child, input: &str) -> Output {
    // A handler that fails before reading its input, such as on a source it cannot open, may close
    // stdin first.
    if let Err(error) = child.stdin.take().unwrap().write_all(input.as_bytes()) {
        assert_eq!(error.kind(), ErrorKind::BrokenPipe, "{error}");
    }
    child.wait_with_output().unwrap()
}

/// Runs the handler for `cli` on `source` with `input` on stdin.
fn hook(cli: &Cli, source: &Path, input: &str) -> Output {
    finish(start(cli, source), input)
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
        let (source, message) = one_waiting(dir.path());
        quiet(&hook(cli, dir.path(), &input));
        assert_waiting(&source, &[message]);
    }
}

#[test]
fn a_closed_output_puts_the_messages_back() {
    for cli in [&CLAUDE, &CODEX] {
        let dir = tempfile::tempdir().unwrap();
        let (source, message) = one_waiting(dir.path());
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
        let (source, message) = one_waiting(dir.path());
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
        &["--wait", "a"],
        &["--wait", "a", "soon"],
        &["--take"],
        &["--count", "a", "b"],
        &["--release", "a"],
    ] {
        let output = run(args, "", &[]);
        assert_eq!(output.status.code(), Some(1), "{args:?}");
        assert!(
            stderr(&output).starts_with("usage: agent-hook [--codex] <source-dir>\n"),
            "{args:?}: {}",
            stderr(&output)
        );
    }
}

/// Runs `agent-hook <args>` with `input` on stdin and only `env` in its environment.
fn run(args: &[&str], input: &str, env: &[(&str, &str)]) -> Output {
    let args: Vec<&OsStr> = args.iter().map(OsStr::new).collect();
    finish(spawn(&args, env), input)
}

#[test]
fn session_start_registers_the_session_and_its_inbox_socket() {
    let messaging = [
        ("CLAUDE_CODE_MESSAGING_SOCKET", "/tmp/cc-socks/35442.sock"),
        ("CLAUDE_CODE_MESSAGING_TOKEN", "child-token"),
    ];
    for (cli, input, env) in [
        (&CLAUDE, CLAUDE_SESSION_START, &messaging[..]),
        (&CODEX, CODEX_SESSION_START, &[]),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let (source, message) = one_waiting(dir.path());
        let mut args: Vec<&OsStr> = cli.flags.iter().map(OsStr::new).collect();
        args.push(dir.path().as_os_str());
        let output = finish(spawn(&args, env), input);
        quiet(&output);
        // It hands nothing over.
        assert_waiting(&source, &[message]);
        let registered = source.registration().unwrap().unwrap();
        let input: Value = serde_json::from_str(input).unwrap();
        assert_eq!(registered.session_id, input["session_id"].as_str().unwrap());
        assert_eq!(
            registered.transcript_path.as_deref(),
            input["transcript_path"].as_str()
        );
        assert_eq!(registered.cwd.as_deref(), input["cwd"].as_str());
        let socket = env.first().map(|(_, value)| (*value).to_owned());
        let token = env.get(1).map(|(_, value)| (*value).to_owned());
        assert_eq!((registered.socket, registered.token), (socket, token));
    }
    let dir = tempfile::tempdir().unwrap();
    let output = run(
        &[dir.path().to_str().unwrap()],
        r#"{"hook_event_name":"SessionStart"}"#,
        &[],
    );
    assert_failed(&output, "agent-hook: no session_id");
}

/// An input of `cli`'s `event` whose transcript is `transcript`.
fn with_transcript(input: &str, transcript: &Path) -> String {
    let mut input: Value = serde_json::from_str(input).unwrap();
    input["transcript_path"] = transcript.to_str().unwrap().into();
    input.to_string()
}

/// A session file of `cli` in which `msg_id` went to the model.
fn transcript_line(cli: &Cli, msg_id: &str) -> String {
    if cli.is_codex() {
        format!(
            r#"{{"timestamp":"2026-11-12T07:52:11.632Z","type":"response_item","payload":{{"type":"message","role":"developer","content":[{{"type":"input_text","text":"<message msg_id=\"{msg_id}\">"}}]}}}}"#
        )
    } else {
        format!(
            r#"{{"type":"attachment","timestamp":"2026-11-12T07:49:14.100Z","attachment":{{"type":"hook_additional_context","content":["<message msg_id=\"{msg_id}\">"]}}}}"#
        )
    }
}

fn signals(source: &Source, msg_id: &str) -> Vec<String> {
    source
        .records(msg_id)
        .unwrap()
        .into_iter()
        .map(|record| record.signal)
        .collect()
}

#[test]
fn confirms_the_messages_the_session_file_shows_and_drops_their_repeats() {
    for cli in [&CLAUDE, &CODEX] {
        let dir = tempfile::tempdir().unwrap();
        let source = Source::open(dir.path().join("source")).unwrap();
        let transcript = dir.path().join("session.jsonl");
        // Handed over earlier, and in the transcript: confirmed.
        let shown = source.add("alice", "shown").unwrap();
        // Handed over, but not in the transcript yet.
        let unseen = source.add("bob", "unseen").unwrap();
        let taken = source.take().unwrap();
        let ids: Vec<String> = taken.iter().map(|m| m.msg_id.clone()).collect();
        source.handed_over(&ids).unwrap();
        // Waiting again, resent by another path, but already in the transcript: a repeat.
        let repeat = source.add("carol", "repeat").unwrap();
        // Waiting and new: handed over.
        let fresh = source.add("dave", "fresh").unwrap();
        fs::write(
            &transcript,
            [&shown.msg_id, &repeat.msg_id]
                .map(|id| transcript_line(cli, id))
                .join("\n"),
        )
        .unwrap();
        let input = with_transcript(cli.post_tool_use, &transcript);
        let context = given(cli, &hook(cli, &dir.path().join("source"), &input), "PostToolUse");
        assert_carries_once(&context, std::slice::from_ref(&fresh));
        assert!(!context.contains(&repeat.msg_id), "{context}");
        assert_handed_over(&source, &[fresh.clone(), repeat.clone()]);
        assert_eq!(signals(&source, &shown.msg_id), ["transcript"]);
        assert!(signals(&source, &unseen.msg_id).is_empty());
        assert_eq!(signals(&source, &repeat.msg_id), ["transcript", "repeat-dropped"]);
        assert!(signals(&source, &fresh.msg_id).is_empty());

        // Only a repeat waiting: nothing is printed, so Stop lets the turn end.
        let again = source.add("erin", "again").unwrap();
        let mut file = fs::OpenOptions::new().append(true).open(&transcript).unwrap();
        writeln!(file, "\n{}", transcript_line(cli, &again.msg_id)).unwrap();
        let input = with_transcript(cli.stop, &transcript);
        quiet(&hook(cli, &dir.path().join("source"), &input));
        assert_handed_over(&source, &[again]);
    }
}

/// A temporary directory, its path, and the source in it.
fn opened() -> (tempfile::TempDir, String, Source) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_string_lossy().into_owned();
    (dir, path.clone(), Source::open(path).unwrap())
}

#[test]
fn takes_counts_and_releases_messages_for_the_mod() {
    let (_dir, path, source) = opened();
    let path = path.as_str();
    let count = |expected: &str| {
        let output = run(&["--count", path], "", &[]);
        assert!(output.status.success(), "{}", stderr(&output));
        assert_eq!(String::from_utf8_lossy(&output.stdout), expected);
    };
    count("0\n");
    let take = run(&["--take", path], "", &[]);
    quiet(&take);

    let messages = [source.add("alice", "one").unwrap(), source.add("bob", "two").unwrap()];
    count("2\n");
    let take = run(&["--take", path], "", &[]);
    assert!(take.status.success(), "{}", stderr(&take));
    let printed: Value = serde_json::from_slice(&take.stdout).unwrap();
    let ids: Vec<&str> = messages.iter().map(|m| m.msg_id.as_str()).collect();
    assert_eq!(printed["ids"], serde_json::json!(ids));
    assert_carries_once(printed["text"].as_str().unwrap(), &messages);
    assert_handed_over(&source, &messages);
    count("0\n");

    let release = run(&["--release", path, ids[0], ids[1]], "", &[]);
    quiet(&release);
    assert_waiting(&source, &messages);
    assert_failed(&run(&["--release", path, ids[0], "nope"], "", &[]), "agent-hook: ");
}

#[test]
fn wait_hands_a_message_over_on_stderr_and_exits_2_to_wake_the_session() {
    let (_dir, path, source) = opened();
    let path = path.as_str();
    // Nothing comes: the wait ends with exit code 0.
    let started = std::time::Instant::now();
    let idle = run(&["--wait", path, "1"], CLAUDE.stop, &[]);
    assert!(started.elapsed() >= std::time::Duration::from_secs(1));
    quiet(&idle);
    assert!(idle.stderr.is_empty());

    let mut waiter = spawn(&[OsStr::new("--wait"), OsStr::new(path), OsStr::new("30")], &[]);
    drop(waiter.stdin.take());
    // A waiter that finds the wait taken leaves at once; this one stays: it holds the wait.
    std::thread::sleep(std::time::Duration::from_millis(500));
    assert!(waiter.try_wait().unwrap().is_none(), "the waiter left");
    let started = std::time::Instant::now();
    let second = run(&["--wait", path, "5"], "", &[]);
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
    quiet(&second);
    let message = source.add("alice", "the token").unwrap();
    let woke = waiter.wait_with_output().unwrap();
    assert_eq!(woke.status.code(), Some(2), "{}", stderr(&woke));
    assert!(woke.stdout.is_empty());
    assert_carries_once(&stderr(&woke), std::slice::from_ref(&message));
    assert_handed_over(&source, &[message]);
}
