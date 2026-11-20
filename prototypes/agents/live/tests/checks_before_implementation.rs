//! The Claude Code live checks of "Checks before implementation" in `docs/dev/agent-delivery.md`:
//! `signal`, `own-text`, `executor-read-only`, `executor-full` and `executor-clean`, each run
//! against a stand-in of the CLI that records how the check starts it and, like Claude Code, runs
//! the session's `SessionStart` hooks with the session's inbox socket; and the report of their
//! results.

use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use agent_live::SENDER;
use agent_testing::write_executable;
use serde_json::{Value, json};

/// The new checks, each run as `live-check <check> claude`.
const SIGNAL: &str = "signal";
const OWN_TEXT: &str = "own-text";
const READ_ONLY: &str = "executor-read-only";
const FULL: &str = "executor-full";
const CLEAN: &str = "executor-clean";
const CHECKS: [&str; 5] = [SIGNAL, OWN_TEXT, READ_ONLY, FULL, CLEAN];

/// A variable of the parent's environment that no executor session may get.
const PARENT_ONLY: (&str, &str) = ("SKALD_PARENT_ONLY", "parent-only-value");

/// A key that would sign Claude Code in other than with the owner's sign-in.
const PARENT_KEY: (&str, &str) = ("ANTHROPIC_API_KEY", "sk-ant-parent-key");

/// Claude Code's built-in tools that only read.
const READING: [&str; 5] = ["Read", "Glob", "Grep", "LS", "NotebookRead"];

/// The longest a check may run against the stand-in.
const RUN_LIMIT: Duration = Duration::from_secs(240);

/// How long a held stand-in session waits for the test before it ends.
const HOLD_LIMIT: Duration = Duration::from_secs(90);

/// One session as the stand-in recorded it.
struct Session {
    dir: PathBuf,
    args: Vec<String>,
    env: BTreeMap<String, String>,
    /// The session's directory, its real path.
    cwd: PathBuf,
}

impl Session {
    /// The value after the flag `flag`, if given.
    fn flag(&self, flag: &str) -> Option<&str> {
        let at = self.args.iter().position(|arg| arg == flag)?;
        self.args.get(at + 1).map(String::as_str)
    }

    /// The symlinks in the session's directory when it started, each with what it resolved to.
    fn links(&self) -> Vec<(String, PathBuf)> {
        fs::read_to_string(self.dir.join("links"))
            .unwrap_or_default()
            .lines()
            .filter_map(|line| line.split_once('\t'))
            .map(|(link, target)| (link.to_owned(), PathBuf::from(target)))
            .collect()
    }

    /// The tools that the stdio MCP servers of the session's MCP configs list, per server.
    fn server_tools(&self) -> Vec<Vec<String>> {
        let mut tools = Vec::new();
        for entry in fs::read_dir(&self.dir).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            if !name.starts_with("mcp-") {
                continue;
            }
            let Ok(config) = serde_json::from_str::<Value>(&fs::read_to_string(&path).unwrap())
            else {
                continue;
            };
            for server in config["mcpServers"].as_object().into_iter().flatten() {
                if let Some(command) = server.1["command"].as_str() {
                    tools.push(list_tools(command, &server.1["args"], &server.1["env"]));
                }
            }
        }
        tools
    }
}

/// What a run of a check left: its output, the sessions the stand-in recorded, the lines the
/// check wrote to the sessions' inbox socket, and the run's directory.
struct Run {
    stdout: String,
    stderr: String,
    sessions: Vec<Session>,
    lines: Vec<Value>,
    /// The tools of each session's MCP servers, listed while the session ran.
    tools: Vec<Vec<String>>,
    dir: tempfile::TempDir,
}

impl Run {
    /// Where the run's failure messages point.
    fn at(&self) -> String {
        format!(
            "stdout:\n{}\nstderr:\n{}\nsessions: {:?}",
            self.stdout,
            self.stderr,
            self.sessions.iter().map(|s| &s.args).collect::<Vec<_>>()
        )
    }

    /// The text of each message line the check wrote to the inbox socket.
    fn socket_texts(&self) -> Vec<String> {
        self.lines
            .iter()
            .filter(|line| line["type"] == "user")
            .map(|line| content_text(&line["message"]["content"]))
            .collect()
    }

    fn tmp(&self) -> PathBuf {
        fs::canonicalize(self.dir.path().join("tmp")).unwrap()
    }
}

/// The text of a message's `content`: a string, or the text of its blocks.
fn content_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|block| block["text"].as_str())
            .collect(),
        _ => String::new(),
    }
}

/// `jq`, which the stand-in reads settings with: `script/test` needs it.
fn jq() -> PathBuf {
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .map(|dir| dir.join("jq"))
        .find(|jq| jq.is_file())
        .expect("jq on PATH")
}

/// The stand-in for `claude`: answers `--version`; otherwise records its arguments, environment,
/// directory, the symlinks there and its config home, copies its settings and MCP configs, and
/// runs the command hooks of `SessionStart` as Claude Code does, with the inbox socket `socket`.
/// A session that ran them waits until `record/done` exists.
fn stand_in(record: &Path, socket: &Path) -> String {
    format!(
        r#"#!/bin/sh
if [ "$1" = --version ]; then echo '2.1.0 (Claude Code)'; exit 0; fi
record='{record}'
rec="$record/claude-$$"
mkdir -p "$rec"
printf '%s\n' "$@" >"$rec/args"
env >"$rec/env"
pwd -P >"$rec/cwd"
find . -type l | while IFS= read -r link; do
  printf '%s\t%s\n' "$link" "$(readlink -f "$link")"
done >"$rec/links"
if [ -n "$CLAUDE_CONFIG_DIR" ]; then
  if [ -d "$CLAUDE_CONFIG_DIR" ]; then cp -RP "$CLAUDE_CONFIG_DIR" "$rec/config"; fi
elif [ -d "$HOME/.claude" ]; then
  cp -RP "$HOME/.claude" "$rec/config"
  if [ -f "$HOME/.claude.json" ]; then cp "$HOME/.claude.json" "$rec/config/.claude.json"; fi
fi
n=0
prev=
for arg in "$@"; do
  case "$prev" in
  --settings | --mcp-config)
    kind=settings
    [ "$prev" = --mcp-config ] && kind=mcp
    case "$arg" in
    '{{'*) printf '%s' "$arg" >"$rec/$kind-$n.json" ;;
    *) cp "$arg" "$rec/$kind-$n.json" 2>/dev/null ;;
    esac
    n=$((n + 1))
    ;;
  esac
  prev=$arg
done
for file in .claude/settings.local.json .claude/settings.json; do
  if [ -f "$file" ]; then cp "$file" "$rec/settings-$n.json"; n=$((n + 1)); fi
done
if [ -f .mcp.json ]; then cp .mcp.json "$rec/mcp-project.json"; fi
hooks=$(cat "$rec"/settings-*.json 2>/dev/null | '{jq}' -r '.hooks.SessionStart[]?.hooks[]?
  | select(.type == "command" and (.async | not) and (.asyncRewake | not))
  | if .args then [.command] + .args | @sh else .command end')
if [ -n "$hooks" ]; then
  input=$(printf '{{"session_id":"stand-in-%s","transcript_path":"%s/transcript.jsonl","cwd":"%s","hook_event_name":"SessionStart","source":"startup"}}' "$$" "$rec" "$(pwd -P)")
  printf '%s\n' "$hooks" | while IFS= read -r hook; do
    printf '%s' "$input" | CLAUDE_CODE_MESSAGING_SOCKET='{socket}' \
      CLAUDE_CODE_MESSAGING_TOKEN=stand-in-token sh -c "$hook" >/dev/null 2>&1
  done
  touch "$rec/started"
  i=0
  while [ ! -e "$record/done" ] && [ "$i" -lt {hold} ]; do sleep 0.1; i=$((i + 1)); done
fi
exit 0
"#,
        record = record.display(),
        socket = socket.display(),
        jq = jq().display(),
        hold = HOLD_LIMIT.as_millis() / 100,
    )
}

/// A fake user home, with Claude Code's config in it, that no check may change.
fn user_home(home: &Path) {
    fs::create_dir_all(home.join(".claude")).unwrap();
    fs::write(home.join(".claude.json"), r#"{"mcpServers":{}}"#).unwrap();
    fs::write(home.join(".claude/settings.json"), r#"{"hooks":{}}"#).unwrap();
    fs::write(
        home.join(".claude/CLAUDE.md"),
        "The user's own instructions.\n",
    )
    .unwrap();
}

/// Every path under `dir` with the contents of each file.
fn snapshot(dir: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    let mut found = BTreeMap::new();
    let mut left = vec![dir.to_path_buf()];
    while let Some(next) = left.pop() {
        for entry in fs::read_dir(&next).unwrap() {
            let path = entry.unwrap().path();
            let meta = fs::symlink_metadata(&path).unwrap();
            if meta.is_dir() {
                left.push(path.clone());
                found.insert(path, None);
            } else {
                found.insert(path.clone(), Some(fs::read(&path).unwrap_or_default()));
            }
        }
    }
    found
}

/// Runs `live-check <check> claude <flags>` with the stand-in, the fake user home, [`PARENT_ONLY`]
/// and [`PARENT_KEY`] in its environment. With `hold`, each session that ran its `SessionStart`
/// hooks waits until the check wrote to its inbox socket and the test listed its MCP servers'
/// tools.
fn run(check: &str, flags: &[&str], hold: bool) -> Run {
    let dir = tempfile::tempdir().unwrap();
    let (bin, record, tmp, home) = (
        dir.path().join("bin"),
        dir.path().join("record"),
        dir.path().join("tmp"),
        dir.path().join("home"),
    );
    for made in [&bin, &record, &tmp] {
        fs::create_dir(made).unwrap();
    }
    user_home(&home);
    let socket = dir.path().join("s.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    listener.set_nonblocking(true).unwrap();
    write_executable(&bin.join("claude"), &stand_in(&record, &socket));
    if !hold {
        fs::write(record.join("done"), "").unwrap();
    }

    let mut child = Command::new(env!("CARGO_BIN_EXE_live-check"))
        .args([check, "claude"])
        .args(flags)
        .env_clear()
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .env("TMPDIR", &tmp)
        .env("HOME", &home)
        .env(PARENT_ONLY.0, PARENT_ONLY.1)
        .env(PARENT_KEY.0, PARENT_KEY.1)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("live-check runs");
    let read_all = |mut from: Box<dyn Read + Send>| {
        std::thread::spawn(move || {
            let mut text = String::new();
            let _ = from.read_to_string(&mut text);
            text
        })
    };
    let stdout = read_all(Box::new(child.stdout.take().unwrap()));
    let stderr = read_all(Box::new(child.stderr.take().unwrap()));

    let started = Instant::now();
    let mut lines = Vec::new();
    let mut tools = Vec::new();
    let mut released = !hold;
    loop {
        accept_lines(&listener, &mut lines);
        if !released {
            let wrote = lines.iter().any(|line| line["type"] == "user");
            let held = recorded(&record)
                .into_iter()
                .find(|s| s.dir.join("started").exists());
            if let (true, Some(session)) = (wrote, held) {
                tools = session.server_tools();
            }
            if wrote || started.elapsed() > HOLD_LIMIT {
                fs::write(record.join("done"), "").unwrap();
                released = true;
            }
        }
        if child.try_wait().unwrap().is_some() {
            break;
        }
        if started.elapsed() > RUN_LIMIT {
            let _ = child.kill();
            panic!("live-check {check} claude {flags:?} ran over {RUN_LIMIT:?}");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    accept_lines(&listener, &mut lines);
    assert!(
        snapshot(&home) == {
            let fresh = tempfile::tempdir().unwrap();
            user_home(fresh.path());
            snapshot(fresh.path())
                .into_iter()
                .map(|(path, contents)| {
                    (
                        home.join(path.strip_prefix(fresh.path()).unwrap()),
                        contents,
                    )
                })
                .collect()
        },
        "live-check {check} claude {flags:?} changed the user's Claude Code config"
    );
    let stdout = stdout.join().unwrap();
    let stderr = stderr.join().unwrap();
    assert_ne!(
        child.wait().unwrap().code(),
        Some(2),
        "live-check {check} claude {flags:?} printed the usage:\n{stderr}"
    );
    Run {
        stdout,
        stderr,
        sessions: recorded(&record),
        lines,
        tools,
        dir,
    }
}

/// Reads each connection waiting on `listener` to its end and adds its JSON lines to `lines`.
fn accept_lines(listener: &UnixListener, lines: &mut Vec<Value>) {
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                // macOS refuses these on a connection its peer already closed.
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
                for line in BufReader::new(stream).lines() {
                    let Ok(line) = line else { break };
                    if let Ok(value) = serde_json::from_str(&line) {
                        lines.push(value);
                    }
                }
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => return,
            Err(error) => panic!("the inbox socket: {error}"),
        }
    }
}

/// The sessions the stand-in recorded in `record`.
fn recorded(record: &Path) -> Vec<Session> {
    let mut sessions: Vec<Session> = fs::read_dir(record)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|dir| dir.is_dir() && dir.join("cwd").exists())
        .map(|dir| Session {
            args: fs::read_to_string(dir.join("args"))
                .unwrap()
                .lines()
                .map(String::from)
                .collect(),
            env: fs::read_to_string(dir.join("env"))
                .unwrap()
                .lines()
                .filter_map(|line| line.split_once('='))
                .map(|(name, value)| (name.to_owned(), value.to_owned()))
                .collect(),
            cwd: PathBuf::from(fs::read_to_string(dir.join("cwd")).unwrap().trim_end()),
            dir,
        })
        .collect();
    sessions.sort_by(|a, b| a.dir.cmp(&b.dir));
    sessions
}

/// The names of the tools a stdio MCP server, `command <args>` with `env`, lists.
fn list_tools(command: &str, args: &Value, env: &Value) -> Vec<String> {
    let mut server = Command::new(command);
    server
        .args(
            args.as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str),
        )
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    for (name, value) in env.as_object().into_iter().flatten() {
        server.env(name, value.as_str().unwrap_or_default());
    }
    let Ok(mut child) = server.spawn() else {
        return Vec::new();
    };
    let (sent, answers) = mpsc::channel::<Value>();
    let out = child.stdout.take().unwrap();
    std::thread::spawn(move || {
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            if let Ok(value) = serde_json::from_str(&line)
                && sent.send(value).is_err()
            {
                return;
            }
        }
    });
    let mut input = child.stdin.take().unwrap();
    let answer = |input: &mut std::process::ChildStdin, request: Value| -> Option<Value> {
        writeln!(input, "{request}").ok()?;
        let id = request["id"].clone();
        let until = Instant::now() + Duration::from_secs(10);
        loop {
            let left = until.checked_duration_since(Instant::now())?;
            let message = answers.recv_timeout(left).ok()?;
            if message["id"] == id {
                return Some(message);
            }
        }
    };
    let initialized = answer(
        &mut input,
        json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": { "name": "acceptance", "version": "0" },
        },
        }),
    );
    let mut names = Vec::new();
    if initialized.is_some() {
        let _ = writeln!(
            input,
            "{}",
            json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })
        );
        if let Some(listed) = answer(
            &mut input,
            json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {} }),
        ) {
            names = listed["result"]["tools"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|tool| tool["name"].as_str().map(String::from))
                .collect();
        }
    }
    drop(input);
    let _ = child.kill();
    let _ = child.wait();
    names
}

#[test]
fn usage_names_each_check_before_implementation() {
    let output = Command::new(env!("CARGO_BIN_EXE_live-check"))
        .env_clear()
        .output()
        .expect("live-check runs");
    let usage = String::from_utf8_lossy(&output.stderr);
    for check in CHECKS {
        assert!(
            usage.contains(check),
            "{check} is not in the usage:\n{usage}"
        );
    }
}

#[test]
fn signal_is_a_fixed_line_without_text_or_sender_and_the_server_has_inbox_and_claim() {
    let runs = [run(SIGNAL, &[], true), run(SIGNAL, &[], true)];
    let mut signals = Vec::new();
    for run in &runs {
        let at = run.at();
        let texts = run.socket_texts();
        assert!(!texts.is_empty(), "no signal reached the socket: {at}");
        for line in run.lines.iter().filter(|line| line["type"] == "user") {
            assert!(
                !line.to_string().contains(SENDER),
                "the signal names the sender: {line}"
            );
        }
        for text in &texts {
            let lower = text.to_lowercase();
            assert!(
                lower.contains("message") && lower.contains("task"),
                "the signal does not say that messages and a task wait: {text}"
            );
        }
        assert!(
            run.tools.iter().any(|tools| {
                tools.iter().any(|tool| tool == "inbox") && tools.iter().any(|tool| tool == "claim")
            }),
            "no MCP server of the session lists inbox and claim: {:?}; {at}",
            run.tools
        );
        signals.extend(texts);
    }
    // A fixed line: the same in every session and run, so no message's text or token is in it.
    assert!(
        signals.windows(2).all(|pair| pair[0] == pair[1]),
        "the signals differ: {signals:?}"
    );
}

#[test]
fn own_text_comes_over_the_socket_labelled_as_from_the_persons_own_agent() {
    let run = run(OWN_TEXT, &[], true);
    let texts = run.socket_texts();
    assert!(
        !texts.is_empty(),
        "no request reached the socket: {}",
        run.at()
    );
    for text in &texts {
        assert!(
            text.to_lowercase().contains("own agent"),
            "not labelled as from the person's own agent: {text}"
        );
    }
}

/// Runs the executor check `check` with `flags` and asserts that each of its sessions runs with no
/// person, `-p`, in a folder of the check's own.
fn executor(check: &str, flags: &[&str]) -> Run {
    let run = run(check, flags, false);
    let at = run.at();
    assert!(!run.sessions.is_empty(), "no session: {at}");
    for session in &run.sessions {
        assert!(session.args.iter().any(|arg| arg == "-p"), "not -p: {at}");
        assert!(
            session.cwd.starts_with(run.tmp()),
            "the session's folder {} is not the check's own: {at}",
            session.cwd.display()
        );
    }
    run
}

#[test]
fn read_only_executor_runs_without_a_person_with_reading_tools_and_without_asking() {
    for flags in [&[][..], &["--real"]] {
        let run = executor(READ_ONLY, flags);
        let at = run.at();
        for session in &run.sessions {
            assert_eq!(session.flag("--permission-mode"), Some("dontAsk"), "{at}");
            let tools: Vec<&str> = session
                .flag("--tools")
                .unwrap_or_default()
                .split([',', ' '])
                .filter(|tool| !tool.is_empty())
                .collect();
            assert!(tools.contains(&"Read"), "--tools {tools:?}: {at}");
            assert!(
                tools.iter().all(|tool| READING.contains(tool)),
                "--tools {tools:?} holds a tool that does more than read: {at}"
            );
        }
        // The agent is told to read a file outside its folder through a symlink inside it too.
        let outside = run.sessions.iter().any(|session| {
            session
                .links()
                .iter()
                .any(|(_, target)| !target.starts_with(&session.cwd))
        });
        assert!(outside, "no symlink in the folder points outside it: {at}");
    }
}

#[test]
fn full_executor_runs_without_a_person_and_may_write_and_run_commands() {
    for flags in [&[][..], &["--real"]] {
        let run = executor(FULL, flags);
        let at = run.at();
        for session in &run.sessions {
            if let Some(tools) = session.flag("--tools") {
                assert!(
                    tools.contains("Bash") && (tools.contains("Write") || tools.contains("Edit")),
                    "--tools {tools} cannot write a file and run a command: {at}"
                );
            }
        }
    }
}

/// The name Claude Code gives a project's directory in the config home: its real path with `-`
/// for each character but ASCII letters and digits.
fn project_key(cwd: &Path) -> String {
    cwd.to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// The JSON file at `path`, or `null`.
fn json_at(path: &Path) -> Value {
    fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or(Value::Null)
}

/// Whether `value` is a non-empty object.
fn filled(value: &Value) -> bool {
    value.as_object().is_some_and(|map| !map.is_empty())
}

/// Asserts that the config home the session got, as it was when the session started, holds an
/// MCP server, a hook, a plugin, a skill, an instruction file and the project's memory, each where
/// Claude Code reads it.
fn assert_planted(session: &Session, at: &str) {
    let config = session.dir.join("config");
    // `CLAUDE_CONFIG_DIR`, or `.claude` in `HOME` with `.claude.json` next to it.
    let home = match (
        session.env.get("CLAUDE_CONFIG_DIR"),
        session.env.get("HOME"),
    ) {
        (Some(dir), _) => PathBuf::from(dir),
        (None, Some(home)) => Path::new(home).join(".claude"),
        (None, None) => panic!("no config home: {at}"),
    };
    assert!(config.is_dir(), "no config home: {at}");

    let state = json_at(&config.join(".claude.json"));
    let servers = filled(&state["mcpServers"])
        || state["projects"]
            .as_object()
            .into_iter()
            .flatten()
            .any(|(_, project)| filled(&project["mcpServers"]));
    assert!(
        servers,
        "no MCP server in {}: {at}",
        home.join(".claude.json").display()
    );

    let settings = json_at(&config.join("settings.json"));
    let hook = settings["hooks"]
        .as_object()
        .into_iter()
        .flatten()
        .flat_map(|(_, matchers)| matchers.as_array().into_iter().flatten())
        .flat_map(|matcher| matcher["hooks"].as_array().into_iter().flatten())
        .any(|hook| hook["command"].is_string());
    assert!(hook, "no hook in settings.json: {settings}; {at}");

    let installed = json_at(&config.join("plugins/installed_plugins.json"));
    let plugin = settings["enabledPlugins"]
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(_, on)| **on == Value::Bool(true))
        .find_map(|(name, _)| installed["plugins"][name].as_array()?.first().cloned());
    let plugin = plugin.unwrap_or_else(|| {
        panic!("no enabled plugin in plugins/installed_plugins.json: {installed}; {at}")
    });
    let install = PathBuf::from(plugin["installPath"].as_str().unwrap_or_default());
    let install = match install.strip_prefix(&home) {
        Ok(inside) => config.join(inside),
        Err(_) => install,
    };
    assert!(
        install.join(".claude-plugin/plugin.json").is_file(),
        "the plugin's installPath holds no .claude-plugin/plugin.json: {plugin}; {at}"
    );

    let skill = fs::read_dir(config.join("skills"))
        .into_iter()
        .flatten()
        .any(|entry| entry.unwrap().path().join("SKILL.md").is_file());
    assert!(skill, "no skills/<name>/SKILL.md: {at}");

    let instructions = fs::read_to_string(config.join("CLAUDE.md")).unwrap_or_default();
    assert!(!instructions.trim().is_empty(), "no CLAUDE.md: {at}");

    let memory = config
        .join("projects")
        .join(project_key(&session.cwd))
        .join("memory");
    let remembered = fs::read_dir(&memory).into_iter().flatten().any(|entry| {
        entry
            .unwrap()
            .path()
            .extension()
            .is_some_and(|ext| ext == "md")
    });
    assert!(
        remembered,
        "no project memory in {}: {at}",
        memory.display()
    );
}

#[test]
fn clean_executor_gets_none_of_the_parents_environment_and_a_planted_config_home() {
    for flags in [&[][..], &["--real"]] {
        let real = !flags.is_empty();
        let run = executor(CLEAN, flags);
        let at = run.at();
        for session in &run.sessions {
            assert!(
                !session.env.contains_key(PARENT_ONLY.0),
                "the parent's variable reached the session: {at}"
            );
            assert!(
                !session.env.values().any(|value| value == PARENT_KEY.1),
                "the parent's key reached the session: {at}"
            );
            if !real {
                assert_planted(session, &at);
            }
        }
    }
}

#[test]
fn checks_with_the_real_model_leave_the_users_config_alone() {
    // `run` asserts that the fake user home is as it was.
    for check in CHECKS {
        run(check, &["--real"], false);
    }
}

/// The section "Checks before implementation" of `docs/dev/agent-delivery.md`.
fn report() -> String {
    let doc = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../docs/dev/agent-delivery.md");
    let doc = fs::read_to_string(doc).unwrap();
    let start = doc
        .find("\n## Checks before implementation\n")
        .expect("the section");
    let rest = &doc[start + 1..];
    let end = rest[3..].find("\n## ").map_or(rest.len(), |end| end + 3);
    rest[..end].to_owned()
}

/// Whether `text` names a Claude Code version, `Claude Code <major>.<minor>.<patch>`.
fn names_a_version(text: &str) -> bool {
    text.match_indices("Claude Code ").any(|(at, prefix)| {
        let version: String = text[at + prefix.len()..]
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        let parts: Vec<&str> = version.trim_end_matches('.').split('.').collect();
        parts.len() == 3 && parts.iter().all(|part| !part.is_empty())
    })
}

#[test]
fn the_report_gives_each_claude_code_checks_result_and_version_on_the_stub_and_the_real_model() {
    let report = report();
    assert!(
        names_a_version(&report),
        "no Claude Code version in the section:\n{report}"
    );
    for check in CHECKS {
        let command = format!("{check} claude");
        let lines: Vec<&str> = report
            .lines()
            .filter(|line| line.contains(&command))
            .collect();
        assert!(
            !lines.is_empty(),
            "{command} is not in the section:\n{report}"
        );
        // One result on the stub and one with `--real`: two lines, or two columns of one row.
        let results: usize = lines
            .iter()
            .map(|line| line.matches("pass").count() + line.matches("fail").count())
            .sum();
        assert!(
            results >= 2,
            "no result of {command} on the stub and with --real:\n{report}"
        );
    }
    assert!(
        report.contains("--real"),
        "no result with --real in the section:\n{report}"
    );
}
