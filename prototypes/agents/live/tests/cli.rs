//! `live-check`'s commands, without running an agent CLI.

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Output};

use agent_live::Agent;
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
    let output = live_check(&["add", file.path().to_str().unwrap(), "s", "t"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).starts_with("live-check: "),
        "{}",
        stderr(&output)
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
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).starts_with("live-check: "),
        "{}",
        stderr(&output)
    );
}

/// A variable of `live-check`'s environment that the CLI on the stub must not get.
const SENTINEL: &str = "SKALD_LIVE_TEST_CREDENTIAL";

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
            // Only these variables, so the stand-ins record no credential of the test's.
            let output = Command::new(env!("CARGO_BIN_EXE_live-check"))
                .args(["inbox", agent.cli()])
                .args(flags)
                .env_clear()
                .env("PATH", &path)
                .env("TMPDIR", &tmp)
                .env(SENTINEL, "secret")
                .output()
                .expect("live-check runs");
            // The stand-ins never call the inbox, so the check fails.
            assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));

            let recorded = recorded(&record, agent.cli());
            // A failure names the variables, never their values.
            let at = format!("{} {flags:?}", agent.cli());
            assert_eq!(recorded.len(), sessions, "{at}");
            for session in &recorded {
                assert!(
                    !session.env.contains_key(SENTINEL),
                    "{at}: {SENTINEL} reached the CLI"
                );
                let home = &session.env["HOME"];
                assert!(
                    home.starts_with(tmp.to_str().unwrap()),
                    "{at}: HOME is not in TMPDIR"
                );
                let expected: BTreeMap<String, String> = agent
                    .environment(Path::new(home), &stub_url(agent, session), &path)
                    .unwrap()
                    .into_iter()
                    .collect();
                assert_eq!(
                    session.env.keys().collect::<Vec<_>>(),
                    expected.keys().collect::<Vec<_>>(),
                    "{at}: the CLI's variables"
                );
                for (name, value) in &expected {
                    assert!(session.env[name] == *value, "{at}: {name} differs");
                }
            }
        }
    }
}
