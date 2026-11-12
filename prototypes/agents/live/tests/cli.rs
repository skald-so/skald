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
use agent_testing::{Exchange, assert_failed, stderr};

/// Runs `live-check` with no variable of the test's environment, so none of its credentials
/// reach the output.
fn live_check(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_live-check"))
        .args(args)
        .env_clear()
        .output()
        .expect("live-check runs")
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
    assert_failed(
        &live_check(&["add", file.path().to_str().unwrap(), "s", "t"]),
        "live-check: ",
    );
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
        &["app-server"],
        &["app-server", "claude"],
        &["app-server", "codex", "--verbose"],
        &["timed-hook"],
        &["timed-hook", "log"],
        &["limits"],
        &["limits", "gemini"],
        &["limits", "claude", "--all"],
        &["limits", "codex", "--real"],
        &["limits", "claude", "--model", "m"],
        &["status-line"],
        &["status-line", "codex"],
        &["status-line", "claude", "--all"],
        &["status-line", "claude", "--real"],
        &["gui"],
        &["gui", "inbox"],
        &["gui", "inbox", "claude"],
        &["gui", "queue", "claude-vscode"],
        &["gui", "app-server", "codex-desktop"],
        &["gui", "channel", "codex-desktop"],
        &["gui", "user-prompt-submit", "claude-desktop"],
        &["gui", "user-prompt-submit", "codex-vscode", "--busy"],
        &["gui", "inbox", "claude-vscode", "--keep"],
        &["gui", "inbox", "claude-vscode", "--busy", "--busy"],
        &["gui", "channel", "claude-vscode", "--busy"],
        &["gui", "inbox", "codex-vscode", "--busy"],
        &["gui-add"],
        &["gui-add", "dir", "token"],
        &["gui-result"],
        &["gui-result", "dir"],
        &["gui-link"],
        &["gui-link", "dir", "--open"],
        &["gui-queue"],
        &["gui-queue", "dir", "--bundled"],
        &["gui-queue", "dir", "--cli", "--cli"],
        &["socket"],
        &["socket", "codex"],
        &["rewake", "claude", "--verbose"],
        &["mod", "codex"],
        &["loop", "claude", "--verbose"],
        &["gui", "loop", "codex-desktop"],
        &["ack"],
        &["ack", "gemini"],
        &["ack", "codex", "--verbose"],
        &["transcript", "claude", "--stub"],
        &["resend", "claude", "--real"],
        &["resend", "codex"],
        &["gui", "socket", "codex-desktop"],
        &["gui", "mod", "codex-vscode"],
        &["gui", "read", "claude-vscode"],
        &["gui", "socket", "claude-vscode", "--busy", "--bypass"],
        &["gui", "rewake", "claude-desktop", "--bypass"],
        &["gui", "read", "codex-desktop", "--bypass"],
        &["gui-socket"],
        &["gui-socket", "dir", "--auth"],
        &["gui-socket", "dir", "--listen"],
        &["gui-socket", "dir", "--listen", "soon"],
        &["gui-socket", "dir", "--listen", "1", "--no-auth"],
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
    assert_failed(&output, "live-check: ");
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

/// The sessions of one run of `live-check`, as the stand-in CLI recorded them.
struct Checked {
    sessions: Vec<Recorded>,
    /// The run in failure messages.
    at: String,
    /// The run's temporary directory, which holds the sessions' files.
    _dir: tempfile::TempDir,
}

/// Runs `live-check <command> <flags>` with stand-in CLIs and asserts that the stand-in of `agent`
/// recorded `sessions` sessions, each with only its environment: with `--real`, that of
/// [`assert_real_environment`]; on the stub, what `on_stub` gives for its home, the stub's address
/// and `PATH`. A failure names the variables, never their values.
fn checked_sessions(
    agent: Agent,
    command: &[&str],
    flags: &[&str],
    sessions: usize,
    on_stub: impl Fn(&Path, &str, &str) -> io::Result<Vec<(String, String)>>,
) -> Checked {
    let (recorded, path, dir) = sessions_of(agent, command, flags);
    let at = format!("{command:?} {flags:?}");
    assert_eq!(recorded.len(), sessions, "{at}");
    for session in &recorded {
        if flags.contains(&"--real") {
            assert_real_environment(agent, &at, session, &path);
        } else {
            assert_stub_environment(&at, session, &dir.path().join("tmp"), |home| {
                on_stub(home, &stub_url(agent, session), &path)
            });
        }
    }
    Checked {
        sessions: recorded,
        at,
        _dir: dir,
    }
}

/// The flags of each run and how many sessions it has, given that number without and with `--all`.
fn runs(one: usize, all: usize) -> [(&'static [&'static str], usize); 4] {
    [
        (&[], one),
        (&["--all"], all),
        (&["--real"], one),
        (&["--real", "--all"], all),
    ]
}

#[test]
fn inbox_sessions_get_only_their_environment() {
    for agent in [Agent::Claude, Agent::Codex] {
        for (flags, sessions) in runs(1, 3) {
            checked_sessions(
                agent,
                &["inbox", agent.cli()],
                flags,
                sessions,
                |h, s, p| agent.environment(h, s, p),
            );
        }
    }
}

const CHANNEL_FLAG: &str = "--dangerously-load-development-channels";

#[test]
fn channel_sessions_get_only_their_environment_and_all_but_the_fourth_load_the_channel() {
    for (flags, sessions) in runs(1, 4) {
        let real = flags.contains(&"--real");
        let checked = checked_sessions(
            Agent::Claude,
            &["channel", "claude"],
            flags,
            sessions,
            stub_environment,
        );
        let at = &checked.at;
        let with_channel = checked
            .sessions
            .iter()
            .filter(|session| session.args.iter().any(|arg| arg == CHANNEL_FLAG))
            .count();
        assert_eq!(
            with_channel,
            sessions.min(3),
            "{at}: sessions that load the channel"
        );
        for session in checked.sessions.iter().filter(|_| real) {
            // The real model signs in from the user's config home: the rest of the check's
            // environment passes on, with the terminal and the child mark.
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
fn hook_sessions_get_only_their_environment_and_run_the_handler_on_a_source_of_their_own() {
    let live_check = Path::new(env!("CARGO_BIN_EXE_live-check"));
    let handler = live_check.with_file_name("agent-hook");
    for (hook, agent, event, one, all) in HOOKS {
        for (flags, sessions) in runs(one, all) {
            let checked =
                checked_sessions(agent, &[hook, agent.cli()], flags, sessions, |h, s, p| {
                    agent.environment(h, s, p)
                });
            let at = &checked.at;
            let mut sources = Vec::new();
            for session in &checked.sessions {
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
    assert_failed(&output, "live-check: ");
}

#[test]
fn queue_sessions_get_only_their_environment_and_run_interactive_with_their_hooks() {
    for (flags, sessions) in runs(1, 3) {
        let real = flags.contains(&"--real");
        let checked = checked_sessions(
            Agent::Codex,
            &["queue", "codex"],
            flags,
            sessions,
            queue::stub_environment,
        );
        let at = &checked.at;
        for session in &checked.sessions {
            // Its own app-server, and the hooks that tell the check its thread and turns.
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
            if real {
                assert!(
                    session.env["TERM"] == "xterm-256color",
                    "{at}: TERM differs"
                );
            }
        }
    }
}

#[test]
fn app_server_sessions_get_only_their_environment_and_run_the_client_or_listen() {
    for (flags, sessions) in runs(1, 3) {
        let checked = checked_sessions(
            Agent::Codex,
            &["app-server", "codex"],
            flags,
            sessions,
            |home, stub, path| Agent::Codex.environment(home, stub, path),
        );
        let at = &checked.at;
        for session in &checked.sessions {
            assert_eq!(session.args[0], "app-server", "{at}");
            for option in ["plugins", "notify=[]", r#"model_reasoning_effort="low""#] {
                assert!(
                    session.args.iter().any(|arg| arg == option),
                    "{at}: no {option}"
                );
            }
        }
        // The client runs the idle and busy threads' own app-servers; the check runs the one that
        // listens.
        let listening = checked
            .sessions
            .iter()
            .filter(|session| session.args[1] == "--listen")
            .count();
        assert_eq!(listening, sessions / 3, "{at}");
    }
}

#[test]
fn limits_checks_run_the_cli_off_the_stub_with_only_the_owners_environment() {
    for (agent, command, sessions) in [
        (Agent::Claude, &["limits", "claude"][..], 1),
        (Agent::Claude, &["status-line", "claude"], 2),
        (Agent::Codex, &["limits", "codex"], 1),
    ] {
        let (recorded, path, _dir) = sessions_of(agent, command, &["--keep"]);
        let at = format!("{command:?}");
        assert_eq!(recorded.len(), sessions, "{at}");
        for session in &recorded {
            assert_real_environment(agent, &at, session, &path);
        }
        let usage = |session: &Recorded| session.args[..2] == ["-p", "/usage"];
        match (agent, command[0]) {
            (Agent::Codex, _) => {
                assert_eq!(recorded[0].args[0], "app-server", "{at}");
                assert!(recorded[0].args.iter().any(|arg| arg == "plugins"), "{at}");
            }
            (Agent::Claude, "limits") => assert!(usage(&recorded[0]), "{at}"),
            (Agent::Claude, _) => {
                assert_eq!(recorded.iter().filter(|s| usage(s)).count(), 1, "{at}");
                let session = recorded.iter().find(|s| !usage(s)).unwrap();
                assert_eq!(session.args[0], "Reply with only OK.", "{at}");
                assert!(session.args.iter().any(|arg| arg == "--settings"), "{at}");
            }
        }
    }
}

/// Runs `live-check` with only `home` as `HOME` and `tmp` as `TMPDIR`.
fn live_check_in(home: &Path, tmp: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_live-check"))
        .args(args)
        .env_clear()
        .env("HOME", home)
        .env("TMPDIR", tmp)
        .output()
        .expect("live-check runs")
}

/// A home where VS Code's list of extensions, recorded in `tests/surfaces/`, holds Claude Code's.
fn home_with_claude_code_for_vs_code() -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    let extensions = home.path().join(".vscode/extensions");
    fs::create_dir_all(&extensions).unwrap();
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/surfaces/extensions.json"),
        extensions.join("extensions.json"),
    )
    .unwrap();
    home
}

/// Runs `live-check <args>` with only `home` as `HOME` and `tmp` as `TMPDIR`; it must succeed.
/// Returns its output and the lines of its stdout.
fn succeeded(home: &Path, tmp: &Path, args: &[&str]) -> (Output, Vec<String>) {
    let output = live_check_in(home, tmp, args);
    assert!(output.status.success(), "{}", stderr(&output));
    let stdout = String::from_utf8(output.stdout.clone()).unwrap();
    let lines = stdout.lines().map(String::from).collect();
    (output, lines)
}

/// The check file of the GUI check prepared in `work`.
fn gui_check(work: &str) -> serde_json::Value {
    serde_json::from_str(&fs::read_to_string(Path::new(work).join("check.json")).unwrap()).unwrap()
}

#[test]
fn gui_prepares_a_check_without_showing_its_token_and_passes_only_on_it() {
    let home = home_with_claude_code_for_vs_code();
    let tmp = tempfile::tempdir().unwrap();
    let (output, lines) = succeeded(home.path(), tmp.path(), &["gui", "inbox", "claude-vscode"]);
    let stdout = lines.join("\n");
    let [version, project, prompt, then] = lines.as_slice() else {
        panic!("{stdout}");
    };
    assert_eq!(*version, "claude-vscode: Claude Code for VS Code 2.1.322");
    assert_eq!(
        *prompt,
        format!("prompt: {}", agent_live::Kind::Idle.prompt())
    );
    let work = then
        .strip_prefix("then: live-check gui-result ")
        .and_then(|rest| rest.strip_suffix(" <the token read from the reply>"))
        .expect("the command to run");
    assert!(Path::new(work).starts_with(tmp.path()), "{work}");
    assert_eq!(*project, format!("project: {work}/project"));
    assert!(Path::new(work).join("project/.mcp.json").exists());

    let check = gui_check(work);
    let token = check["token"].as_str().unwrap();
    assert!(!stdout.contains(token), "the token is shown");
    assert!(!stderr(&output).contains(token), "the token is shown");

    let result = |token: &str| live_check_in(home.path(), tmp.path(), &["gui-result", work, token]);
    let waiting = result(token);
    assert_eq!(waiting.status.code(), Some(1), "{}", stderr(&waiting));

    let source = Source::open(Path::new(work).join("source")).unwrap();
    let ids: Vec<String> = source
        .take()
        .unwrap()
        .into_iter()
        .map(|m| m.msg_id)
        .collect();
    source.handed_over(&ids).unwrap();
    let wrong = result("NONE");
    assert_eq!(wrong.status.code(), Some(1), "{}", stderr(&wrong));
    let read = result(token);
    assert!(read.status.success(), "{}", stderr(&read));
    let stdout = String::from_utf8(read.stdout).unwrap();
    assert_eq!(
        stdout,
        "claude-vscode: Claude Code for VS Code 2.1.322, no session transcript\n  the reported \
         token matches; the message is handed over\ninbox on claude-vscode with the real model: \
         pass\n"
    );
}

#[test]
fn gui_turns_off_the_users_claude_plugins_and_servers_and_loads_the_mod_from_the_project() {
    let home = home_with_claude_code_for_vs_code();
    fs::create_dir_all(home.path().join(".claude")).unwrap();
    fs::write(
        home.path().join(".claude/settings.json"),
        r#"{"enabledPlugins":{"tool@market":true,"idle@market":false},"model":"opus"}"#,
    )
    .unwrap();
    fs::write(
        home.path().join(".claude.json"),
        r#"{"mcpServers":{"search":{"command":"secret-command"}}}"#,
    )
    .unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let (_, lines) = succeeded(home.path(), tmp.path(), &["gui", "mod", "claude-vscode"]);
    assert!(
        !lines
            .iter()
            .any(|line| line.starts_with("VS Code profile:")),
        "{lines:?}"
    );
    let project = lines
        .iter()
        .find_map(|line| line.strip_prefix("project: "))
        .expect("the project");
    let settings: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(Path::new(project).join(".claude/settings.local.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        settings["enabledPlugins"],
        serde_json::json!({ "tool@market": false })
    );
    assert_eq!(
        settings["permissions"]["deny"],
        serde_json::json!(["mcp__search"])
    );
    let plugin = settings["env"]["CLAUDE_CODE_PLUGIN_DIRS"].as_str().unwrap();
    assert!(
        Path::new(plugin)
            .join(".claude-plugin/plugin.json")
            .exists(),
        "{plugin}"
    );
    assert!(!settings.to_string().contains("secret-command"));
}

#[test]
fn gui_link_opens_the_sessions_deep_link() {
    let tmp = tempfile::tempdir().unwrap();
    let work = tmp.path().join("check");
    fs::create_dir_all(work.join("source")).unwrap();
    fs::write(
        work.join("check.json"),
        r#"{"surface":"codex-desktop","path":"read","version":"v","token":"tok","busy":false,"thread":"019a-thread"}"#,
    )
    .unwrap();
    let bin = tmp.path().join("bin");
    fs::create_dir_all(&bin).unwrap();
    let opened = tmp.path().join("opened");
    let open = bin.join("open");
    fs::write(
        &open,
        format!("#!/bin/sh\nprintf '%s\\n' \"$@\" >'{}'\n", opened.display()),
    )
    .unwrap();
    fs::set_permissions(&open, fs::Permissions::from_mode(0o755)).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_live-check"))
        .args(["gui-link", work.to_str().unwrap()])
        .env_clear()
        .env("PATH", &bin)
        .env("HOME", tmp.path())
        .output()
        .expect("live-check runs");
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "opened: codex://threads/019a-thread\n"
    );
    assert_eq!(
        fs::read_to_string(&opened).unwrap(),
        "codex://threads/019a-thread\n"
    );
}

#[test]
fn gui_adds_the_busy_case_message_without_showing_its_token() {
    let home = home_with_claude_code_for_vs_code();
    let tmp = tempfile::tempdir().unwrap();
    let args = ["gui", "post-tool-use", "claude-vscode", "--busy"];
    let (_, lines) = succeeded(home.path(), tmp.path(), &args);
    let [_, _, prompt, add, _] = lines.as_slice() else {
        panic!("{lines:?}");
    };
    assert_eq!(
        *prompt,
        format!(
            "prompt: {}",
            agent_live::hook::Hook::PostToolUse.prompt(agent_live::Kind::Busy)
        )
    );
    let work = add
        .strip_prefix("once the shell command runs: live-check gui-add ")
        .expect("the command that adds the message");
    let source = Source::open(Path::new(work).join("source")).unwrap();
    assert!(source.take().unwrap().is_empty(), "added before gui-add");

    let (added, lines) = succeeded(home.path(), tmp.path(), &["gui-add", work]);
    let check = gui_check(work);
    let token = check["token"].as_str().unwrap();
    let [msg_id] = lines.as_slice() else {
        panic!("{lines:?}");
    };
    assert_eq!(msg_id, check["msg_id"].as_str().unwrap());
    assert!(!msg_id.contains(token) && !stderr(&added).contains(token));
    assert_eq!(source.state(msg_id).unwrap(), Some(State::Waiting));
    assert_failed(
        &live_check_in(home.path(), tmp.path(), &["gui-add", work]),
        "live-check: the check's message is already added",
    );
}

#[test]
fn gui_fails_without_the_surface_or_the_check() {
    let home = tempfile::tempdir().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    for args in [
        &["gui", "inbox", "claude-vscode"][..],
        &["gui-result", tmp.path().to_str().unwrap(), "tok"],
        &["gui-link", tmp.path().to_str().unwrap()],
    ] {
        assert_failed(
            &live_check_in(home.path(), tmp.path(), args),
            "live-check: ",
        );
    }
    let output = Command::new(env!("CARGO_BIN_EXE_live-check"))
        .args(["gui", "inbox", "claude-vscode"])
        .env_clear()
        .output()
        .expect("live-check runs");
    assert_failed(&output, "live-check: HOME is not set");
}

/// A home where VS Code's list of extensions and the Codex CLI that Codex for VS Code bundles,
/// recorded in `tests/surfaces/`, hold Codex for VS Code's.
fn home_with_codex_for_vs_code() -> tempfile::TempDir {
    let home = home_with_claude_code_for_vs_code();
    let platform = home
        .path()
        .join(".vscode/extensions/openai.chatgpt-26.1021.22027-darwin-arm64/bin/macos-aarch64");
    fs::create_dir_all(&platform).unwrap();
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/surfaces/codex-package-extension.json"),
        platform.join("codex-package.json"),
    )
    .unwrap();
    home
}

/// The `thread/list` exchange recorded from the Codex CLI that the Codex surfaces bundle, shortened.
fn thread_list() -> Exchange {
    Exchange::read(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/surfaces/thread-list.jsonl"))
}

/// Runs `live-check gui-queue <work> --cli` with `bin` first on `PATH`, `home` as `HOME` and `tmp`
/// as `TMPDIR`.
fn gui_queue(bin: &Path, home: &Path, tmp: &Path, work: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_live-check"))
        .args(["gui-queue", work, "--cli"])
        .env_clear()
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .env("HOME", home)
        .env("TMPDIR", tmp)
        .output()
        .expect("live-check runs")
}

/// Prepares a `codex queue` GUI check on Codex for VS Code, in the `busy` case or not, with a home
/// of [`home_with_codex_for_vs_code`] and its own `TMPDIR`. Returns them, the lines it printed and
/// the check's directory, which the line that names `gui-queue` gives.
fn queue_check(busy: bool) -> (tempfile::TempDir, tempfile::TempDir, Vec<String>, String) {
    let home = home_with_codex_for_vs_code();
    let tmp = tempfile::tempdir().unwrap();
    let mut args = vec!["gui", "queue", "codex-vscode"];
    if busy {
        args.push("--busy");
    }
    let (_, lines) = succeeded(home.path(), tmp.path(), &args);
    let when = if busy {
        "once the shell command runs"
    } else {
        "once the first turn ended"
    };
    let work = lines[3]
        .strip_prefix(&format!("{when}: live-check gui-queue "))
        .and_then(|rest| rest.strip_suffix(" [--cli]"))
        .expect("the command that queues the message")
        .to_owned();
    (home, tmp, lines, work)
}

#[test]
fn gui_queues_the_message_into_the_thread_its_prompt_started() {
    let (home, tmp, lines, work) = queue_check(false);
    let work = work.as_str();
    let [version, _, prompt, _, _] = lines.as_slice() else {
        panic!("{lines:?}");
    };
    assert_eq!(
        *version,
        "codex-vscode: Codex for VS Code 26.1021.22027, codex 0.164.0-alpha.11.1"
    );
    assert_eq!(
        *prompt,
        format!("prompt: {}", queue::prompt(agent_live::Kind::Idle))
    );
    let source = Source::open(Path::new(work).join("source")).unwrap();
    assert!(source.take().unwrap().is_empty(), "added before gui-queue");

    // The stand-in answers `thread/list` as recorded, then `codex queue` with nothing.
    let codex = tempfile::tempdir().unwrap();
    let exchange = thread_list();
    exchange.stand_in(codex.path(), "", "");
    let output = gui_queue(&codex.path().join("bin"), home.path(), tmp.path(), work);
    assert!(output.status.success(), "{}", stderr(&output));
    let stdout = String::from_utf8(output.stdout).unwrap();
    let check = gui_check(work);
    let thread = "01a1ce22-0000-7000-8000-000000000001";
    assert_eq!(check["thread"], thread);
    let msg_id = check["msg_id"].as_str().unwrap();
    // The stand-in answers the queue's listing with none queued: the message was taken.
    assert_eq!(
        stdout,
        format!("thread: {thread}\n{msg_id}\nhanded over: 1\ntaken from the queue: 0.0 s\n")
    );
    let initialize: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(codex.path().join("received/0")).unwrap())
            .unwrap();
    assert_eq!(
        initialize["params"]["capabilities"]["experimentalApi"],
        true
    );
    assert!(!stdout.contains(check["token"].as_str().unwrap()));
    assert_eq!(
        source
            .state(msg_id)
            .unwrap()
            .map(|s| matches!(s, State::HandedOver { .. })),
        Some(true)
    );

    let list: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(codex.path().join("received/2")).unwrap())
            .unwrap();
    let project = Path::new(work).join("project");
    assert_eq!(list["method"], "thread/list");
    assert_eq!(
        list["params"]["cwd"],
        serde_json::json!([project, fs::canonicalize(&project).unwrap()])
    );
    let args = fs::read_to_string(codex.path().join("args")).unwrap();
    let text = format!(
        "The check token is {}. Reply with only the check token.",
        check["token"].as_str().unwrap()
    );
    assert!(
        args.starts_with(&format!("queue\n--thread\n{thread}\n--message\n")),
        "{args}"
    );
    assert!(args.contains(&text), "{args}");
}

#[test]
fn gui_queue_adds_nothing_without_the_thread() {
    let (home, tmp, _, work) = queue_check(true);
    let work = work.as_str();
    fs::create_dir_all(home.path().join(".codex")).unwrap();
    fs::write(
        home.path().join(".codex/config.toml"),
        "[mcp_servers.alpha]\ncommand = \"a\"\n",
    )
    .unwrap();
    let codex = tempfile::tempdir().unwrap();
    let mut exchange = thread_list();
    for line in &mut exchange.0 {
        if let Some(data) = line["server"]["result"].get_mut("data") {
            data.as_array_mut()
                .unwrap()
                .retain(|thread| thread["preview"] == "Reply with only OK.");
        }
    }
    exchange.stand_in(codex.path(), "", "");
    let output = gui_queue(&codex.path().join("bin"), home.path(), tmp.path(), work);
    assert_failed(
        &output,
        &format!(
            "live-check: no thread in {work}/project started with the check's prompt: send it first"
        ),
    );
    assert!(
        gui_check(work).get("msg_id").is_none(),
        "added without the thread"
    );
    // The lookup's app-server starts none of the user's MCP servers.
    let args = fs::read_to_string(codex.path().join("args")).unwrap();
    assert!(args.starts_with("app-server\n"), "{args}");
    assert!(
        args.contains("\nmcp_servers.alpha.enabled=false\n"),
        "{args}"
    );

    // Without the bundled CLI the command fails before it runs any.
    let other = tempfile::tempdir().unwrap();
    let output = live_check_in(other.path(), tmp.path(), &["gui-queue", work]);
    assert_failed(&output, "live-check: ");
    assert!(gui_check(work).get("msg_id").is_none());
}

/// The settings file a Claude Code session of a stand-in got with `--settings`.
fn settings_of(session: &Recorded) -> serde_json::Value {
    let at = session
        .args
        .iter()
        .position(|arg| arg == "--settings")
        .expect("a settings file");
    serde_json::from_str(&fs::read_to_string(&session.args[at + 1]).unwrap()).unwrap()
}

/// The commands of the hooks of `event` in `settings`, each with its arguments.
fn hook_commands(settings: &serde_json::Value, event: &str) -> Vec<Vec<String>> {
    settings["hooks"][event][0]["hooks"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|hook| {
            std::iter::once(&hook["command"])
                .chain(hook["args"].as_array().into_iter().flatten())
                .map(|word| word.as_str().unwrap().to_owned())
                .collect()
        })
        .collect()
}

#[test]
fn wake_sessions_get_only_their_environment_and_register_on_session_start() {
    let handler = Path::new(env!("CARGO_BIN_EXE_live-check")).with_file_name("agent-hook");
    let handler = handler.to_str().unwrap();
    for (wake, all) in [("socket", 4), ("rewake", 3), ("mod", 3), ("loop", 1)] {
        for (flags, sessions) in runs(1, all) {
            let real = flags.contains(&"--real");
            let checked = checked_sessions(
                Agent::Claude,
                &[wake, "claude"],
                flags,
                sessions,
                |home, stub, path| {
                    // The bypass session runs as the desktop app and VS Code run Claude Code.
                    if home.parent().unwrap().ends_with("s") {
                        Agent::Claude.environment(home, stub, path)
                    } else {
                        stub_environment(home, stub, path)
                    }
                },
            );
            let at = &checked.at;
            for session in &checked.sessions {
                let settings = settings_of(session);
                let starts = hook_commands(&settings, "SessionStart");
                assert_eq!(starts[0][0], handler, "{at}: the registration");
                assert_eq!(starts[0].len(), 2, "{at}: {starts:?}");
                let bypass = session.args[0] == "-p";
                assert_eq!(
                    bypass,
                    wake == "socket" && session.args.contains(&"bypassPermissions".to_owned()),
                    "{at}"
                );
                if !bypass && real {
                    assert!(
                        session.env["TERM"] == "xterm-256color",
                        "{at}: TERM differs"
                    );
                }
                let rewake = starts
                    .iter()
                    .chain(&hook_commands(&settings, "Stop"))
                    .filter(|command| command.get(1).is_some_and(|arg| arg == "--wait"))
                    .count();
                assert_eq!(rewake, if wake == "rewake" { 2 } else { 0 }, "{at}");
                // The loop's prompt calls inbox, and `/loop` needs the slash commands.
                let looped = session.args[0].starts_with("/loop 1m ");
                assert_eq!(looped, wake == "loop", "{at}");
                assert_eq!(
                    session
                        .args
                        .contains(&"--disable-slash-commands".to_owned()),
                    !looped,
                    "{at}"
                );
                let plugin = session.args.iter().position(|arg| arg == "--plugin-dir");
                assert_eq!(plugin.is_some(), wake == "mod", "{at}");
                if let Some(at_plugin) = plugin {
                    let dir = Path::new(&session.args[at_plugin + 1]);
                    assert!(dir.join(".claude-plugin/plugin.json").is_file(), "{at}");
                }
            }
        }
    }
}

#[test]
fn read_sessions_get_only_their_environment() {
    for agent in [Agent::Claude, Agent::Codex] {
        for (flags, sessions) in runs(1, 2) {
            let checked =
                checked_sessions(agent, &["ack", agent.cli()], flags, sessions, |h, s, p| {
                    agent.environment(h, s, p)
                });
            for session in &checked.sessions {
                let prompt = session
                    .args
                    .iter()
                    .any(|arg| arg.contains("the ack tool") || arg.contains("as seen"));
                assert!(prompt, "{}: {:?}", checked.at, session.args);
            }
        }
        for (flags, sessions) in runs(1, 1) {
            let checked = checked_sessions(
                agent,
                &["transcript", agent.cli()],
                flags,
                sessions,
                |h, s, p| agent.environment(h, s, p),
            );
            for session in &checked.sessions {
                // The session keeps its file, which the check reads.
                for kept in ["--no-session-persistence", "--ephemeral"] {
                    assert!(!session.args.contains(&kept.to_owned()), "{}", checked.at);
                }
            }
        }
    }
    for flags in [&[][..], &["--all"]] {
        let checked =
            checked_sessions(Agent::Claude, &["resend", "claude"], flags, 1, |h, s, p| {
                Agent::Claude.environment(h, s, p)
            });
        let settings = settings_of(&checked.sessions[0]);
        let start = &hook_commands(&settings, "SessionStart")[0];
        let stop = &hook_commands(&settings, "Stop")[0];
        // The socket's source and the next path's.
        assert!(
            start[1].ends_with("/source") && stop[1].ends_with("/fallback"),
            "{start:?} {stop:?}"
        );
    }
}

#[test]
fn gui_socket_hands_the_message_to_the_registered_session_and_sends_it_again() {
    let home = home_with_claude_code_for_vs_code();
    let tmp = tempfile::tempdir().unwrap();
    let (_, lines) = succeeded(home.path(), tmp.path(), &["gui", "socket", "claude-vscode"]);
    let work = lines
        .iter()
        .find_map(|line| line.strip_prefix("once the first turn ended: live-check gui-socket "))
        .and_then(|rest| rest.strip_suffix(" [--no-auth] [--listen <seconds>]"))
        .expect("the command that hands the message over")
        .to_owned();
    let project = Path::new(&work).join("project");
    let settings: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(project.join(".claude/settings.local.json")).unwrap(),
    )
    .unwrap();
    assert!(
        settings["hooks"]["SessionStart"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .ends_with("/source'"),
        "{settings}"
    );
    assert_failed(
        &live_check_in(home.path(), tmp.path(), &["gui-socket", &work]),
        "live-check: no session registered",
    );

    // The session's SessionStart hook registered its socket, which reads two connections.
    let socket = tmp.path().join("1.sock");
    let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
    let source = Source::open(Path::new(&work).join("source")).unwrap();
    source
        .register(&agent_source::Registration {
            session_id: "s1".to_owned(),
            socket: Some(socket.to_string_lossy().into_owned()),
            token: Some("child-token".to_owned()),
            at_ms: 1,
            ..agent_source::Registration::default()
        })
        .unwrap();
    let reader = std::thread::spawn(move || {
        (0..2)
            .map(|_| {
                let (stream, _) = listener.accept().unwrap();
                std::io::BufRead::lines(std::io::BufReader::new(stream))
                    .map(Result::unwrap)
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>()
    });
    let check = |args: &[&str]| -> Vec<String> {
        let (output, lines) = succeeded(home.path(), tmp.path(), args);
        let token = gui_check(&work)["token"].as_str().unwrap().to_owned();
        assert!(!String::from_utf8_lossy(&output.stdout).contains(&token));
        lines
    };
    let first = check(&["gui-socket", &work, "--no-auth"]);
    let msg_id = gui_check(&work)["msg_id"].as_str().unwrap().to_owned();
    assert_eq!(first, [msg_id.clone(), "handed over: 1".to_owned()]);
    let again = check(&["gui-socket", &work]);
    assert_eq!(again, [msg_id.clone(), "handed over: 1".to_owned()]);
    let connections = reader.join().unwrap();
    // Without the token, then with it; the same message both times.
    assert_eq!(connections[0].len(), 1);
    assert_eq!(connections[1].len(), 2);
    assert!(connections[1][0].contains("child-token"));
    for connection in &connections {
        assert!(connection.last().unwrap().contains(&msg_id));
    }
}
