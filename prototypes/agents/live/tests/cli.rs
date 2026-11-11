//! `live-check`'s commands, without running an agent CLI.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Output};

use agent_live::Agent;
use agent_live::channel::stub_environment;
use agent_live::queue;
use agent_source::{Source, State};

/// Runs `live-check` with no variable of the test's environment, so none of its credentials
/// reach the output.
fn live_check(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_live-check"))
        .args(args)
        .env_clear()
        .output()
        .expect("live-check runs")
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Asserts that `live-check` failed with exit code 1 and said why.
fn assert_failed(output: &Output) {
    assert_eq!(output.status.code(), Some(1), "{}", stderr(output));
    assert!(
        stderr(output).starts_with("live-check: "),
        "{}",
        stderr(output)
    );
}

#[test]
fn add_puts_a_waiting_message_in_the_source() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    let source = source.to_str().unwrap();
    let output = live_check(&["add", source, "a sender", "a text"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let msg_id = String::from_utf8(output.stdout).unwrap();
    let msg_id = msg_id.trim();
    let source = Source::open(source).unwrap();
    assert_eq!(source.state(msg_id).unwrap(), Some(State::Waiting));
    let taken = source.take().unwrap();
    assert_eq!(taken.len(), 1);
    assert_eq!(taken[0].msg_id, msg_id);
    assert_eq!(taken[0].sender, "a sender");
    assert_eq!(taken[0].text, "a text");
}

#[test]
fn add_fails_when_the_source_cannot_open() {
    let file = tempfile::NamedTempFile::new().unwrap();
    assert_failed(&live_check(&[
        "add",
        file.path().to_str().unwrap(),
        "s",
        "t",
    ]));
}

#[test]
fn wrong_arguments_print_the_usage() {
    for args in [
        &[][..],
        &["add", "dir", "sender"],
        &["inbox"],
        &["inbox", "gemini"],
        &["inbox", "claude", "--model"],
        &["inbox", "codex", "--verbose"],
        &["inbox", "codex", "--all", "--real", "--stub"],
        &["channel"],
        &["channel", "codex"],
        &["channel", "claude", "--verbose"],
        &["stop"],
        &["stop", "gemini"],
        &["post-tool-use", "claude", "--verbose"],
        &["pre-tool-use", "claude"],
        &["user-prompt-submit", "claude"],
        &["user-prompt-submit", "codex", "--verbose"],
        &["queue"],
        &["queue", "claude"],
        &["queue", "codex", "--verbose"],
        &["timed-hook"],
        &["timed-hook", "log"],
    ] {
        let output = live_check(args);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert!(stderr(&output).starts_with("usage: live-check"), "{args:?}");
    }
}

#[test]
fn inbox_fails_without_the_cli() {
    let empty = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_live-check"))
        .args(["inbox", "claude", "--all", "--real", "--keep"])
        .env_clear()
        .env("PATH", empty.path())
        .output()
        .expect("live-check runs");
    assert_failed(&output);
}

/// A variable of `live-check`'s environment that the CLI on the stub must not get.
const SENTINEL: &str = "SKALD_LIVE_TEST_CREDENTIAL";

/// Variables, with fake values, that would sign the CLIs in other than with the owner's own sign-in:
/// off the stub, each CLI must not get its own.
const SIGN_IN: [(&str, &str); 9] = [
    ("ANTHROPIC_API_KEY", "sk-ant-fake"),
    ("ANTHROPIC_AUTH_TOKEN", "fake-token"),
    ("ANTHROPIC_BASE_URL", "http://127.0.0.1:1"),
    ("CLAUDE_CODE_OAUTH_TOKEN", "fake-oauth-token"),
    ("CLAUDE_CODE_USE_BEDROCK", "1"),
    ("AWS_BEARER_TOKEN_BEDROCK", "fake-bearer"),
    ("OPENAI_API_KEY", "sk-fake"),
    ("OPENAI_BASE_URL", "http://127.0.0.1:1"),
    ("CODEX_API_KEY", "fake-codex-key"),
];

/// Asserts that a session of `agent` off the stub got `live-check`'s `PATH` and [`SENTINEL`],
/// none of the variables of [`SIGN_IN`] that would sign it in and no config home of its own.
fn assert_real_environment(agent: Agent, at: &str, session: &Recorded, path: &str) {
    assert!(session.env["PATH"] == path, "{at}: PATH differs");
    assert!(
        session
            .env
            .get(SENTINEL)
            .is_some_and(|value| value == "secret"),
        "{at}: {SENTINEL} did not pass on"
    );
    for (name, _) in SIGN_IN {
        let set = session.env.contains_key(name);
        if agent.replaces_sign_in(name) {
            assert!(!set, "{at}: {name} is set");
        } else {
            assert!(set, "{at}: {name} did not pass on");
        }
    }
    for config in ["CLAUDE_CONFIG_DIR", "CODEX_HOME"] {
        assert!(!session.env.contains_key(config), "{at}: {config} is set");
    }
}

/// Asserts that a session on the stub got only the environment `expected` gives for its `HOME`,
/// which lies in `tmp`, and not [`SENTINEL`].
fn assert_stub_environment(
    at: &str,
    session: &Recorded,
    tmp: &Path,
    expected: impl FnOnce(&Path) -> io::Result<Vec<(String, String)>>,
) {
    assert!(
        !session.env.contains_key(SENTINEL),
        "{at}: {SENTINEL} reached the CLI"
    );
    let home = Path::new(&session.env["HOME"]);
    assert!(home.starts_with(tmp), "{at}: HOME is not in TMPDIR");
    let expected: BTreeMap<String, String> = expected(home).unwrap().into_iter().collect();
    assert_eq!(
        session.env.keys().collect::<Vec<_>>(),
        expected.keys().collect::<Vec<_>>(),
        "{at}: the CLI's variables"
    );
    for (name, value) in &expected {
        assert!(session.env[name] == *value, "{at}: {name} differs");
    }
}

/// Variables the stand-ins' shell sets itself.
const SHELL_VARS: [&str; 3] = ["PWD", "SHLVL", "_"];

/// What a stand-in CLI recorded in one session: its arguments and environment.
struct Recorded {
    args: Vec<String>,
    env: BTreeMap<String, String>,
}

/// Puts stand-ins for `claude` and `codex` in `bin`: they answer `--version` and otherwise record
/// their arguments and environment in `record/<cli>-<pid>` and end the session without output.
fn stand_ins(bin: &Path, record: &Path) {
    for cli in ["claude", "codex"] {
        let script = format!(
            "#!/bin/sh\n\
             if [ \"$1\" = --version ]; then echo 0.0.0; exit 0; fi\n\
             dir='{}/{cli}-'$$\n\
             mkdir \"$dir\"\n\
             printf '%s\\n' \"$@\" >\"$dir/args\"\n\
             env >\"$dir/env\"\n",
            record.display()
        );
        let path = bin.join(cli);
        fs::write(&path, script).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

/// The sessions the stand-in `cli` recorded.
fn recorded(record: &Path, cli: &str) -> Vec<Recorded> {
    let mut sessions = Vec::new();
    for entry in fs::read_dir(record).unwrap() {
        let dir = entry.unwrap().path();
        let name = dir.file_name().unwrap().to_str().unwrap().to_owned();
        if !name.starts_with(&format!("{cli}-")) {
            continue;
        }
        let args = fs::read_to_string(dir.join("args")).unwrap();
        let env = fs::read_to_string(dir.join("env")).unwrap();
        sessions.push(Recorded {
            args: args.lines().map(String::from).collect(),
            env: env
                .lines()
                .filter_map(|line| line.split_once('='))
                .filter(|(name, _)| !SHELL_VARS.contains(name))
                .map(|(name, value)| (name.to_owned(), value.to_owned()))
                .collect(),
        });
    }
    sessions
}

/// The stub's address in a session's arguments or environment.
fn stub_url(agent: Agent, session: &Recorded) -> String {
    match agent {
        Agent::Claude => session.env["ANTHROPIC_BASE_URL"].clone(),
        Agent::Codex => {
            let arg = session
                .args
                .iter()
                .find_map(|arg| arg.strip_prefix("model_providers.stub.base_url="))
                .expect("the stub provider's address");
            serde_json::from_str::<String>(arg)
                .unwrap()
                .strip_suffix("/v1")
                .unwrap()
                .to_owned()
        }
    }
}

#[test]
fn inbox_runs_one_session_or_all_three_on_the_stub_with_only_its_environment() {
    for agent in [Agent::Claude, Agent::Codex] {
        for (flags, sessions) in [(&[][..], 1), (&["--all"][..], 3)] {
            let (recorded, path, dir) = sessions_of(agent, &["inbox", agent.cli()], flags);
            // A failure names the variables, never their values.
            let at = format!("{} {flags:?}", agent.cli());
            assert_eq!(recorded.len(), sessions, "{at}");
            for session in &recorded {
                assert_stub_environment(&at, session, &dir.path().join("tmp"), |home| {
                    agent.environment(home, &stub_url(agent, session), &path)
                });
            }
        }
    }
}

#[test]
fn inbox_on_the_real_model_drops_the_variables_that_replace_the_sign_in() {
    for agent in [Agent::Claude, Agent::Codex] {
        for (flags, sessions) in [(&["--real"][..], 1), (&["--real", "--all"][..], 3)] {
            let (recorded, path, _dir) = sessions_of(agent, &["inbox", agent.cli()], flags);
            let at = format!("{} {flags:?}", agent.cli());
            assert_eq!(recorded.len(), sessions, "{at}");
            for session in &recorded {
                assert_real_environment(agent, &at, session, &path);
            }
        }
    }
}

/// Runs `live-check channel claude <flags>` with stand-in CLIs and returns the sessions the
/// stand-in `claude` recorded and the `PATH` it ran with. Asserts that there are `sessions` and
/// all but the fourth load the channel.
fn channel_sessions(
    flags: &[&str],
    sessions: usize,
) -> (Vec<Recorded>, String, tempfile::TempDir, String) {
    let (recorded, path, dir) = sessions_of(Agent::Claude, &["channel", "claude"], flags);
    let at = format!("{flags:?}");
    assert_eq!(recorded.len(), sessions, "{at}");
    let with_channel = recorded
        .iter()
        .filter(|session| session.args.iter().any(|arg| arg == CHANNEL_FLAG))
        .count();
    assert_eq!(
        with_channel,
        sessions.min(3),
        "{at}: sessions that load the channel"
    );
    (recorded, path, dir, at)
}

/// Runs `live-check <command> <flags>` with stand-in CLIs, [`SENTINEL`] and [`SIGN_IN`] and no
/// other variable of the test's in its environment, and returns the sessions the stand-in of
/// `agent` recorded and the `PATH` it ran with.
fn sessions_of(
    agent: Agent,
    command: &[&str],
    flags: &[&str],
) -> (Vec<Recorded>, String, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let (bin, record, tmp) = (
        dir.path().join("bin"),
        dir.path().join("record"),
        dir.path().join("tmp"),
    );
    for dir in [&bin, &record, &tmp] {
        fs::create_dir(dir).unwrap();
    }
    stand_ins(&bin, &record);
    let path = format!("{}:/usr/bin:/bin", bin.display());
    let output = Command::new(env!("CARGO_BIN_EXE_live-check"))
        .args(command)
        .args(flags)
        .env_clear()
        .env("PATH", &path)
        .env("TMPDIR", &tmp)
        .env(SENTINEL, "secret")
        .envs(SIGN_IN)
        .output()
        .expect("live-check runs");
    // The stand-in ends each session at once, so the check fails.
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    (recorded(&record, agent.cli()), path, dir)
}

const CHANNEL_FLAG: &str = "--dangerously-load-development-channels";

#[test]
fn channel_runs_one_session_or_all_four_on_the_stub_with_only_its_environment() {
    for (flags, sessions) in [(&[][..], 1), (&["--all"][..], 4)] {
        let (recorded, path, dir, at) = channel_sessions(flags, sessions);
        for session in &recorded {
            assert_stub_environment(&at, session, &dir.path().join("tmp"), |home| {
                stub_environment(home, &stub_url(Agent::Claude, session), &path)
            });
        }
    }
}

#[test]
fn channel_on_the_real_model_drops_the_sign_in_variables_and_adds_the_terminal_and_child_mark() {
    for (flags, sessions) in [(&["--real"][..], 1), (&["--real", "--all"][..], 4)] {
        let (recorded, path, _dir, at) = channel_sessions(flags, sessions);
        for session in &recorded {
            // The real model signs in from the user's config home: the rest of the check's
            // environment passes on.
            assert_real_environment(Agent::Claude, &at, session, &path);
            assert!(
                session.env["TERM"] == "xterm-256color",
                "{at}: TERM differs"
            );
            assert!(
                session.env["CLAUDE_CODE_CHILD_SESSION"] == "1",
                "{at}: CLAUDE_CODE_CHILD_SESSION differs"
            );
        }
    }
}

/// The hook command and source of a Claude Code hook session, from the settings file in its
/// arguments.
fn hook_of(session: &Recorded, event: &str) -> (String, String) {
    let at = session
        .args
        .iter()
        .position(|arg| arg == "--settings")
        .expect("a settings file");
    let settings: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&session.args[at + 1]).unwrap()).unwrap();
    let hooks = settings["hooks"].as_object().unwrap();
    assert_eq!(
        hooks.keys().collect::<Vec<_>>(),
        [event],
        "only the hook under check"
    );
    let handler = &hooks[event][0]["hooks"][0];
    (
        handler["command"].as_str().unwrap().to_owned(),
        handler["args"][0].as_str().unwrap().to_owned(),
    )
}

/// The words of the hook command of a Codex hook session, from the `-c hooks.<event>=` value in its
/// arguments; the command quotes each word.
fn codex_hook_of(session: &Recorded, event: &str) -> Vec<String> {
    let hooks: Vec<&String> = session
        .args
        .iter()
        .filter(|arg| arg.starts_with("hooks."))
        .collect();
    assert_eq!(hooks.len(), 1, "only the hook under check: {hooks:?}");
    let prefix = format!("hooks.{event}=[{{hooks=[{{type=\"command\",command=");
    let command = hooks[0]
        .strip_prefix(&prefix)
        .and_then(|rest| rest.strip_suffix("}]}]"))
        .expect("one command hook");
    let command: String = serde_json::from_str(command).unwrap();
    command
        .split(' ')
        .map(|word| word.trim_matches('\'').to_owned())
        .collect()
}

/// The checks of each hook on each agent: `live-check`'s arguments, the hook event and how many
/// sessions run without and with `--all`.
const HOOKS: [(&str, Agent, &str, usize, usize); 5] = [
    ("post-tool-use", Agent::Claude, "PostToolUse", 1, 3),
    ("stop", Agent::Claude, "Stop", 1, 3),
    ("post-tool-use", Agent::Codex, "PostToolUse", 1, 3),
    ("stop", Agent::Codex, "Stop", 1, 3),
    ("user-prompt-submit", Agent::Codex, "UserPromptSubmit", 1, 2),
];

#[test]
fn hooks_run_one_session_or_all_on_the_stub_with_only_its_environment() {
    let live_check = Path::new(env!("CARGO_BIN_EXE_live-check"));
    let handler = live_check.with_file_name("agent-hook");
    for (hook, agent, event, one, all) in HOOKS {
        for (flags, sessions) in [(&[][..], one), (&["--all"][..], all)] {
            let (recorded, path, dir) = sessions_of(agent, &[hook, agent.cli()], flags);
            let at = format!("{hook} {} {flags:?}", agent.cli());
            assert_eq!(recorded.len(), sessions, "{at}");
            let mut sources = Vec::new();
            for session in &recorded {
                assert_stub_environment(&at, session, &dir.path().join("tmp"), |home| {
                    agent.environment(home, &stub_url(agent, session), &path)
                });
                let source = match agent {
                    Agent::Claude => {
                        let (command, source) = hook_of(session, event);
                        assert_eq!(Path::new(&command), handler, "{at}");
                        source
                    }
                    Agent::Codex => {
                        let words = codex_hook_of(session, event);
                        assert_eq!(words.len(), 6, "{at}: {words:?}");
                        assert_eq!(Path::new(&words[0]), live_check, "{at}");
                        assert_eq!(words[1], "timed-hook", "{at}");
                        assert!(words[2].ends_with("/hook-runs.jsonl"), "{at}: {words:?}");
                        assert_eq!(Path::new(&words[3]), handler, "{at}");
                        assert_eq!(words[4], "--codex", "{at}");
                        assert!(
                            session
                                .args
                                .contains(&"--dangerously-bypass-hook-trust".to_owned()),
                            "{at}"
                        );
                        words[5].clone()
                    }
                };
                sources.push(source);
            }
            sources.sort();
            sources.dedup();
            assert_eq!(sources.len(), sessions, "{at}: a source per session");
        }
    }
}

#[test]
fn hooks_on_the_real_model_drop_the_variables_that_replace_the_sign_in() {
    for (hook, agent, _, one, all) in HOOKS {
        for (flags, sessions) in [(&["--real"][..], one), (&["--real", "--all"][..], all)] {
            let (recorded, path, _dir) = sessions_of(agent, &[hook, agent.cli()], flags);
            let at = format!("{hook} {} {flags:?}", agent.cli());
            assert_eq!(recorded.len(), sessions, "{at}");
            for session in &recorded {
                assert_real_environment(agent, &at, session, &path);
            }
        }
    }
}

#[test]
fn timed_hook_runs_the_handler_and_records_its_run() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("hook-runs.jsonl");
    let mut child = Command::new(env!("CARGO_BIN_EXE_live-check"))
        .args([
            "timed-hook",
            log.to_str().unwrap(),
            "/bin/sh",
            "-c",
            "cat; exit 3",
        ])
        .env_clear()
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let input = r#"{"hook_event_name":"Stop","session_id":"s1"}"#;
    std::io::Write::write_all(&mut child.stdin.take().unwrap(), input.as_bytes()).unwrap();
    let output = child.wait_with_output().unwrap();
    // The handler's exit code and stdout pass on.
    assert_eq!(output.status.code(), Some(3));
    assert_eq!(String::from_utf8_lossy(&output.stdout), input);
    let run: serde_json::Value =
        serde_json::from_str(fs::read_to_string(&log).unwrap().trim()).unwrap();
    assert_eq!(run["hook_event_name"], "Stop");
    assert_eq!(run["outcome"], "error");
    assert_eq!(run["stdout"], input);
    assert_eq!(run["input"]["session_id"], "s1");

    // A log it cannot write fails the hook with exit code 1, never 2.
    let output = Command::new(env!("CARGO_BIN_EXE_live-check"))
        .args(["timed-hook", "/nonexistent/log", "/bin/sh", "-c", "exit 0"])
        .env_clear()
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    assert_failed(&output);
}

/// Runs `live-check queue codex <flags>` with stand-in CLIs and returns the sessions the stand-in
/// `codex` recorded, after asserting that there are `sessions`, each an interactive session that
/// runs its own app-server and records its hooks.
fn queue_sessions(flags: &[&str], sessions: usize) -> (Vec<Recorded>, String, tempfile::TempDir) {
    let (recorded, path, dir) = sessions_of(Agent::Codex, &["queue", "codex"], flags);
    let at = format!("{flags:?}");
    assert_eq!(recorded.len(), sessions, "{at}");
    for session in &recorded {
        assert_eq!(session.args[0], "--no-daemon", "{at}");
        for event in ["Stop", "PreToolUse"] {
            assert!(
                session
                    .args
                    .iter()
                    .any(|arg| arg.starts_with(&format!("hooks.{event}="))),
                "{at}: no {event} hook"
            );
        }
    }
    (recorded, path, dir)
}

#[test]
fn queue_runs_one_session_or_all_three_on_the_stub_with_only_its_environment() {
    for (flags, sessions) in [(&[][..], 1), (&["--all"][..], 3)] {
        let (recorded, path, dir) = queue_sessions(flags, sessions);
        let at = format!("{flags:?}");
        for session in &recorded {
            assert_stub_environment(&at, session, &dir.path().join("tmp"), |home| {
                queue::stub_environment(home, &stub_url(Agent::Codex, session), &path)
            });
        }
    }
}

#[test]
fn queue_on_the_real_model_drops_the_variables_that_replace_the_sign_in() {
    for (flags, sessions) in [(&["--real"][..], 1), (&["--real", "--all"][..], 3)] {
        let (recorded, path, _dir) = queue_sessions(flags, sessions);
        let at = format!("{flags:?}");
        for session in &recorded {
            assert_real_environment(Agent::Codex, &at, session, &path);
            assert!(
                session.env["TERM"] == "xterm-256color",
                "{at}: TERM differs"
            );
        }
    }
}
