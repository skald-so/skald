//! The Codex live checks of "Checks before implementation" in `docs/dev/agent-delivery.md`:
//! `signal`, `executor-read-only`, `executor-full` and `executor-clean`, each run as
//! `live-check <check> codex` against a stand-in of the CLI that records how the check starts it
//! and, like Codex, runs the hooks of the session's config, writes into its config home and takes
//! `codex queue`; and the report of their results.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use agent_live::SENDER;
use agent_testing::{snapshot, write_executable};
use serde_json::{Value, json};

/// The new checks, each run as `live-check <check> codex`.
const SIGNAL: &str = "signal";
const READ_ONLY: &str = "executor-read-only";
const FULL: &str = "executor-full";
const CLEAN: &str = "executor-clean";
const CHECKS: [&str; 4] = [SIGNAL, READ_ONLY, FULL, CLEAN];

/// The version the stand-in gives for `codex --version`.
const VERSION: &str = "0.0.0-acceptance";

/// A variable of the parent's environment that no executor session may get.
const PARENT_ONLY: (&str, &str) = ("SKALD_PARENT_ONLY", "parent-only-value");

/// A key that would sign Codex in: no session on the model stub may get it.
const PARENT_KEY: (&str, &str) = ("OPENAI_API_KEY", "sk-parent-key");

/// The longest a check may run against the stand-in.
const RUN_LIMIT: Duration = Duration::from_secs(300);

/// How long a stand-in terminal UI session waits for a queued message before it ends.
const HOLD_LIMIT: Duration = Duration::from_secs(90);

/// One run of the stand-in, as it recorded itself.
struct Invocation {
    dir: PathBuf,
    /// `tui` for the terminal UI, else the subcommand: `exec`, `queue`, `login`, `app-server`, …
    kind: String,
    args: Vec<String>,
    env: BTreeMap<String, String>,
    /// The session's directory, `-C` or else the working directory, its real path.
    folder: PathBuf,
    /// The config home, `CODEX_HOME` or else `~/.codex`, its real path.
    config_home: PathBuf,
}

impl Invocation {
    /// The value of the option named one of `names`, as `<name> <value>` or `<name>=<value>`.
    fn option(&self, names: &[&str]) -> Option<&str> {
        self.args.iter().enumerate().find_map(|(at, arg)| {
            names.iter().find_map(|name| {
                if arg == name {
                    self.args.get(at + 1).map(String::as_str)
                } else {
                    arg.strip_prefix(&format!("{name}="))
                }
            })
        })
    }

    /// The values of each `-c` or `--config` option.
    fn configs(&self) -> Vec<&str> {
        self.args
            .windows(2)
            .filter(|pair| pair[0] == "-c" || pair[0] == "--config")
            .map(|pair| pair[1].as_str())
            .chain(
                self.args
                    .iter()
                    .filter_map(|arg| arg.strip_prefix("--config=")),
            )
            .collect()
    }

    /// The Codex sandbox the session runs in, as its arguments set it.
    fn sandbox(&self) -> Option<String> {
        let bypass = ["--dangerously-bypass-approvals-and-sandbox", "--yolo"];
        if self.args.iter().any(|arg| bypass.contains(&arg.as_str())) {
            return Some("danger-full-access".to_owned());
        }
        if let Some(mode) = self.option(&["--sandbox", "-s"]) {
            return Some(mode.to_owned());
        }
        if let Some(mode) = self
            .configs()
            .iter()
            .find_map(|value| value.strip_prefix("sandbox_mode="))
        {
            return Some(mode.trim_matches(['"', '\'']).to_owned());
        }
        self.args
            .iter()
            .any(|arg| arg == "--full-auto")
            .then(|| "workspace-write".to_owned())
    }

    /// The terminal UI session's thread, which its hooks gave as `session_id`.
    fn thread(&self) -> String {
        fs::read_to_string(self.dir.join("thread"))
            .unwrap_or_default()
            .trim_end()
            .to_owned()
    }

    /// The config home as it was when the stand-in started.
    fn config(&self) -> PathBuf {
        self.dir.join("config")
    }

    /// The stdio MCP servers the session got, `-c mcp_servers.<name>.…` or `[mcp_servers.<name>]`
    /// in its config home's `config.toml`: each server's command and arguments.
    fn mcp_servers(&self) -> Vec<(String, Vec<String>)> {
        let mut servers: BTreeMap<String, (Option<String>, Vec<String>)> = BTreeMap::new();
        let mut set = |name: &str, key: &str, value: &str| {
            let entry = servers.entry(name.to_owned()).or_default();
            match key {
                "command" => entry.0 = serde_json::from_str(value).ok(),
                "args" => entry.1 = serde_json::from_str(value).unwrap_or_default(),
                _ => {}
            }
        };
        for value in self.configs() {
            let Some(rest) = value.strip_prefix("mcp_servers.") else {
                continue;
            };
            let Some((path, value)) = rest.split_once('=') else {
                continue;
            };
            if let Some((name, key)) = path.rsplit_once('.') {
                set(name.trim_matches('"'), key.trim(), value.trim());
            }
        }
        let toml = fs::read_to_string(self.config().join("config.toml")).unwrap_or_default();
        let mut section: Option<String> = None;
        for line in toml.lines().map(str::trim) {
            if let Some(header) = line.strip_prefix('[') {
                section = header
                    .trim_end_matches(']')
                    .strip_prefix("mcp_servers.")
                    .map(|name| name.trim_matches('"').to_owned());
            } else if let (Some(name), Some((key, value))) = (&section, line.split_once('=')) {
                set(name, key.trim(), value.trim());
            }
        }
        servers
            .into_values()
            .filter_map(|(command, args)| Some((command?, args)))
            .collect()
    }
}

/// What a run of a check left: its output, what the stand-in recorded, and the run's directory.
struct Run {
    stdout: String,
    stderr: String,
    invocations: Vec<Invocation>,
    /// The tools of each terminal UI session's MCP servers, listed while the check ran.
    tools: Vec<Vec<String>>,
    dir: tempfile::TempDir,
}

impl Run {
    /// Where the run's failure messages point.
    fn at(&self) -> String {
        format!(
            "stdout:\n{}\nstderr:\n{}\ninvocations: {:?}",
            self.stdout,
            self.stderr,
            self.invocations
                .iter()
                .map(|i| (&i.kind, &i.args))
                .collect::<Vec<_>>()
        )
    }

    /// The invocations of the kind `kind`.
    fn of(&self, kind: &str) -> Vec<&Invocation> {
        self.invocations.iter().filter(|i| i.kind == kind).collect()
    }

    /// Each `codex queue` run: its thread, its text and the run itself.
    fn queued(&self) -> Vec<(String, String, &Invocation)> {
        self.of("queue")
            .into_iter()
            .map(|queue| {
                (
                    queue.option(&["--thread"]).unwrap_or_default().to_owned(),
                    queue.option(&["--message"]).unwrap_or_default().to_owned(),
                    queue,
                )
            })
            .collect()
    }

    /// The run's `TMPDIR`, its real path.
    fn tmp(&self) -> PathBuf {
        fs::canonicalize(self.dir.path().join("tmp")).unwrap()
    }
}

/// `jq`, which the stand-in reads hooks and app-server requests with: `script/test` needs it.
fn jq() -> String {
    let found = Command::new("sh")
        .args(["-c", "command -v jq"])
        .output()
        .expect("sh runs");
    let jq = String::from_utf8_lossy(&found.stdout).trim().to_owned();
    assert!(!jq.is_empty(), "no jq on PATH");
    jq
}

/// The stand-in for `codex`. Like Codex, it writes into its config home whenever it starts, even
/// for `--version`. It answers `--version`; otherwise it records its arguments,
/// environment, directory and a copy of its config home in `record/<kind>-<pid>`, and then:
/// - the terminal UI, like Codex, writes into its config home and runs the command hooks of `-c
///   hooks.<event>=…` and of the config home's `hooks.json`: `SessionStart`, a `PreToolUse` of a
///   shell command and `Stop`, with its thread as `session_id`; then `Stop` again for each new
///   `codex queue` run, until a while after `record/done` exists;
/// - `exec`, like Codex, writes into its config home and ends without output;
/// - `queue` ends at once; `login` signs in in its config home; `app-server` answers each request.
const STAND_IN: &str = r#"#!/bin/sh
record='@RECORD@'
jq='@JQ@'
mkdir -p "${CODEX_HOME:-$HOME/.codex}/tmp/arg0" && : >"${CODEX_HOME:-$HOME/.codex}/tmp/arg0/codex-arg0.lock"
if [ "$1" = --version ] || [ "$1" = -V ]; then echo 'codex-cli @VERSION@'; exit 0; fi
sub=
folder=$(pwd -P)
prev=
for arg in "$@"; do
  case "$prev" in
  -C | --cd) folder=$(cd "$arg" 2>/dev/null && pwd -P) ;;
  esac
  case "$arg" in
  --cd=*) folder=$(cd "${arg#--cd=}" 2>/dev/null && pwd -P) ;;
  esac
  if [ -z "$sub" ]; then
    case "$prev" in
    -c | --config | -C | --cd | -m | --model | -s | --sandbox | -a | --ask-for-approval | -p | --profile | --enable | --disable | -i | --image | --add-dir | --local-provider | --remote | --remote-auth-token-env) ;;
    *)
      case "$arg" in
      -*) ;;
      exec | e) sub=exec ;;
      queue | login | logout | app-server | mcp | mcp-server | features | sandbox | debug | apply | completion | cloud | resume | fork) sub=$arg ;;
      *) sub=tui ;;
      esac
      ;;
    esac
  fi
  prev=$arg
done
[ -n "$sub" ] || sub=tui
rec="$record/$sub-$$"
mkdir -p "$rec"
printf '%s\0' "$@" >"$rec/args"
env >"$rec/env"
printf '%s\n' "$folder" >"$rec/folder"
config=${CODEX_HOME:-$HOME/.codex}
if [ -d "$config" ]; then
  cp -RP "$config" "$rec/config" 2>/dev/null
  (cd "$config" && pwd -P) >"$rec/config-home"
else
  printf '%s\n' "$config" >"$rec/config-home"
fi
prev=
for arg in "$@"; do
  if [ "$prev" = -c ] || [ "$prev" = --config ]; then
    case "$arg" in
    hooks.*=*)
      event=${arg#hooks.}
      event=${event%%=*}
      printf '%s\n' "$arg" | sed -E -n 's/.*command="(([^"\\]|\\.)*)".*/\1/p' |
        sed -e 's/\\"/"/g' -e 's/\\\\/\\/g' >>"$rec/hooks-$event"
      ;;
    esac
  fi
  prev=$arg
done
run_hooks() {
  {
    cat "$rec/hooks-$1" 2>/dev/null
    if [ -f "$config/hooks.json" ]; then
      "$jq" -r --arg e "$1" '.hooks[$e][]?.hooks[]? | select(.type == "command") | .command' \
        "$config/hooks.json" 2>/dev/null
    fi
  } | while IFS= read -r hook || [ -n "$hook" ]; do
    if [ -n "$hook" ]; then printf '%s' "$2" | sh -c "$hook" >/dev/null 2>&1; fi
  done
}
case "$sub" in
tui)
  mkdir -p "$config" && printf '{"session":"stand-in"}\n' >>"$config/history.jsonl"
  thread=$(printf '019a0000-0000-7000-8000-%012d' $$)
  printf '%s\n' "$thread" >"$rec/thread"
  base='"session_id":"'"$thread"'","transcript_path":null,"cwd":"'"$folder"'","model":"stub","turn_id":"turn-1"'
  stop="{$base,\"hook_event_name\":\"Stop\",\"stop_hook_active\":false,\"last_assistant_message\":\"OK\"}"
  run_hooks SessionStart "{$base,\"hook_event_name\":\"SessionStart\",\"source\":\"startup\"}"
  run_hooks PreToolUse "{$base,\"hook_event_name\":\"PreToolUse\",\"tool_name\":\"Bash\",\"tool_use_id\":\"call-1\",\"tool_input\":{\"command\":\"sleep 20\"}}"
  run_hooks Stop "$stop"
  seen=0
  quiet=0
  i=0
  while [ "$i" -lt @HOLD@ ]; do
    n=$(find "$record" -maxdepth 1 -name 'queue-*' | wc -l | tr -d ' ')
    if [ "$n" -gt "$seen" ]; then
      seen=$n
      quiet=0
      run_hooks Stop "$stop"
    elif [ "$seen" -gt 0 ] && [ -e "$record/done" ]; then
      quiet=$((quiet + 1))
      [ "$quiet" -lt 50 ] || break
    fi
    sleep 0.1
    i=$((i + 1))
  done
  ;;
exec)
  mkdir -p "$config" && printf '{"session":"stand-in"}\n' >>"$config/history.jsonl"
  ;;
login)
  case " $* " in *' status '*) echo 'Logged in using an API key'; exit 0 ;; esac
  case " $* " in *' --with-api-key '*) cat >/dev/null ;; esac
  mkdir -p "$config" && printf '{"auth_mode":"apikey"}\n' >"$config/auth.json"
  ;;
app-server)
  while IFS= read -r line; do
    id=$(printf '%s' "$line" | "$jq" -c '.id // empty' 2>/dev/null)
    if [ -n "$id" ]; then printf '{"jsonrpc":"2.0","id":%s,"result":{"data":[]}}\n' "$id"; fi
  done
  ;;
esac
exit 0
"#;

/// The stand-in's script, recording in `record`.
fn stand_in(record: &Path) -> String {
    STAND_IN
        .replace("@RECORD@", &record.to_string_lossy())
        .replace("@JQ@", &jq())
        .replace("@VERSION@", VERSION)
        .replace("@HOLD@", &(HOLD_LIMIT.as_millis() / 100).to_string())
}

/// A fake user home, with Codex's config in it, that no check may change.
fn user_home(home: &Path) {
    let codex = home.join(".codex");
    fs::create_dir_all(&codex).unwrap();
    fs::write(
        codex.join("config.toml"),
        "[mcp_servers.users]\ncommand = \"users-server\"\n",
    )
    .unwrap();
    fs::write(codex.join("hooks.json"), r#"{"hooks":{}}"#).unwrap();
    fs::write(codex.join("AGENTS.md"), "The user's own instructions.\n").unwrap();
    fs::write(codex.join("auth.json"), r#"{"auth_mode":"chatgpt"}"#).unwrap();
}

/// The invocations the stand-in recorded in `record`.
fn recorded(record: &Path) -> Vec<Invocation> {
    let mut found: Vec<Invocation> = fs::read_dir(record)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|dir| dir.is_dir() && dir.join("config-home").exists())
        .map(|dir| {
            let name = dir.file_name().unwrap().to_string_lossy().into_owned();
            let kind = name.rsplit_once('-').map_or(&*name, |(kind, _)| kind);
            let text = |file: &str| fs::read_to_string(dir.join(file)).unwrap_or_default();
            Invocation {
                kind: kind.to_owned(),
                args: fs::read(dir.join("args"))
                    .unwrap_or_default()
                    .split(|byte| *byte == 0)
                    .map(|arg| String::from_utf8_lossy(arg).into_owned())
                    .collect::<Vec<_>>()
                    .split_last()
                    .map(|(_, args)| args.to_vec())
                    .unwrap_or_default(),
                env: text("env")
                    .lines()
                    .filter_map(|line| line.split_once('='))
                    .map(|(name, value)| (name.to_owned(), value.to_owned()))
                    .collect(),
                folder: PathBuf::from(text("folder").trim_end()),
                config_home: PathBuf::from(text("config-home").trim_end()),
                dir,
            }
        })
        .collect();
    found.sort_by(|a, b| a.dir.cmp(&b.dir));
    found
}

/// Runs `live-check <check> codex <flags>` with the stand-in, the fake user home, [`PARENT_ONLY`]
/// and [`PARENT_KEY`] in its environment. A stand-in terminal UI session keeps running until a
/// message was queued and the test listed the tools of its MCP servers.
///
/// Asserts what every run must give: the check runs and fails, as the stand-in's sessions do
/// nothing; it prints the Codex version; each Codex it runs has a config home of its own; the
/// user's Codex config stays as it was.
fn run(check: &str, flags: &[&str]) -> Run {
    let dir = tempfile::tempdir().unwrap();
    let path = |name: &str| dir.path().join(name);
    for name in ["bin", "record", "tmp"] {
        fs::create_dir(path(name)).unwrap();
    }
    let (record, home) = (path("record"), path("home"));
    user_home(&home);
    let before = snapshot(&home);
    write_executable(&path("bin").join("codex"), &stand_in(&record));

    let mut child = Command::new(env!("CARGO_BIN_EXE_live-check"))
        .arg(check)
        .arg("codex")
        .args(flags)
        .env_clear()
        .envs([
            ("PATH", format!("{}:/usr/bin:/bin", path("bin").display())),
            ("TMPDIR", path("tmp").display().to_string()),
            ("HOME", home.display().to_string()),
            (PARENT_ONLY.0, PARENT_ONLY.1.to_owned()),
            (PARENT_KEY.0, PARENT_KEY.1.to_owned()),
        ])
        .stdin(Stdio::null())
        .stdout(File::create(path("stdout")).unwrap())
        .stderr(File::create(path("stderr")).unwrap())
        .spawn()
        .expect("live-check runs");
    let started = Instant::now();
    let (status, tools) = held(&record, || {
        if started.elapsed() > RUN_LIMIT {
            let _ = child.kill();
            panic!("live-check {check} codex {flags:?} ran over {RUN_LIMIT:?}");
        }
        child.try_wait().unwrap()
    });
    let run = Run {
        stdout: fs::read_to_string(path("stdout")).unwrap_or_default(),
        stderr: fs::read_to_string(path("stderr")).unwrap_or_default(),
        invocations: recorded(&record),
        tools,
        dir,
    };
    let at = run.at();
    assert_ne!(
        status.code(),
        Some(2),
        "live-check {check} codex {flags:?} printed the usage: {at}"
    );
    assert_eq!(
        status.code(),
        Some(1),
        "live-check {check} codex {flags:?} did not fail, though Codex did nothing: {at}"
    );
    assert!(
        run.stdout.contains(VERSION),
        "live-check {check} codex {flags:?} did not print the Codex version: {at}"
    );
    assert!(
        snapshot(&home) == before,
        "live-check {check} codex {flags:?} changed the user's Codex config: {at}"
    );
    let users = fs::canonicalize(home.join(".codex")).unwrap();
    for invocation in &run.invocations {
        assert!(
            invocation.env.contains_key("CODEX_HOME")
                && !invocation.config_home.starts_with(&users),
            "live-check {check} codex {flags:?} ran `codex {}` in the user's config home: {at}",
            invocation.kind
        );
    }
    run
}

/// Polls `exited` until it gives the check's exit status. Meanwhile, once the check ran `codex
/// queue`, or [`HOLD_LIMIT`] passed, lists the tools of the terminal UI sessions' MCP servers and
/// lets those sessions end. Returns the status and the tools.
fn held(
    record: &Path,
    mut exited: impl FnMut() -> Option<ExitStatus>,
) -> (ExitStatus, Vec<Vec<String>>) {
    let since = Instant::now();
    let done = record.join("done");
    let mut tools = Vec::new();
    loop {
        if !done.exists() {
            let now = recorded(record);
            if now.iter().any(|i| i.kind == "queue") {
                tools = now
                    .iter()
                    .filter(|i| i.kind == "tui")
                    .flat_map(Invocation::mcp_servers)
                    .map(|(command, args)| list_tools(&command, &args))
                    .collect();
                fs::write(&done, "").unwrap();
            } else if since.elapsed() > HOLD_LIMIT {
                fs::write(&done, "").unwrap();
            }
        }
        if let Some(status) = exited() {
            return (status, tools);
        }
        thread::sleep(Duration::from_millis(50));
    }
}

/// The names of the tools a stdio MCP server, `command <args>`, lists, or none if it lists none
/// within 10 s.
fn list_tools(command: &str, args: &[String]) -> Vec<String> {
    let spawned = Command::new(command)
        .args(args)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let Ok(mut server) = spawned else {
        return Vec::new();
    };
    let lines = BufReader::new(server.stdout.take().unwrap()).lines();
    let reader = thread::spawn(move || {
        lines
            .map_while(Result::ok)
            .filter_map(|line| serde_json::from_str::<Value>(&line).ok())
            .find(|message| message["id"] == 2)
    });
    let mut stdin = server.stdin.take().unwrap();
    let hello = json!({
        "protocolVersion": "2025-06-18",
        "capabilities": {},
        "clientInfo": { "name": "acceptance", "version": "0" },
    });
    for request in [
        json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": hello }),
        json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {} }),
    ] {
        let _ = writeln!(stdin, "{request}");
    }
    let until = Instant::now() + Duration::from_secs(10);
    while !reader.is_finished() && Instant::now() < until {
        thread::sleep(Duration::from_millis(20));
    }
    let _ = server.kill();
    let _ = server.wait();
    drop(stdin);
    let listed = reader.join().ok().flatten().unwrap_or(Value::Null);
    listed["result"]["tools"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|tool| tool["name"].as_str().map(str::to_owned))
        .collect()
}

#[test]
fn usage_names_each_codex_check_before_implementation() {
    // Without arguments, live-check prints its usage.
    let printed = Command::new(env!("CARGO_BIN_EXE_live-check"))
        .env_clear()
        .stdin(Stdio::null())
        .output()
        .expect("live-check runs")
        .stderr;
    let usage = String::from_utf8(printed).unwrap();
    let lines: Vec<&str> = usage
        .lines()
        .filter(|line| line.contains("codex"))
        .collect();
    for check in CHECKS {
        assert!(
            lines.iter().any(|line| line.contains(check)),
            "no usage line of {check} codex:\n{usage}"
        );
    }
}

/// Asserts that each signal of `run` went over `codex queue` to the thread of one of its terminal
/// UI sessions, with that session's config home, as a line that names no sender and says that
/// messages and a task wait; returns the signals' texts.
fn assert_signals(run: &Run) -> Vec<String> {
    let at = run.at();
    let sessions = run.of("tui");
    assert!(!sessions.is_empty(), "no terminal UI session: {at}");
    let queued = run.queued();
    assert!(!queued.is_empty(), "nothing went over codex queue: {at}");
    for (thread, text, queue) in &queued {
        let session = sessions
            .iter()
            .find(|session| session.thread() == *thread)
            .unwrap_or_else(|| panic!("queued for {thread:?}, no session's thread: {at}"));
        assert_eq!(
            queue.config_home, session.config_home,
            "codex queue ran in another config home than its session's: {at}"
        );
        assert!(
            !text.contains(SENDER),
            "the signal names the sender: {text}"
        );
        let lower = text.to_lowercase();
        assert!(
            lower.contains("message") && lower.contains("task"),
            "the signal does not say that messages and a task wait: {text}"
        );
    }
    queued.into_iter().map(|(_, text, _)| text).collect()
}

#[test]
fn signal_goes_over_codex_queue_to_an_idle_session_as_a_fixed_line_without_text_or_sender() {
    let runs = [run(SIGNAL, &[]), run(SIGNAL, &[])];
    let mut signals = Vec::new();
    for run in &runs {
        signals.extend(assert_signals(run));
        let lists_both = |tools: &Vec<String>| {
            ["inbox", "claim"]
                .iter()
                .all(|name| tools.iter().any(|tool| tool == name))
        };
        assert!(
            run.tools.iter().any(lists_both),
            "no MCP server of the session lists inbox and claim: {:?}; {}",
            run.tools,
            run.at()
        );
    }
    // A fixed line: the same in every run, so no message's text or token is in it.
    assert!(
        signals.windows(2).all(|pair| pair[0] == pair[1]),
        "the signals differ: {signals:?}"
    );
}

#[test]
fn signal_with_all_also_goes_to_a_busy_session() {
    let run = run(SIGNAL, &["--all"]);
    let signals = assert_signals(&run);
    assert!(
        signals.len() >= 2,
        "no signal to the busy session: {}",
        run.at()
    );
    assert!(
        signals.windows(2).all(|pair| pair[0] == pair[1]),
        "the signals differ: {signals:?}"
    );
}

#[test]
fn signal_with_the_real_model_runs_codex_in_a_config_home_of_its_own() {
    // `run` asserts that each Codex ran in a config home of its own and the user's stayed as it was.
    let run = run(SIGNAL, &["--real"]);
    assert_signals(&run);
}

/// Runs the executor check `check` on the model stub and asserts that it runs only `codex exec`,
/// with no person, each session in a folder of the check's own and without the parent's variable
/// or key.
fn executor(check: &str) -> Run {
    let run = run(check, &[]);
    let at = run.at();
    let sessions = run.of("exec");
    assert!(!sessions.is_empty(), "no codex exec session: {at}");
    assert!(
        run.of("tui").is_empty(),
        "an executor check ran the terminal UI: {at}"
    );
    for session in sessions {
        assert!(
            session.folder.starts_with(run.tmp()),
            "the session's folder {} is not the check's own: {at}",
            session.folder.display()
        );
        assert!(
            !session.env.contains_key(PARENT_ONLY.0),
            "the parent's variable reached the session: {at}"
        );
        assert!(
            !session.env.values().any(|value| value == PARENT_KEY.1),
            "the parent's key reached the session: {at}"
        );
    }
    run
}

#[test]
fn read_only_executor_runs_codex_exec_in_the_read_only_sandbox() {
    let run = executor(READ_ONLY);
    let at = run.at();
    for session in run.of("exec") {
        assert_eq!(
            session.sandbox().as_deref(),
            Some("read-only"),
            "{:?}: {at}",
            session.args
        );
    }
}

#[test]
fn full_executor_runs_codex_exec_with_full_access() {
    let run = executor(FULL);
    let at = run.at();
    for session in run.of("exec") {
        assert_eq!(
            session.sandbox().as_deref(),
            Some("danger-full-access"),
            "{:?}: {at}",
            session.args
        );
    }
}

/// What of an MCP server, a hook, a plugin, a rule, a skill and an instruction file the config
/// home the session got, as it was when the session started, lacks where Codex reads it.
fn not_planted(session: &Invocation) -> Vec<&'static str> {
    let config = session.config();
    let read = |file: &str| fs::read_to_string(config.join(file)).unwrap_or_default();
    let toml = read("config.toml");
    let hooks: Value = serde_json::from_str(&read("hooks.json")).unwrap_or(Value::Null);
    let hook_commands = hooks["hooks"]
        .as_object()
        .into_iter()
        .flat_map(|events| events.values())
        .flat_map(|matchers| matchers.as_array().into_iter().flatten())
        .flat_map(|matcher| matcher["hooks"].as_array().into_iter().flatten())
        .filter(|hook| hook["command"].is_string())
        .count();
    let tables = |name: &str| {
        toml.lines()
            .map(str::trim_start)
            .any(|line| line.starts_with('[') && line.trim_start_matches('[').starts_with(name))
    };
    // The files of the config home, relative to it.
    let files: Vec<PathBuf> = snapshot(&config)
        .into_iter()
        .filter_map(|(path, contents)| contents.map(|_| path))
        .collect();
    let under = |dir: &str, last: &dyn Fn(&Path) -> bool| {
        files.iter().any(|file| file.starts_with(dir) && last(file))
    };
    let found = [
        ("an MCP server", tables("mcp_servers.")),
        ("a hook", hook_commands > 0 || tables("hooks")),
        (
            "a plugin",
            under("plugins", &|file| {
                file.ends_with("plugin.json")
                    && file
                        .parent()
                        .is_some_and(|dir| dir.ends_with(".codex-plugin"))
            }),
        ),
        (
            "a rule",
            under("rules", &|file| {
                file.extension().is_some_and(|ext| ext == "rules")
            }),
        ),
        (
            "a skill",
            under("skills", &|file| file.ends_with("SKILL.md")),
        ),
        ("an instruction file", !read("AGENTS.md").trim().is_empty()),
    ];
    found
        .into_iter()
        .filter_map(|(what, here)| (!here).then_some(what))
        .collect()
}

#[test]
fn clean_executor_plants_the_config_home_and_gets_none_of_the_parents_environment() {
    // `executor` asserts that no session gets the parent's variable or key.
    let run = executor(CLEAN);
    let lacks: Vec<Vec<&str>> = run.of("exec").into_iter().map(not_planted).collect();
    assert!(
        lacks.iter().any(Vec::is_empty),
        "no session's config home holds an MCP server, a hook, a plugin, a rule, a skill and an \
         instruction file; each lacks {lacks:?}: {}",
        run.at()
    );
}

/// The part of the Markdown `text` under the heading line `heading`, to the next heading of its
/// level or higher.
fn under_heading(text: &str, heading: &str) -> String {
    let level = heading.chars().take_while(|c| *c == '#').count();
    let ends = |line: &str| {
        let hashes = line.chars().take_while(|c| *c == '#').count();
        (1..=level).contains(&hashes) && line[hashes..].starts_with(' ')
    };
    let mut lines = text.lines().skip_while(|line| *line != heading);
    assert!(lines.next().is_some(), "no heading {heading:?}:\n{text}");
    lines
        .take_while(|line| !ends(line))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The section "Checks before implementation" of `docs/dev/agent-delivery.md`.
fn report() -> String {
    let doc = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../docs/dev/agent-delivery.md");
    under_heading(
        &fs::read_to_string(doc).unwrap(),
        "## Checks before implementation",
    )
}

/// Its subsection on Codex.
fn codex_report() -> String {
    under_heading(&report(), "### Codex")
}

/// Whether `text` names a Codex version, `Codex <major>.<minor>.<patch>`.
fn names_a_version(text: &str) -> bool {
    text.split("Codex ").skip(1).any(|after| {
        let word = after
            .split(|c: char| !c.is_ascii_digit() && c != '.')
            .next()
            .unwrap_or_default()
            .trim_end_matches('.');
        let numbers: Vec<&str> = word.split('.').collect();
        numbers.len() == 3 && numbers.iter().all(|n| !n.is_empty())
    })
}

/// How many times the words `pass` and `fail` are in `line`.
fn results(line: &str) -> usize {
    line.split(|c: char| !c.is_ascii_alphabetic())
        .filter(|word| *word == "pass" || *word == "fail")
        .count()
}

#[test]
fn the_report_gives_each_codex_checks_result_and_version_on_macos_and_linux() {
    let report = codex_report();
    assert!(
        names_a_version(&report),
        "no Codex version in the subsection:\n{report}"
    );
    for os in ["macOS", "Linux"] {
        assert!(report.contains(os), "no result on {os}:\n{report}");
    }
    for check in CHECKS {
        let command = format!("{check} codex");
        let lines: Vec<&str> = report
            .lines()
            .filter(|line| line.contains(&command))
            .collect();
        assert!(
            !lines.is_empty(),
            "{command} is not in the subsection:\n{report}"
        );
        // The signal on the stub and with `--real`, an executor check on macOS and in the Linux
        // container: two lines, or two columns of one row.
        let given: usize = lines.iter().map(|line| results(line)).sum();
        assert!(given >= 2, "no two results of {command}:\n{report}");
    }
}

#[test]
fn the_report_says_which_read_only_limits_codex_enforces_and_what_an_executor_takes() {
    let report = codex_report();
    let lower = report.to_lowercase();
    // No file written, no command run, no network, nothing read outside the folder.
    for limit in ["read-only", "writ", "command", "network", "outside"] {
        assert!(
            lower.contains(limit),
            "no word of read-only's limit {limit:?}:\n{report}"
        );
    }
    // The flags and environment that give an executor none of the user's MCP servers, hooks and
    // plugins.
    for item in ["mcp server", "hook", "plugin", "environment", "codex exec"] {
        assert!(lower.contains(item), "no word of {item:?}:\n{report}");
    }
}

#[test]
fn the_codex_items_of_checks_before_implementation_link_to_their_findings() {
    let report = report();
    let items: Vec<&str> = report
        .lines()
        .take_while(|line| !line.starts_with("### "))
        // An item that opens with a link to its own section has its findings there.
        .filter(|line| line.starts_with("- ") && !line.starts_with("- ["))
        .collect();
    assert!(!items.is_empty(), "no items:\n{report}");
    for item in items {
        assert!(
            item.contains("](#codex)"),
            "the item does not link to the Codex findings: {item}"
        );
    }
}
