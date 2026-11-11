//! What the live checks need from each CLI: how to attach a path to a session in a temporary project,
//! the environment it runs in, and how to read the session's events; the model stub; and the checks'
//! options, prompts, messages and verdicts; [`channel`] has the same for the Channels path and
//! [`hook`] for Claude Code's hooks. The binary only starts the CLIs and reads their output.

pub mod channel;
pub mod hook;
mod stub;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, ExitStatus};
use std::time::Duration;

use agent_inbox::INBOX;
use agent_source::{Message, Source, State};
use serde_json::{Value, json};

pub use stub::{Call, DONE, PLACEHOLDER, QUICK, Request, Response, SHELL, Stub, StubLog};

/// The name of the MCP server in each session's config.
pub const SERVER: &str = "skald";

/// The sender label of the checks' messages.
pub const SENDER: &str = "live-check of skald";

/// The variable that holds the placeholder key for Codex's stub provider.
pub const STUB_KEY: &str = "SKALD_STUB_API_KEY";

/// An agent CLI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Agent {
    Claude,
    Codex,
}

impl Agent {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "claude" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            _ => None,
        }
    }

    /// The command that runs the CLI.
    pub fn cli(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }

    /// The command that starts the CLI. Claude Code gets none of the variables of `live-check`'s
    /// environment that [`replaces_sign_in`] names, so off the stub it runs on the owner's sign-in.
    pub fn command(self) -> Command {
        let mut command = Command::new(self.cli());
        if self == Self::Claude {
            for (name, _) in std::env::vars_os() {
                if name.to_str().is_some_and(replaces_sign_in) {
                    command.env_remove(name);
                }
            }
        }
        command
    }

    /// The CLI's version line from the output of `<cli> --version`.
    pub fn version(self, stdout: &[u8]) -> String {
        format!("{}: {}", self.cli(), String::from_utf8_lossy(stdout).trim())
    }

    /// Attaches the `inbox` MCP server, `inbox <source>`, to sessions in `project` and returns the
    /// arguments of a non-interactive `kind` session there, on the model stub at `stub` if given.
    /// Only files in `project` and the arguments change the CLI's config; they load as little of
    /// the user's config as the CLI allows, at its lowest reasoning effort.
    pub fn session(
        self,
        project: &Path,
        inbox: &Path,
        source: &Path,
        kind: Kind,
        model: Option<&str>,
        stub: Option<&str>,
    ) -> io::Result<Vec<String>> {
        let inbox = inbox.to_string_lossy();
        let source = source.to_string_lossy();
        let mut args = Vec::new();
        match self {
            Self::Claude => {
                let servers = project.join(".mcp.json");
                fs::write(
                    &servers,
                    pretty(
                        &json!({ "mcpServers": { SERVER: { "command": inbox, "args": [source] } } }),
                    ),
                )?;
                // Without it, Claude Code asks a person to approve each call.
                let settings =
                    json!({ "permissions": { "allow": [inbox_tool(), "Bash(sleep:*)"] } });
                fs::create_dir_all(project.join(".claude"))?;
                fs::write(
                    project.join(".claude/settings.local.json"),
                    pretty(&settings),
                )?;
                args.extend(
                    [
                        "-p",
                        &kind.prompt(),
                        "--output-format",
                        "stream-json",
                        "--verbose",
                        // Only the project's local settings, only its MCP server, no skills.
                        "--setting-sources",
                        "local",
                        "--strict-mcp-config",
                        "--mcp-config",
                        &servers.to_string_lossy(),
                        "--disable-slash-commands",
                        // Only the built-in tools the session needs; MCP tools load at the start.
                        "--tools",
                        kind.claude_tools(),
                        "--effort",
                        "low",
                        "--no-session-persistence",
                    ]
                    .map(String::from),
                );
                if let Some(model) = model {
                    args.extend(["--model".to_owned(), model.to_owned()]);
                }
            }
            Self::Codex => {
                // `-c` values are TOML; a JSON string or array of strings is valid TOML.
                let server = format!("mcp_servers.{SERVER}");
                let mut config = vec![
                    format!("{server}.command={}", json!(inbox)),
                    format!("{server}.args={}", json!([source])),
                    // Without it, `codex exec` refuses the tool call: it cannot ask a person.
                    format!("{server}.default_tools_approval_mode=\"approve\""),
                    "skills.include_instructions=false".to_owned(),
                    "model_reasoning_effort=\"low\"".to_owned(),
                ];
                if let Some(stub) = stub {
                    let provider = "model_providers.stub";
                    config.extend([
                        "model_provider=\"stub\"".to_owned(),
                        format!("{provider}.name=\"stub\""),
                        format!("{provider}.base_url={}", json!(format!("{stub}/v1"))),
                        format!("{provider}.env_key=\"{STUB_KEY}\""),
                        format!("{provider}.wire_api=\"responses\""),
                    ]);
                }
                args.extend(
                    [
                        "exec",
                        "--json",
                        "--skip-git-repo-check",
                        "--ephemeral",
                        "--ignore-user-config",
                        "--ignore-rules",
                        "--disable",
                        "plugins",
                        "-C",
                    ]
                    .map(String::from),
                );
                args.push(project.to_string_lossy().into_owned());
                for value in config {
                    args.extend(["-c".to_owned(), value]);
                }
                if let Some(model) = model {
                    args.extend(["-m".to_owned(), model.to_owned()]);
                }
                args.push(kind.prompt());
            }
        }
        Ok(args)
    }

    /// The CLI's environment on the model stub at `stub`: only `path` as `PATH`, `home` as `HOME`
    /// with an empty config home in it, the stub's address and the placeholder key. The CLI finds
    /// no sign-in there and no credential but the placeholder.
    pub fn environment(
        self,
        home: &Path,
        stub: &str,
        path: &str,
    ) -> io::Result<Vec<(String, String)>> {
        let (config_var, config_dir) = match self {
            Self::Claude => ("CLAUDE_CONFIG_DIR", home.join(".claude")),
            Self::Codex => ("CODEX_HOME", home.join(".codex")),
        };
        fs::create_dir_all(&config_dir)?;
        let mut env = vec![
            ("PATH", path.to_owned()),
            ("HOME", home.to_string_lossy().into_owned()),
            (config_var, config_dir.to_string_lossy().into_owned()),
        ];
        match self {
            Self::Claude => env.extend([
                ("ANTHROPIC_BASE_URL", stub.to_owned()),
                ("ANTHROPIC_API_KEY", PLACEHOLDER.to_owned()),
            ]),
            Self::Codex => env.push((STUB_KEY, PLACEHOLDER.to_owned())),
        }
        Ok(env
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value))
            .collect())
    }

    /// What one line of the session's JSON output says that matters to a check.
    pub fn events(self, line: &str) -> Vec<Event> {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            return Vec::new();
        };
        let count = |value: &Value| value.as_u64().unwrap_or(0);
        match self {
            Self::Claude => match value["type"].as_str() {
                Some("system") if value["subtype"] == "init" => vec![Event::Started {
                    sign_in: value["apiKeySource"].as_str().map(String::from),
                    inbox_connected: value["mcp_servers"].as_array().is_some_and(|servers| {
                        servers.iter().any(|server| {
                            server["name"] == SERVER && server["status"] == "connected"
                        })
                    }),
                }],
                // With `--include-hook-events`.
                Some("system") => {
                    let text = |key: &str| value[key].as_str().map(String::from);
                    match (value["subtype"].as_str(), text("hook_id")) {
                        (Some("hook_started"), Some(id)) => text("hook_event")
                            .map(|event| Event::HookStarted { id, event })
                            .into_iter()
                            .collect(),
                        (Some("hook_response"), Some(id)) => vec![Event::HookEnded {
                            id,
                            outcome: text("outcome").unwrap_or_default(),
                            stdout: text("stdout").unwrap_or_default(),
                        }],
                        _ => Vec::new(),
                    }
                }
                Some("assistant") => value["message"]["content"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(
                        |block| match (block["type"].as_str(), block["name"].as_str()) {
                            (Some("tool_use"), Some("Bash")) => Some(Event::ShellStarted),
                            (Some("tool_use"), Some(name)) if name == inbox_tool() => {
                                Some(Event::InboxCalled)
                            }
                            _ => None,
                        },
                    )
                    .collect(),
                Some("result") => {
                    let usage = &value["usage"];
                    let cached = count(&usage["cache_read_input_tokens"]);
                    let mut events: Vec<_> = value["result"]
                        .as_str()
                        .map(|reply| Event::Reply(reply.to_owned()))
                        .into_iter()
                        .collect();
                    if usage.is_object() {
                        events.push(Event::Used(Usage {
                            input: count(&usage["input_tokens"])
                                + count(&usage["cache_creation_input_tokens"])
                                + cached,
                            cached,
                            output: count(&usage["output_tokens"]),
                        }));
                    }
                    events
                }
                _ => Vec::new(),
            },
            Self::Codex => {
                let item = &value["item"];
                match (value["type"].as_str(), item["type"].as_str()) {
                    (Some("item.started"), Some("command_execution")) => vec![Event::ShellStarted],
                    (Some("item.started"), Some("mcp_tool_call"))
                        if item["server"] == SERVER && item["tool"] == INBOX =>
                    {
                        vec![Event::InboxCalled]
                    }
                    (Some("item.completed"), Some("agent_message")) => item["text"]
                        .as_str()
                        .map(|text| Event::Reply(text.to_owned()))
                        .into_iter()
                        .collect(),
                    (Some("turn.completed"), _) if value["usage"].is_object() => {
                        let usage = &value["usage"];
                        vec![Event::Used(Usage {
                            input: count(&usage["input_tokens"]),
                            cached: count(&usage["cached_input_tokens"]),
                            output: count(&usage["output_tokens"]),
                        })]
                    }
                    _ => Vec::new(),
                }
            }
        }
    }
}

/// Claude Code's name of the `inbox` tool.
fn inbox_tool() -> String {
    format!("mcp__{SERVER}__{INBOX}")
}

fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).expect("JSON values serialize")
}

/// Whether a message in `state` is handed over.
fn handed_over(state: Option<State>) -> bool {
    matches!(state, Some(State::HandedOver { .. }))
}

/// A session's case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// The message waits before the session starts.
    Idle,
    /// The message comes while the agent runs a shell command.
    Busy,
}

impl Kind {
    /// The session's prompt.
    pub fn prompt(self) -> String {
        let ask = format!(
            "the {INBOX} tool of the {SERVER} MCP server and reply with only the check token from \
             the message, or NONE if there is none."
        );
        match self {
            Self::Idle => format!("Call {ask}"),
            Self::Busy => format!("Run the shell command `{SHELL}`. Then call {ask}"),
        }
    }

    /// The model stub's tool calls, in order, before it replies `DONE`.
    pub fn script(self) -> &'static [Call] {
        match self {
            Self::Idle => &[Call::Inbox],
            Self::Busy => &[Call::Shell, Call::Inbox],
        }
    }

    /// The built-in tools of a Claude Code session.
    fn claude_tools(self) -> &'static str {
        match self {
            Self::Idle => "",
            Self::Busy => "Bash",
        }
    }
}

/// The tokens a session used, as the CLI reports them.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Usage {
    /// Input tokens, cached ones included.
    pub input: u64,
    pub cached: u64,
    pub output: u64,
}

/// An event of a session that a check acts on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// Claude Code's session started: how it is signed in (`apiKeySource`, `none` for a Claude
    /// account) and whether the inbox server connected.
    Started {
        sign_in: Option<String>,
        inbox_connected: bool,
    },
    /// The agent started a shell command.
    ShellStarted,
    /// The agent called the `inbox` tool.
    InboxCalled,
    /// The agent's reply; the last one ends the turn.
    Reply(String),
    /// The tokens a turn used.
    Used(Usage),
    /// Claude Code started the hook `id` of the hook event `event`.
    HookStarted { id: String, event: String },
    /// The hook `id` ended with `outcome`, such as `success`, and printed `stdout`.
    HookEnded {
        id: String,
        outcome: String,
        stdout: String,
    },
}

/// The options of `live-check inbox`.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Options<'a> {
    /// `--model <model>`: the model of every session.
    pub model: Option<&'a str>,
    /// `--keep`: keep the sessions' logs after a pass.
    pub keep: bool,
    /// `--all`: also the busy session and the one of two idle sessions.
    pub all: bool,
    /// `--real`: the real model answers instead of the stub.
    pub real: bool,
}

impl<'a> Options<'a> {
    /// Parses `[--model <model>] [--keep] [--all] [--real]`, in any order; `None` for anything else.
    pub fn parse(mut options: &[&'a str]) -> Option<Self> {
        let mut parsed = Self::default();
        loop {
            match options {
                [] => return Some(parsed),
                ["--model", model, rest @ ..] => (parsed.model, options) = (Some(*model), rest),
                ["--keep", rest @ ..] => (parsed.keep, options) = (true, rest),
                ["--all", rest @ ..] => (parsed.all, options) = (true, rest),
                ["--real", rest @ ..] => (parsed.real, options) = (true, rest),
                _ => return None,
            }
        }
    }

    /// Whether the sessions' logs stay after a check that ended with `pass`: after a fail, always.
    pub fn keeps_logs(&self, pass: bool) -> bool {
        self.keep || !pass
    }

    /// The model stub of a session, started with `script`; none with `--real`.
    pub fn stub(&self, script: &'static [Call]) -> io::Result<Option<Stub>> {
        if self.real {
            Ok(None)
        } else {
            Stub::start(script).map(Some)
        }
    }

    /// What answers the sessions.
    pub fn model_name(&self) -> &'static str {
        if self.real {
            "the real model"
        } else {
            "the model stub"
        }
    }
}

/// Whether the variable `name` would make Claude Code sign in other than with the owner's
/// subscription: with an API key, a token, another endpoint or a cloud provider. These are every
/// `ANTHROPIC_*` and `AWS_BEARER_TOKEN_*` variable, `CLAUDE_CODE_USE_*`, and every `CLAUDE_*`
/// variable whose name holds `KEY`, `TOKEN`, `AUTH`, `SECRET` or `PROVIDER`.
pub fn replaces_sign_in(name: &str) -> bool {
    const MARKS: [&str; 5] = ["KEY", "TOKEN", "AUTH", "SECRET", "PROVIDER"];
    ["ANTHROPIC_", "AWS_BEARER_TOKEN_", "CLAUDE_CODE_USE_"]
        .iter()
        .any(|prefix| name.starts_with(prefix))
        || (name.starts_with("CLAUDE_") && MARKS.iter().any(|mark| name.contains(mark)))
}

/// The `inbox` server's binary, next to `live-check`.
pub const INBOX_BIN: &str = "agent-inbox";

/// The binary `name` of the workspace next to `live-check`, the binary `exe`.
pub fn binary(exe: &Path, name: &str) -> Result<PathBuf, String> {
    let binary = exe.with_file_name(name);
    if binary.exists() {
        Ok(binary)
    } else {
        Err(format!(
            "{} is missing: build the workspace",
            binary.display()
        ))
    }
}

/// What every session of a check shares.
#[derive(Debug, Clone, Copy)]
pub struct Setup<'a> {
    /// The check's temporary directory.
    pub work: &'a Path,
    pub agent: Agent,
    /// The `agent-inbox` binary.
    pub inbox: &'a Path,
    pub model: Option<&'a str>,
    /// `live-check`'s `PATH`, the only variable the CLI on the stub inherits.
    pub path: &'a str,
}

impl Setup<'_> {
    /// Prepares the `kind` session `name` in `work/<name>`: a project with the `inbox` server
    /// attached and, on `stub`, a home of its own.
    pub fn run(&self, name: &str, kind: Kind, stub: Option<&Stub>) -> io::Result<Run> {
        let dir = self.work.join(name);
        let project = dir.join("project");
        let source = dir.join("source");
        fs::create_dir_all(&project)?;
        let url = stub.map(Stub::url);
        let args = self.agent.session(
            &project,
            self.inbox,
            &source,
            kind,
            self.model,
            url.as_deref(),
        )?;
        let env = match &url {
            Some(url) => Some(self.agent.environment(&dir.join("home"), url, self.path)?),
            None => None,
        };
        Ok(Run {
            name: name.to_owned(),
            agent: self.agent,
            project,
            source,
            args,
            env,
            log: dir.join("session.jsonl"),
            stderr: dir.join("session.stderr"),
        })
    }
}

/// A session to run: its project, its inbox's source, its logs, the CLI's arguments and environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub name: String,
    pub agent: Agent,
    pub project: PathBuf,
    pub source: PathBuf,
    pub args: Vec<String>,
    /// The CLI's whole environment; `None` inherits `live-check`'s, for the real model's sign-in,
    /// but for what [`Agent::command`] drops.
    pub env: Option<Vec<(String, String)>>,
    /// The session's JSON output.
    pub log: PathBuf,
    /// The CLI's stderr.
    pub stderr: PathBuf,
}

/// Why a session ended badly, if it did: it was stopped after `timeout`, or its CLI failed.
pub fn session_failure(timed_out: bool, timeout: Duration, status: ExitStatus) -> Option<String> {
    if timed_out {
        Some(format!("timed out after {} s", timeout.as_secs()))
    } else if status.success() {
        None
    } else {
        Some(status.to_string())
    }
}

/// A random check token.
pub fn token() -> String {
    format!("tok-{}", &uuid::Uuid::new_v4().simple().to_string()[..12])
}

/// Adds a check's message with `token` to `source`.
pub fn add_message(source: &Source, token: &str) -> io::Result<Message> {
    source.add(SENDER, &format!("The check token is {token}."))
}

/// The busy check's message: added once, when the agent starts its first shell command.
#[derive(Debug)]
pub struct BusyMessage<T>(Option<T>);

impl<T> Default for BusyMessage<T> {
    fn default() -> Self {
        Self(None)
    }
}

impl<T> BusyMessage<T> {
    /// Adds the message with `add` if `event` starts the session's first shell command.
    pub fn on_event(&mut self, event: &Event, add: impl FnOnce() -> T) {
        if *event == Event::ShellStarted && self.0.is_none() {
            self.0 = Some(add());
        }
    }

    /// What `add` returned; `None` if the agent ran no shell command.
    pub fn added(self) -> Option<T> {
        self.0
    }
}

/// What a session did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Session {
    /// The agent's last reply.
    pub reply: String,
    /// Claude Code's `apiKeySource`.
    pub sign_in: Option<String>,
    /// Whether the inbox server connected, if the CLI says.
    pub inbox_connected: Option<bool>,
    pub inbox_called: bool,
    /// The tokens it used, if the CLI says.
    pub usage: Option<Usage>,
    /// What the model stub saw; `None` on the real model.
    pub stub: Option<StubLog>,
    /// When the session started, in milliseconds since the Unix epoch.
    pub started_ms: u64,
}

impl Session {
    pub fn new(started_ms: u64) -> Self {
        Self {
            started_ms,
            ..Self::default()
        }
    }

    /// Records what `event` says about the session.
    pub fn record(&mut self, event: &Event) {
        match event {
            Event::Started {
                sign_in,
                inbox_connected,
            } => {
                self.sign_in = sign_in.clone();
                self.inbox_connected = Some(*inbox_connected);
            }
            Event::InboxCalled => self.inbox_called = true,
            Event::Reply(text) => self.reply = text.clone(),
            Event::Used(used) => {
                let usage = self.usage.get_or_insert_default();
                usage.input += used.input;
                usage.cached += used.cached;
                usage.output += used.output;
            }
            Event::ShellStarted | Event::HookStarted { .. } | Event::HookEnded { .. } => {}
        }
    }

    /// Whether `token` reached the model: in a request to the stub, or in the real model's reply.
    fn got(&self, token: &str) -> bool {
        match &self.stub {
            Some(stub) => stub.saw(token),
            None => self.reply.contains(token),
        }
    }

    /// Whether the stub got no credential but the placeholder.
    fn clean(&self) -> bool {
        self.stub
            .as_ref()
            .is_none_or(|stub| stub.failure().is_none())
    }

    /// The idle and busy checks: the session called `inbox`, `token` reached the model and the
    /// message with it, in `state`, was handed over. A message never added has no state.
    pub fn received(&self, token: &str, state: Option<State>) -> bool {
        self.clean() && self.inbox_called && self.got(token) && handed_over(state)
    }

    /// The other idle session's check: it called `inbox` and `token` did not reach the model.
    pub fn missed(&self, token: &str) -> bool {
        self.clean() && self.inbox_called && !self.got(token)
    }

    pub fn summary(&self, token: &str) -> String {
        let mut summary = self.head();
        if self.inbox_connected == Some(false) {
            summary.push_str(", the inbox server did not connect");
        }
        if !self.inbox_called {
            summary.push_str(", inbox not called");
        }
        summary.push_str(&self.tail(token));
        summary
    }

    /// The start of a summary: the reply and the sign-in.
    fn head(&self) -> String {
        let mut head = format!("reply {:?}", self.reply.trim());
        if let Some(sign_in) = &self.sign_in {
            head.push_str(&format!(", apiKeySource {sign_in}"));
        }
        head
    }

    /// The end of a summary: what the stub saw, or the tokens the real model used.
    fn tail(&self, token: &str) -> String {
        match &self.stub {
            Some(stub) => {
                let reached = if stub.saw(token) {
                    "reached"
                } else {
                    "did not reach"
                };
                let mut tail = format!(
                    ", requests to the stub: {}, the token {reached} it",
                    stub.requests.len()
                );
                if let Some(failure) = stub.failure() {
                    tail.push_str(&format!(", {failure}"));
                }
                tail
            }
            None => self.usage.map_or_else(String::new, |usage| {
                format!(
                    ", tokens: input {} (cached {}), output {}",
                    usage.input, usage.cached, usage.output
                )
            }),
        }
    }
}

/// How long after `from_ms` a message in `state` was handed over.
pub fn handover_delay(state: Option<State>, from_ms: u64) -> String {
    match state {
        Some(State::HandedOver { at_ms }) => {
            format!("{:.1} s", at_ms.saturating_sub(from_ms) as f64 / 1000.0)
        }
        other => format!("never (state {other:?})"),
    }
}

/// The `inbox` path's checks: each returns its line in the output.
#[derive(Debug, Default)]
pub struct InboxCheck {
    passed: Vec<bool>,
}

impl InboxCheck {
    /// The idle session and its message with `token`, now in `state`.
    pub fn idle(&mut self, session: &Session, token: &str, state: Option<State>) -> String {
        let details = format!(
            "{}; handed over {} after the session started",
            session.summary(token),
            handover_delay(state, session.started_ms)
        );
        self.report("idle session", session.received(token, state), &details)
    }

    /// The other of two idle sessions, which must not get the message with `token`.
    pub fn other(&mut self, session: &Session, token: &str) -> String {
        let details = format!(
            "{}; the message is the first session's",
            session.summary(token)
        );
        self.report(
            "the other of two idle sessions",
            session.missed(token),
            &details,
        )
    }

    /// The busy session and its message with `token`: added at `added_at_ms`, if the agent ran a
    /// shell command, and now in `state`.
    pub fn busy(
        &mut self,
        session: &Session,
        token: &str,
        added_at_ms: Option<u64>,
        state: Option<State>,
    ) -> String {
        let message = match added_at_ms {
            Some(added_at_ms) => format!(
                "added while the shell command ran, handed over {} later",
                handover_delay(state, added_at_ms)
            ),
            None => "the agent ran no shell command".to_owned(),
        };
        let details = format!("{}; {message}", session.summary(token));
        self.report("busy session", session.received(token, state), &details)
    }

    /// Whether every check passed.
    pub fn pass(&self) -> bool {
        self.passed.iter().all(|pass| *pass)
    }

    /// The last line of the output.
    pub fn verdict(&self, agent: Agent, options: &Options) -> String {
        format!(
            "{INBOX} on {} with {}: {}",
            agent.cli(),
            options.model_name(),
            verdict(self.pass())
        )
    }

    fn report(&mut self, check: &str, pass: bool, details: &str) -> String {
        self.passed.push(pass);
        report(check, pass, details)
    }
}

/// A check's line in the output.
fn report(check: &str, pass: bool, details: &str) -> String {
    format!("  {check}: {} — {details}", verdict(pass))
}

fn verdict(pass: bool) -> &'static str {
    if pass { "pass" } else { "fail" }
}

/// `live-check`'s exit code after a check that ended with `pass`.
pub fn exit_code(pass: bool) -> ExitCode {
    if pass {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Lines recorded from Claude Code 2.1.322 (`--output-format stream-json --verbose`), shortened.
    const CLAUDE_INIT: &str = r#"{"type":"system","subtype":"init","mcp_servers":[{"name":"exa","status":"connected","source":"user"},{"name":"skald","status":"connected","source":"dynamic"}],"apiKeySource":"none","claude_code_version":"2.1.322"}"#;
    const CLAUDE_INIT_FAILED: &str = r#"{"type":"system","subtype":"init","mcp_servers":[{"name":"skald","status":"failed","source":"dynamic"}],"apiKeySource":"ANTHROPIC_API_KEY"}"#;
    const CLAUDE_INIT_WITHOUT_INBOX: &str = r#"{"type":"system","subtype":"init","mcp_servers":[{"name":"exa","status":"connected","source":"user"},{"name":"skald","status":"pending","source":"dynamic"}],"apiKeySource":"none"}"#;
    const CLAUDE_INIT_NO_SERVERS: &str =
        r#"{"type":"system","subtype":"init","apiKeySource":"none"}"#;
    const CLAUDE_HOOK: &str = r#"{"type":"system","subtype":"hook_response","mcp_servers":[{"name":"skald","status":"connected"}]}"#;
    const CLAUDE_SEARCH: &str = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_01","name":"ToolSearch","input":{"query":"select:mcp__skald__inbox","max_results":1}}]}}"#;
    const CLAUDE_INBOX: &str = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_02","name":"mcp__skald__inbox","input":{}}]}}"#;
    const CLAUDE_BASH: &str = r#"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":""},{"type":"tool_use","id":"toolu_03","name":"Bash","input":{"command":"sleep 20"}}]}}"#;
    const CLAUDE_BOTH: &str = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_03","name":"Bash","input":{"command":"sleep 20"}},{"type":"tool_use","id":"toolu_02","name":"mcp__skald__inbox","input":{}}]}}"#;
    const CLAUDE_TEXT: &str =
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"tok-7f3a91"}]}}"#;
    const CLAUDE_RESULT: &str = r#"{"type":"result","subtype":"success","is_error":false,"result":"tok-7f3a91","stop_reason":"end_turn","usage":{"input_tokens":6,"cache_creation_input_tokens":3900,"cache_read_input_tokens":15200,"output_tokens":42}}"#;
    const CLAUDE_RESULT_WITHOUT_USAGE: &str =
        r#"{"type":"result","subtype":"error_during_execution","is_error":true}"#;
    // With `--include-hook-events`.
    const CLAUDE_HOOK_STARTED: &str = r#"{"type":"system","subtype":"hook_started","hook_id":"c899","hook_name":"PostToolUse:Bash","hook_event":"PostToolUse","uuid":"32d0","session_id":"3b19"}"#;
    const CLAUDE_HOOK_ENDED: &str = r#"{"type":"system","subtype":"hook_response","hook_id":"c899","hook_name":"PostToolUse:Bash","hook_event":"PostToolUse","output":"{}\n","stdout":"{}\n","stderr":"","exit_code":0,"outcome":"success","uuid":"9600","session_id":"3b19"}"#;

    // Lines recorded from Codex 0.162.1 (`codex exec --json`), shortened.
    const CODEX_INBOX: &str = r#"{"type":"item.started","item":{"id":"item_2","type":"mcp_tool_call","server":"skald","tool":"inbox","arguments":{},"result":null,"error":null,"status":"in_progress"}}"#;
    const CODEX_INBOX_DONE: &str = r#"{"type":"item.completed","item":{"id":"item_2","type":"mcp_tool_call","server":"skald","tool":"inbox","arguments":{},"result":{"content":[{"type":"text","text":"No messages are waiting."}]},"error":null,"status":"completed"}}"#;
    const CODEX_OTHER_TOOL: &str = r#"{"type":"item.started","item":{"id":"item_4","type":"mcp_tool_call","server":"exa","tool":"inbox","arguments":{},"status":"in_progress"}}"#;
    const CODEX_OTHER_SERVER_TOOL: &str = r#"{"type":"item.started","item":{"id":"item_4","type":"mcp_tool_call","server":"skald","tool":"outbox","arguments":{},"status":"in_progress"}}"#;
    const CODEX_SHELL: &str = r#"{"type":"item.started","item":{"id":"item_1","type":"command_execution","command":"/bin/zsh -lc 'sleep 20'","aggregated_output":"","exit_code":null,"status":"in_progress"}}"#;
    const CODEX_SHELL_DONE: &str = r#"{"type":"item.completed","item":{"id":"item_1","type":"command_execution","command":"/bin/zsh -lc 'sleep 20'","exit_code":0,"status":"completed"}}"#;
    const CODEX_REPLY: &str = r#"{"type":"item.completed","item":{"id":"item_3","type":"agent_message","text":"tok-c0d3x2"}}"#;
    const CODEX_ERROR: &str = r#"{"type":"item.completed","item":{"id":"item_0","type":"error","message":"Codex is ignoring 1 unrecognized configuration setting."}}"#;
    const CODEX_DONE: &str = r#"{"type":"turn.completed","usage":{"input_tokens":44124,"cached_input_tokens":3456,"cache_write_input_tokens":0,"output_tokens":118,"reasoning_output_tokens":64}}"#;
    const CODEX_STARTED: &str = r#"{"type":"turn.started"}"#;
    const CODEX_DONE_WITHOUT_USAGE: &str = r#"{"type":"turn.completed"}"#;

    #[test]
    fn reads_claude_code_events() {
        let claude = Agent::Claude;
        assert_eq!(
            claude.events(CLAUDE_INIT),
            [Event::Started {
                sign_in: Some("none".to_owned()),
                inbox_connected: true
            }]
        );
        assert_eq!(
            claude.events(CLAUDE_INIT_FAILED),
            [Event::Started {
                sign_in: Some("ANTHROPIC_API_KEY".to_owned()),
                inbox_connected: false
            }]
        );
        assert_eq!(
            claude.events(CLAUDE_INIT_WITHOUT_INBOX),
            [Event::Started {
                sign_in: Some("none".to_owned()),
                inbox_connected: false
            }]
        );
        assert_eq!(
            claude.events(CLAUDE_INIT_NO_SERVERS),
            [Event::Started {
                sign_in: Some("none".to_owned()),
                inbox_connected: false
            }]
        );
        assert_eq!(claude.events(CLAUDE_HOOK), []);
        assert_eq!(
            claude.events(CLAUDE_HOOK_STARTED),
            [Event::HookStarted {
                id: "c899".to_owned(),
                event: "PostToolUse".to_owned()
            }]
        );
        assert_eq!(
            claude.events(CLAUDE_HOOK_ENDED),
            [Event::HookEnded {
                id: "c899".to_owned(),
                outcome: "success".to_owned(),
                stdout: "{}\n".to_owned()
            }]
        );
        let bare = r#"{"type":"system","subtype":"hook_response","hook_id":"h"}"#;
        assert_eq!(
            claude.events(bare),
            [Event::HookEnded {
                id: "h".to_owned(),
                outcome: String::new(),
                stdout: String::new()
            }]
        );
        for line in [
            r#"{"type":"system","subtype":"hook_started","hook_event":"Stop"}"#,
            r#"{"type":"system","subtype":"hook_started","hook_id":"h"}"#,
            r#"{"type":"system","subtype":"hook_response","hook_event":"Stop"}"#,
            r#"{"type":"system","subtype":"hook_progress","hook_id":"h","hook_event":"Stop"}"#,
            r#"{"type":"system","subtype":"status","hook_id":"h","hook_event":"Stop"}"#,
        ] {
            assert_eq!(claude.events(line), [], "{line}");
        }
        assert_eq!(claude.events(CLAUDE_SEARCH), []);
        assert_eq!(claude.events(CLAUDE_INBOX), [Event::InboxCalled]);
        assert_eq!(claude.events(CLAUDE_BASH), [Event::ShellStarted]);
        assert_eq!(
            claude.events(CLAUDE_BOTH),
            [Event::ShellStarted, Event::InboxCalled]
        );
        assert_eq!(claude.events(CLAUDE_TEXT), []);
        assert_eq!(
            claude.events(CLAUDE_RESULT),
            [
                Event::Reply("tok-7f3a91".to_owned()),
                Event::Used(Usage {
                    input: 19_106,
                    cached: 15_200,
                    output: 42
                })
            ]
        );
        assert_eq!(claude.events(CLAUDE_RESULT_WITHOUT_USAGE), []);
        assert_eq!(claude.events("not JSON"), []);
        // Codex's events mean nothing to Claude Code.
        assert_eq!(claude.events(CODEX_REPLY), []);
        assert_eq!(claude.events(CODEX_DONE), []);
    }

    #[test]
    fn reads_codex_events() {
        let codex = Agent::Codex;
        assert_eq!(codex.events(CODEX_INBOX), [Event::InboxCalled]);
        assert_eq!(codex.events(CODEX_INBOX_DONE), []);
        assert_eq!(codex.events(CODEX_OTHER_TOOL), []);
        assert_eq!(codex.events(CODEX_OTHER_SERVER_TOOL), []);
        assert_eq!(codex.events(CODEX_SHELL), [Event::ShellStarted]);
        assert_eq!(codex.events(CODEX_SHELL_DONE), []);
        assert_eq!(
            codex.events(CODEX_REPLY),
            [Event::Reply("tok-c0d3x2".to_owned())]
        );
        assert_eq!(codex.events(CODEX_ERROR), []);
        assert_eq!(
            codex.events(CODEX_DONE),
            [Event::Used(Usage {
                input: 44_124,
                cached: 3_456,
                output: 118
            })]
        );
        assert_eq!(codex.events(CODEX_STARTED), []);
        assert_eq!(codex.events(CODEX_DONE_WITHOUT_USAGE), []);
        assert_eq!(codex.events(CLAUDE_RESULT), []);
        assert_eq!(codex.events("not JSON"), []);
    }

    #[test]
    fn names_the_variables_that_replace_the_sign_in() {
        for name in [
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_AUTH_TOKEN",
            "ANTHROPIC_BASE_URL",
            "ANTHROPIC_FOUNDRY_API_KEY",
            "AWS_BEARER_TOKEN_BEDROCK",
            "CLAUDE_CODE_USE_BEDROCK",
            "CLAUDE_CODE_USE_VERTEX",
            "CLAUDE_CODE_OAUTH_TOKEN",
            "CLAUDE_CODE_API_KEY_FILE_DESCRIPTOR",
            "CLAUDE_CODE_GATEWAY_TOKEN",
            "CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST",
            "CLAUDE_API_KEY",
            "CLAUDE_SECRET",
        ] {
            assert!(replaces_sign_in(name), "{name}");
        }
        for name in [
            "PATH",
            "HOME",
            "USER",
            "TERM",
            "TMPDIR",
            "CLAUDE_CONFIG_DIR",
            "CLAUDE_CODE_CHILD_SESSION",
            "OPENAI_API_KEY",
            "SKALD_STUB_API_KEY",
            "MY_ANTHROPIC_API_KEY",
        ] {
            assert!(!replaces_sign_in(name), "{name}");
        }
    }

    #[test]
    fn parses_agent_names() {
        assert_eq!(Agent::parse("claude"), Some(Agent::Claude));
        assert_eq!(Agent::parse("codex"), Some(Agent::Codex));
        assert_eq!(Agent::parse("gemini"), None);
        assert_eq!(Agent::Claude.cli(), "claude");
        assert_eq!(Agent::Codex.cli(), "codex");
    }

    #[test]
    fn claude_code_session_loads_only_the_project_config() {
        let project = tempfile::tempdir().unwrap();
        let servers = project.path().join(".mcp.json");
        let servers = servers.to_str().unwrap();
        let args = Agent::Claude
            .session(
                project.path(),
                Path::new("/bin/inbox"),
                Path::new("/tmp/source"),
                Kind::Idle,
                Some("haiku"),
                None,
            )
            .unwrap();
        assert_eq!(
            args,
            [
                "-p",
                &Kind::Idle.prompt(),
                "--output-format",
                "stream-json",
                "--verbose",
                "--setting-sources",
                "local",
                "--strict-mcp-config",
                "--mcp-config",
                servers,
                "--disable-slash-commands",
                "--tools",
                "",
                "--effort",
                "low",
                "--no-session-persistence",
                "--model",
                "haiku"
            ]
        );
        let config: Value = serde_json::from_slice(&fs::read(servers).unwrap()).unwrap();
        assert_eq!(
            config,
            json!({ "mcpServers": { "skald": { "command": "/bin/inbox", "args": ["/tmp/source"] } } })
        );
        let settings: Value = serde_json::from_slice(
            &fs::read(project.path().join(".claude/settings.local.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            settings,
            json!({ "permissions": { "allow": ["mcp__skald__inbox", "Bash(sleep:*)"] } })
        );

        // The stub changes nothing in Claude Code's arguments: it is in the environment.
        let busy = Agent::Claude
            .session(
                project.path(),
                Path::new("/bin/inbox"),
                Path::new("/s"),
                Kind::Busy,
                None,
                Some("http://127.0.0.1:1"),
            )
            .unwrap();
        assert_eq!(busy[1], Kind::Busy.prompt());
        assert_eq!(busy[11..13], ["--tools", "Bash"]);
        assert_eq!(busy.len(), 16, "{busy:?}");
    }

    #[test]
    fn codex_session_sets_config_by_arguments() {
        let project = tempfile::tempdir().unwrap();
        let args = Agent::Codex
            .session(
                project.path(),
                Path::new("/bin/my inbox"),
                Path::new("/tmp/\"source\""),
                Kind::Busy,
                Some("gpt-5.5"),
                None,
            )
            .unwrap();
        let project = project.path().to_string_lossy().into_owned();
        let common = [
            "exec",
            "--json",
            "--skip-git-repo-check",
            "--ephemeral",
            "--ignore-user-config",
            "--ignore-rules",
            "--disable",
            "plugins",
            "-C",
            &project,
            "-c",
            r#"mcp_servers.skald.command="/bin/my inbox""#,
            "-c",
            r#"mcp_servers.skald.args=["/tmp/\"source\""]"#,
            "-c",
            r#"mcp_servers.skald.default_tools_approval_mode="approve""#,
            "-c",
            "skills.include_instructions=false",
            "-c",
            r#"model_reasoning_effort="low""#,
        ];
        assert_eq!(
            args,
            [&common[..], &["-m", "gpt-5.5", &Kind::Busy.prompt()]].concat()
        );
        assert_eq!(
            fs::read_dir(&project).unwrap().count(),
            0,
            "no files in the project"
        );

        let args = Agent::Codex
            .session(
                Path::new(&project),
                Path::new("/bin/my inbox"),
                Path::new("/tmp/\"source\""),
                Kind::Idle,
                None,
                Some("http://127.0.0.1:4100"),
            )
            .unwrap();
        assert_eq!(
            args,
            [
                &common[..],
                &[
                    "-c",
                    r#"model_provider="stub""#,
                    "-c",
                    r#"model_providers.stub.name="stub""#,
                    "-c",
                    r#"model_providers.stub.base_url="http://127.0.0.1:4100/v1""#,
                    "-c",
                    r#"model_providers.stub.env_key="SKALD_STUB_API_KEY""#,
                    "-c",
                    r#"model_providers.stub.wire_api="responses""#,
                    &Kind::Idle.prompt(),
                ]
            ]
            .concat()
        );
    }

    #[test]
    fn the_cli_on_the_stub_gets_only_the_placeholder_and_an_empty_config_home() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path().join("home");
        let at = |dir: &str| home.join(dir).to_string_lossy().into_owned();
        let owned = |pairs: &[(&str, &str)]| -> Vec<(String, String)> {
            pairs
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect()
        };
        let home_str = home.to_string_lossy().into_owned();
        assert_eq!(
            Agent::Claude
                .environment(&home, "http://127.0.0.1:1", "/bin:/usr/bin")
                .unwrap(),
            owned(&[
                ("PATH", "/bin:/usr/bin"),
                ("HOME", &home_str),
                ("CLAUDE_CONFIG_DIR", &at(".claude")),
                ("ANTHROPIC_BASE_URL", "http://127.0.0.1:1"),
                ("ANTHROPIC_API_KEY", PLACEHOLDER),
            ])
        );
        assert_eq!(
            Agent::Codex
                .environment(&home, "http://127.0.0.1:1", "/bin")
                .unwrap(),
            owned(&[
                ("PATH", "/bin"),
                ("HOME", &home_str),
                ("CODEX_HOME", &at(".codex")),
                ("SKALD_STUB_API_KEY", PLACEHOLDER),
            ])
        );
        for config in [".claude", ".codex"] {
            assert_eq!(
                fs::read_dir(home.join(config)).unwrap().count(),
                0,
                "{config} is an empty directory"
            );
        }
    }

    #[test]
    fn parses_options() {
        assert_eq!(Options::parse(&[]), Some(Options::default()));
        assert_eq!(
            Options::parse(&["--keep"]),
            Some(Options {
                keep: true,
                ..Options::default()
            })
        );
        assert_eq!(
            Options::parse(&["--model", "gpt-5.5"]),
            Some(Options {
                model: Some("gpt-5.5"),
                ..Options::default()
            })
        );
        assert_eq!(
            Options::parse(&["--all"]),
            Some(Options {
                all: true,
                ..Options::default()
            })
        );
        assert_eq!(
            Options::parse(&["--real"]),
            Some(Options {
                real: true,
                ..Options::default()
            })
        );
        assert_eq!(
            Options::parse(&["--real", "--keep", "--model", "haiku", "--all"]),
            Some(Options {
                model: Some("haiku"),
                keep: true,
                all: true,
                real: true,
            })
        );
        assert_eq!(Options::parse(&["--model"]), None);
        assert_eq!(Options::parse(&["--keep", "--verbose"]), None);
        assert_eq!(Options::parse(&["gpt-5.5"]), None);
    }

    #[test]
    fn starts_the_stub_unless_the_model_is_real() {
        let real = Options {
            real: true,
            ..Options::default()
        };
        assert!(real.stub(Kind::Idle.script()).unwrap().is_none());
        assert_eq!(real.model_name(), "the real model");
        let stub = Options::default()
            .stub(Kind::Busy.script())
            .unwrap()
            .unwrap();
        assert!(stub.url().starts_with("http://127.0.0.1:"));
        assert_eq!(Options::default().model_name(), "the model stub");
    }

    #[test]
    fn scripts_the_stub_and_the_prompts_per_case() {
        assert_eq!(Kind::Idle.script(), [Call::Inbox]);
        assert_eq!(Kind::Busy.script(), [Call::Shell, Call::Inbox]);
        assert_eq!(
            Kind::Idle.prompt(),
            "Call the inbox tool of the skald MCP server and reply with only the check token \
             from the message, or NONE if there is none."
        );
        assert_eq!(
            Kind::Busy.prompt(),
            "Run the shell command `sleep 20`. Then call the inbox tool of the skald MCP server \
             and reply with only the check token from the message, or NONE if there is none."
        );
    }

    /// A session that called `inbox` and replied `reply`.
    fn replied(reply: &str) -> Session {
        let mut session = Session::new(1_000);
        session.record(&Event::InboxCalled);
        session.record(&Event::Reply(reply.to_owned()));
        session
    }

    /// A session on the stub that called `inbox`; `token` reached the stub if given.
    fn stubbed(token: Option<&str>, credentials: &[&str]) -> Session {
        let mut session = replied(DONE);
        session.stub = Some(StubLog {
            requests: vec![
                "{}".to_owned(),
                format!("{{\"text\":\"{}\"}}", token.unwrap_or("none")),
            ],
            credentials: credentials.iter().map(|name| (*name).to_owned()).collect(),
        });
        session
    }

    #[test]
    fn records_session_events() {
        let mut session = Session::new(1_000);
        assert_eq!(session.started_ms, 1_000);
        assert!(!session.inbox_called);
        session.record(&Event::Started {
            sign_in: Some("none".to_owned()),
            inbox_connected: false,
        });
        session.record(&Event::ShellStarted);
        session.record(&Event::HookStarted {
            id: "h".to_owned(),
            event: "Stop".to_owned(),
        });
        session.record(&Event::HookEnded {
            id: "h".to_owned(),
            outcome: "success".to_owned(),
            stdout: "tok-0".to_owned(),
        });
        session.record(&Event::Reply("working".to_owned()));
        session.record(&Event::InboxCalled);
        session.record(&Event::Reply("tok-1".to_owned()));
        session.record(&Event::Used(Usage {
            input: 100,
            cached: 60,
            output: 5,
        }));
        session.record(&Event::Used(Usage {
            input: 10,
            cached: 6,
            output: 1,
        }));
        assert_eq!(
            session,
            Session {
                reply: "tok-1".to_owned(),
                sign_in: Some("none".to_owned()),
                inbox_connected: Some(false),
                inbox_called: true,
                usage: Some(Usage {
                    input: 110,
                    cached: 66,
                    output: 6
                }),
                stub: None,
                started_ms: 1_000,
            }
        );
    }

    #[test]
    fn on_the_real_model_a_session_receives_a_message_it_repeats_after_the_handover() {
        let handed_over = Some(State::HandedOver { at_ms: 2_000 });
        assert!(replied("The token is tok-1.").received("tok-1", handed_over));
        assert!(!replied("NONE").received("tok-1", handed_over));
        assert!(!replied("tok-1").received("tok-1", Some(State::Taken)));
        assert!(!replied("tok-1").received("tok-1", Some(State::Waiting)));
        // The busy check's message was never added.
        assert!(!replied("tok-1").received("tok-1", None));
        let mut uncalled = Session::new(1_000);
        uncalled.record(&Event::Reply("tok-1".to_owned()));
        assert!(!uncalled.received("tok-1", handed_over));
    }

    #[test]
    fn on_the_stub_a_session_receives_a_message_whose_token_reaches_the_stub() {
        let handed_over = Some(State::HandedOver { at_ms: 2_000 });
        assert!(stubbed(Some("tok-1"), &[]).received("tok-1", handed_over));
        assert!(!stubbed(None, &[]).received("tok-1", handed_over));
        assert!(!stubbed(Some("tok-1"), &[]).received("tok-1", Some(State::Taken)));
        // The reply does not count on the stub.
        let mut repeated = stubbed(None, &[]);
        repeated.reply = "tok-1".to_owned();
        assert!(!repeated.received("tok-1", handed_over));
        // A credential other than the placeholder fails the session.
        assert!(!stubbed(Some("tok-1"), &["x-api-key"]).received("tok-1", handed_over));
        let mut uncalled = stubbed(Some("tok-1"), &[]);
        uncalled.inbox_called = false;
        assert!(!uncalled.received("tok-1", handed_over));
    }

    #[test]
    fn the_other_session_misses_a_message_after_calling_inbox() {
        assert!(replied("NONE").missed("tok-1"));
        assert!(!replied("tok-1").missed("tok-1"));
        assert!(!Session::new(1_000).missed("tok-1"));
        assert!(stubbed(None, &[]).missed("tok-1"));
        assert!(!stubbed(Some("tok-1"), &[]).missed("tok-1"));
        assert!(!stubbed(None, &["authorization"]).missed("tok-1"));
        let mut uncalled = stubbed(None, &[]);
        uncalled.inbox_called = false;
        assert!(!uncalled.missed("tok-1"));
    }

    #[test]
    fn summarizes_a_session() {
        assert_eq!(replied(" tok-1\n").summary("tok-1"), r#"reply "tok-1""#);
        let mut session = Session::new(1_000);
        session.record(&Event::Started {
            sign_in: Some("none".to_owned()),
            inbox_connected: false,
        });
        assert_eq!(
            session.summary("tok-1"),
            r#"reply "", apiKeySource none, the inbox server did not connect, inbox not called"#
        );
        session.record(&Event::Started {
            sign_in: None,
            inbox_connected: true,
        });
        session.record(&Event::Used(Usage {
            input: 19_106,
            cached: 15_200,
            output: 42,
        }));
        assert_eq!(
            session.summary("tok-1"),
            r#"reply "", inbox not called, tokens: input 19106 (cached 15200), output 42"#
        );
        assert_eq!(
            stubbed(Some("tok-1"), &[]).summary("tok-1"),
            r#"reply "Done.", requests to the stub: 2, the token reached it"#
        );
        // On the stub, the CLI's token counts are the stub's, so they are not shown.
        let mut session = stubbed(None, &["x-api-key"]);
        session.record(&Event::Used(Usage::default()));
        assert_eq!(
            session.summary("tok-1"),
            r#"reply "Done.", requests to the stub: 2, the token did not reach it, the stub got a credential other than the placeholder in x-api-key"#
        );
    }

    #[test]
    fn measures_the_handover_delay() {
        assert_eq!(
            handover_delay(Some(State::HandedOver { at_ms: 5_900 }), 1_000),
            "4.9 s"
        );
        assert_eq!(
            handover_delay(Some(State::HandedOver { at_ms: 22_600 }), 1_000),
            "21.6 s"
        );
        assert_eq!(
            handover_delay(Some(State::HandedOver { at_ms: 500 }), 1_000),
            "0.0 s"
        );
        assert_eq!(
            handover_delay(Some(State::Taken), 1_000),
            "never (state Some(Taken))"
        );
        assert_eq!(handover_delay(None, 1_000), "never (state None)");
    }

    #[test]
    fn reads_the_cli_version() {
        assert_eq!(
            Agent::Claude.version(b"2.1.322 (Claude Code)\n"),
            "claude: 2.1.322 (Claude Code)"
        );
        assert_eq!(
            Agent::Codex.version(b"codex-cli 0.162.1\n"),
            "codex: codex-cli 0.162.1"
        );
    }

    #[test]
    fn keeps_the_logs_when_asked_or_after_a_fail() {
        let keep = Options {
            keep: true,
            ..Options::default()
        };
        assert!(!Options::default().keeps_logs(true));
        assert!(Options::default().keeps_logs(false));
        assert!(keep.keeps_logs(true));
        assert!(keep.keeps_logs(false));
    }

    #[test]
    fn finds_a_binary_next_to_live_check() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("live-check");
        let inbox = dir.path().join(INBOX_BIN);
        assert_eq!(
            binary(&exe, INBOX_BIN),
            Err(format!(
                "{} is missing: build the workspace",
                inbox.display()
            ))
        );
        fs::write(&inbox, "").unwrap();
        assert_eq!(binary(&exe, INBOX_BIN), Ok(inbox));
        assert!(binary(&exe, "agent-other").is_err());
    }

    #[test]
    fn prepares_a_session_in_its_own_directory() {
        let work = tempfile::tempdir().unwrap();
        let inbox = Path::new("/bin/inbox");
        let setup = Setup {
            work: work.path(),
            agent: Agent::Codex,
            inbox,
            model: Some("m"),
            path: "/bin",
        };
        let run = setup.run("a", Kind::Busy, None).unwrap();
        let dir = work.path().join("a");
        let project = dir.join("project");
        let source = dir.join("source");
        assert!(project.is_dir());
        assert_eq!(
            run,
            Run {
                name: "a".to_owned(),
                agent: Agent::Codex,
                args: Agent::Codex
                    .session(&project, inbox, &source, Kind::Busy, Some("m"), None)
                    .unwrap(),
                env: None,
                project,
                source,
                log: dir.join("session.jsonl"),
                stderr: dir.join("session.stderr"),
            }
        );

        let stub = Stub::start(Kind::Idle.script()).unwrap();
        let claude = Setup {
            agent: Agent::Claude,
            model: None,
            ..setup
        };
        let run = claude.run("b", Kind::Idle, Some(&stub)).unwrap();
        let dir = work.path().join("b");
        assert_eq!(run.args[..2], ["-p", &Kind::Idle.prompt()]);
        assert!(run.project.join(".mcp.json").is_file());
        assert_eq!(
            run.env,
            Some(
                Agent::Claude
                    .environment(&dir.join("home"), &stub.url(), "/bin")
                    .unwrap()
            )
        );
        let codex = setup.run("c", Kind::Idle, Some(&stub)).unwrap();
        assert!(
            codex.args.contains(&format!(
                r#"model_providers.stub.base_url="{}/v1""#,
                stub.url()
            )),
            "{:?}",
            codex.args
        );
        assert!(
            codex.env.unwrap().contains(&(
                "CODEX_HOME".to_owned(),
                work.path()
                    .join("c/home/.codex")
                    .to_string_lossy()
                    .into_owned()
            ))
        );
    }

    #[test]
    fn tells_why_a_session_failed() {
        use std::os::unix::process::ExitStatusExt;
        let timeout = Duration::from_secs(300);
        let ok = ExitStatus::from_raw(0);
        let failed = ExitStatus::from_raw(1 << 8);
        let killed = ExitStatus::from_raw(9);
        assert_eq!(session_failure(false, timeout, ok), None);
        assert_eq!(
            session_failure(false, timeout, failed),
            Some("exit status: 1".to_owned())
        );
        assert_eq!(
            session_failure(true, timeout, killed),
            Some("timed out after 300 s".to_owned())
        );
        assert_eq!(
            session_failure(true, timeout, ok),
            Some("timed out after 300 s".to_owned())
        );
    }

    #[test]
    fn makes_random_tokens() {
        let token = token();
        assert_eq!(token.len(), 16, "{token}");
        assert!(token.starts_with("tok-"), "{token}");
        assert!(token[4..].chars().all(|c| c.is_ascii_hexdigit()), "{token}");
        assert_ne!(token, super::token());
    }

    #[test]
    fn adds_a_message_with_the_token() {
        let dir = tempfile::tempdir().unwrap();
        let source = Source::open(dir.path()).unwrap();
        let message = add_message(&source, "tok-1").unwrap();
        assert_eq!(message.sender, "live-check of skald");
        assert_eq!(message.text, "The check token is tok-1.");
        assert_eq!(source.state(&message.msg_id).unwrap(), Some(State::Waiting));
    }

    #[test]
    fn adds_the_busy_message_at_the_first_shell_command() {
        let mut busy = BusyMessage::default();
        let mut calls = 0;
        for event in [
            Event::Started {
                sign_in: None,
                inbox_connected: true,
            },
            Event::InboxCalled,
            Event::Reply("working".to_owned()),
            Event::Used(Usage::default()),
        ] {
            busy.on_event(&event, || {
                calls += 1;
                "early"
            });
        }
        assert_eq!(calls, 0, "added before the shell command");
        busy.on_event(&Event::ShellStarted, || {
            calls += 1;
            "first"
        });
        busy.on_event(&Event::ShellStarted, || {
            calls += 1;
            "second"
        });
        assert_eq!(calls, 1);
        assert_eq!(busy.added(), Some("first"));
        assert_eq!(BusyMessage::<()>::default().added(), None);
    }

    #[test]
    fn checks_the_inbox_path() {
        let handed_over = Some(State::HandedOver { at_ms: 5_900 });
        let mut check = InboxCheck::default();
        assert_eq!(
            check.idle(&replied("tok-1"), "tok-1", handed_over),
            r#"  idle session: pass — reply "tok-1"; handed over 4.9 s after the session started"#
        );
        assert_eq!(
            check.other(&replied("NONE"), "tok-1"),
            r#"  the other of two idle sessions: pass — reply "NONE"; the message is the first session's"#
        );
        assert_eq!(
            check.busy(&replied("tok-2"), "tok-2", Some(3_000), handed_over),
            r#"  busy session: pass — reply "tok-2"; added while the shell command ran, handed over 2.9 s later"#
        );
        assert!(check.pass());
        assert_eq!(
            check.verdict(Agent::Codex, &Options::default()),
            "inbox on codex with the model stub: pass"
        );

        // One failed check fails the path.
        assert_eq!(
            check.busy(&replied("NONE"), "tok-2", None, None),
            r#"  busy session: fail — reply "NONE"; the agent ran no shell command"#
        );
        assert!(!check.pass());
        let real = Options {
            real: true,
            ..Options::default()
        };
        assert_eq!(
            check.verdict(Agent::Claude, &real),
            "inbox on claude with the real model: fail"
        );

        let mut check = InboxCheck::default();
        assert!(
            check
                .idle(&replied("NONE"), "tok-1", None)
                .contains(": fail — ")
        );
        assert!(!check.pass());
        let mut check = InboxCheck::default();
        assert!(
            check
                .other(&replied("tok-1"), "tok-1")
                .contains(": fail — ")
        );
        assert!(!check.pass());
    }

    #[test]
    fn exits_with_the_verdict() {
        assert_eq!(exit_code(true), ExitCode::SUCCESS);
        assert_eq!(exit_code(false), ExitCode::FAILURE);
    }

    #[test]
    fn reports_a_check() {
        assert_eq!(
            report("busy session", true, "x"),
            "  busy session: pass — x"
        );
        assert_eq!(
            report("busy session", false, "x"),
            "  busy session: fail — x"
        );
    }
}
