//! What the live checks need from each CLI: how to attach a path to a session in a temporary project,
//! and how to read the session's events; and the checks' options, prompts, messages and verdicts.
//! The binary only starts the CLIs and reads their output.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{ExitCode, ExitStatus};
use std::time::Duration;

use agent_inbox::INBOX;
use agent_source::{Message, Source, State};
use serde_json::{Value, json};

/// The name of the MCP server in each session's config.
pub const SERVER: &str = "skald";

/// The sender label of the checks' messages.
pub const SENDER: &str = "live-check of skald";

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

    /// The CLI's version line from the output of `<cli> --version`.
    pub fn version(self, stdout: &[u8]) -> String {
        format!("{}: {}", self.cli(), String::from_utf8_lossy(stdout).trim())
    }

    /// Attaches the `inbox` MCP server, `inbox <source>`, to sessions in `project` and returns the
    /// arguments of a non-interactive session there that gets `prompt`. Only files in `project` and
    /// the arguments change the CLI's config.
    pub fn session(
        self,
        project: &Path,
        inbox: &Path,
        source: &Path,
        prompt: &str,
        model: Option<&str>,
    ) -> io::Result<Vec<String>> {
        let inbox = inbox.to_string_lossy();
        let source = source.to_string_lossy();
        let mut args = Vec::new();
        match self {
            Self::Claude => {
                let servers =
                    json!({ "mcpServers": { SERVER: { "command": inbox, "args": [source] } } });
                fs::write(project.join(".mcp.json"), pretty(&servers))?;
                // Without these, Claude Code asks a person to approve the project's MCP server and
                // each call of its tool.
                let settings = json!({
                    "enableAllProjectMcpServers": true,
                    "permissions": { "allow": [inbox_tool(), "Bash(sleep:*)"] },
                });
                fs::create_dir_all(project.join(".claude"))?;
                fs::write(
                    project.join(".claude/settings.local.json"),
                    pretty(&settings),
                )?;
                args.extend(
                    ["-p", prompt, "--output-format", "stream-json", "--verbose"].map(String::from),
                );
                if let Some(model) = model {
                    args.extend(["--model".to_owned(), model.to_owned()]);
                }
            }
            Self::Codex => {
                // `-c` values are TOML; a JSON string or array of strings is valid TOML.
                let server = format!("mcp_servers.{SERVER}");
                let config = [
                    format!("{server}.command={}", json!(inbox)),
                    format!("{server}.args={}", json!([source])),
                    // Without it, `codex exec` refuses the tool call: it cannot ask a person.
                    format!("{server}.default_tools_approval_mode=\"approve\""),
                ];
                args.extend(
                    [
                        "exec",
                        "--json",
                        "--skip-git-repo-check",
                        "--ephemeral",
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
                args.push(prompt.to_owned());
            }
        }
        Ok(args)
    }

    /// What one line of the session's JSON output says, if it matters to a check.
    pub fn event(self, line: &str) -> Option<Event> {
        let value: Value = serde_json::from_str(line).ok()?;
        match self {
            Self::Claude => match value["type"].as_str()? {
                "system" if value["subtype"] == "init" => Some(Event::Started {
                    sign_in: value["apiKeySource"].as_str().map(String::from),
                    inbox_connected: value["mcp_servers"]
                        .as_array()?
                        .iter()
                        .any(|server| server["name"] == SERVER && server["status"] == "connected"),
                }),
                "assistant" => {
                    let content = value["message"]["content"].as_array()?;
                    content.iter().find_map(|block| {
                        match (block["type"].as_str(), block["name"].as_str()) {
                            (Some("tool_use"), Some("Bash")) => Some(Event::ShellStarted),
                            (Some("tool_use"), Some(name)) if name == inbox_tool() => {
                                Some(Event::InboxCalled)
                            }
                            _ => None,
                        }
                    })
                }
                "result" => Some(Event::Reply(value["result"].as_str()?.to_owned())),
                _ => None,
            },
            Self::Codex => {
                let item = &value["item"];
                match (value["type"].as_str()?, item["type"].as_str()?) {
                    ("item.started", "command_execution") => Some(Event::ShellStarted),
                    ("item.started", "mcp_tool_call")
                        if item["server"] == SERVER && item["tool"] == INBOX =>
                    {
                        Some(Event::InboxCalled)
                    }
                    ("item.completed", "agent_message") => {
                        Some(Event::Reply(item["text"].as_str()?.to_owned()))
                    }
                    _ => None,
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
}

/// The options of `live-check inbox`.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Options<'a> {
    /// `--model <model>`: the model of every session.
    pub model: Option<&'a str>,
    /// `--keep`: keep the sessions' logs after a pass.
    pub keep: bool,
}

impl<'a> Options<'a> {
    /// Parses `[--model <model>] [--keep]`, in any order; `None` for anything else.
    pub fn parse(mut options: &[&'a str]) -> Option<Self> {
        let mut parsed = Self::default();
        loop {
            match options {
                [] => return Some(parsed),
                ["--model", model, rest @ ..] => (parsed.model, options) = (Some(*model), rest),
                ["--keep", rest @ ..] => (parsed.keep, options) = (true, rest),
                _ => return None,
            }
        }
    }

    /// Whether the sessions' logs stay after a check that ended with `pass`: after a fail, always.
    pub fn keeps_logs(&self, pass: bool) -> bool {
        self.keep || !pass
    }
}

/// The `agent-inbox` binary next to `live-check`, the binary `exe`.
pub fn inbox_server(exe: &Path) -> Result<PathBuf, String> {
    let inbox = exe.with_file_name("agent-inbox");
    if inbox.exists() {
        Ok(inbox)
    } else {
        Err(format!(
            "{} is missing: build the workspace",
            inbox.display()
        ))
    }
}

/// A session to run: its project, its inbox's source, its logs and the CLI's arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub name: String,
    pub agent: Agent,
    pub project: PathBuf,
    pub source: PathBuf,
    pub args: Vec<String>,
    /// The session's JSON output.
    pub log: PathBuf,
    /// The CLI's stderr.
    pub stderr: PathBuf,
}

impl Run {
    /// Prepares the session `name` in `work/<name>`: a project with the `inbox` server attached.
    pub fn new(
        work: &Path,
        name: &str,
        agent: Agent,
        inbox: &Path,
        prompt: &str,
        model: Option<&str>,
    ) -> io::Result<Self> {
        let dir = work.join(name);
        let project = dir.join("project");
        let source = dir.join("source");
        fs::create_dir_all(&project)?;
        let args = agent.session(&project, inbox, &source, prompt, model)?;
        Ok(Self {
            name: name.to_owned(),
            agent,
            project,
            source,
            args,
            log: dir.join("session.jsonl"),
            stderr: dir.join("session.stderr"),
        })
    }
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

/// The idle sessions' prompt.
pub fn idle_prompt() -> String {
    format!("Call {}", ask())
}

/// The busy session's prompt: its message comes while the shell command runs.
pub fn busy_prompt() -> String {
    format!("Run the shell command `sleep 20`. Then call {}", ask())
}

fn ask() -> String {
    format!(
        "the {INBOX} tool of the {SERVER} MCP server and reply with only the check token from \
         the message, or NONE if there is none."
    )
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
            Event::ShellStarted => {}
        }
    }

    /// The idle and busy checks: the session called `inbox` and repeated `token`, and the message
    /// with it, in `state`, was handed over. A message never added has no state.
    pub fn received(&self, token: &str, state: Option<State>) -> bool {
        self.inbox_called
            && self.reply.contains(token)
            && matches!(state, Some(State::HandedOver { .. }))
    }

    /// The other idle session's check: it called `inbox` and did not get `token`.
    pub fn missed(&self, token: &str) -> bool {
        self.inbox_called && !self.reply.contains(token)
    }

    pub fn summary(&self) -> String {
        let mut summary = format!("reply {:?}", self.reply.trim());
        if let Some(sign_in) = &self.sign_in {
            summary.push_str(&format!(", apiKeySource {sign_in}"));
        }
        if self.inbox_connected == Some(false) {
            summary.push_str(", the inbox server did not connect");
        }
        if !self.inbox_called {
            summary.push_str(", inbox not called");
        }
        summary
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
    /// The idle session, one of two, and its message with `token`, now in `state`.
    pub fn idle(&mut self, session: &Session, token: &str, state: Option<State>) -> String {
        let details = format!(
            "{}; handed over {} after the session started",
            session.summary(),
            handover_delay(state, session.started_ms)
        );
        self.report(
            "idle session, one of two",
            session.received(token, state),
            &details,
        )
    }

    /// The other idle session, which must not get the message with `token`.
    pub fn other(&mut self, session: &Session, token: &str) -> String {
        let details = format!("{}; the message did not reach it", session.summary());
        self.report("the other idle session", session.missed(token), &details)
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
        let details = format!("{}; {message}", session.summary());
        self.report("busy session", session.received(token, state), &details)
    }

    /// Whether every check passed.
    pub fn pass(&self) -> bool {
        self.passed.iter().all(|pass| *pass)
    }

    /// The last line of the output.
    pub fn verdict(&self, agent: Agent) -> String {
        format!("{INBOX} on {}: {}", agent.cli(), verdict(self.pass()))
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
    const CLAUDE_INIT: &str = r#"{"type":"system","subtype":"init","mcp_servers":[{"name":"exa","status":"connected","source":"user"},{"name":"skald","status":"connected","source":"project"}],"apiKeySource":"none","claude_code_version":"2.1.322"}"#;
    const CLAUDE_INIT_FAILED: &str = r#"{"type":"system","subtype":"init","mcp_servers":[{"name":"skald","status":"failed","source":"project"}],"apiKeySource":"ANTHROPIC_API_KEY"}"#;
    const CLAUDE_INIT_WITHOUT_INBOX: &str = r#"{"type":"system","subtype":"init","mcp_servers":[{"name":"exa","status":"connected","source":"user"},{"name":"skald","status":"pending","source":"project"}],"apiKeySource":"none"}"#;
    const CLAUDE_HOOK: &str = r#"{"type":"system","subtype":"hook_response","mcp_servers":[{"name":"skald","status":"connected"}]}"#;
    const CLAUDE_SEARCH: &str = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_01","name":"ToolSearch","input":{"query":"select:mcp__skald__inbox","max_results":1}}]}}"#;
    const CLAUDE_INBOX: &str = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_02","name":"mcp__skald__inbox","input":{}}]}}"#;
    const CLAUDE_BASH: &str = r#"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":""},{"type":"tool_use","id":"toolu_03","name":"Bash","input":{"command":"sleep 20"}}]}}"#;
    const CLAUDE_TEXT: &str =
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"tok-7f3a91"}]}}"#;
    const CLAUDE_RESULT: &str = r#"{"type":"result","subtype":"success","is_error":false,"result":"tok-7f3a91","stop_reason":"end_turn"}"#;

    // Lines recorded from Codex 0.162.1 (`codex exec --json`), shortened.
    const CODEX_INBOX: &str = r#"{"type":"item.started","item":{"id":"item_2","type":"mcp_tool_call","server":"skald","tool":"inbox","arguments":{},"result":null,"error":null,"status":"in_progress"}}"#;
    const CODEX_INBOX_DONE: &str = r#"{"type":"item.completed","item":{"id":"item_2","type":"mcp_tool_call","server":"skald","tool":"inbox","arguments":{},"result":{"content":[{"type":"text","text":"No messages are waiting."}]},"error":null,"status":"completed"}}"#;
    const CODEX_OTHER_TOOL: &str = r#"{"type":"item.started","item":{"id":"item_4","type":"mcp_tool_call","server":"exa","tool":"inbox","arguments":{},"status":"in_progress"}}"#;
    const CODEX_SHELL: &str = r#"{"type":"item.started","item":{"id":"item_1","type":"command_execution","command":"/bin/zsh -lc 'sleep 20'","aggregated_output":"","exit_code":null,"status":"in_progress"}}"#;
    const CODEX_REPLY: &str = r#"{"type":"item.completed","item":{"id":"item_3","type":"agent_message","text":"tok-c0d3x2"}}"#;
    const CODEX_ERROR: &str = r#"{"type":"item.completed","item":{"id":"item_0","type":"error","message":"Codex is ignoring 1 unrecognized configuration setting."}}"#;
    const CODEX_DONE: &str = r#"{"type":"turn.completed","usage":{"input_tokens":44124}}"#;

    #[test]
    fn reads_claude_code_events() {
        let claude = Agent::Claude;
        assert_eq!(
            claude.event(CLAUDE_INIT),
            Some(Event::Started {
                sign_in: Some("none".to_owned()),
                inbox_connected: true
            })
        );
        assert_eq!(
            claude.event(CLAUDE_INIT_FAILED),
            Some(Event::Started {
                sign_in: Some("ANTHROPIC_API_KEY".to_owned()),
                inbox_connected: false
            })
        );
        assert_eq!(
            claude.event(CLAUDE_INIT_WITHOUT_INBOX),
            Some(Event::Started {
                sign_in: Some("none".to_owned()),
                inbox_connected: false
            })
        );
        assert_eq!(claude.event(CLAUDE_HOOK), None);
        assert_eq!(claude.event(CLAUDE_SEARCH), None);
        assert_eq!(claude.event(CLAUDE_INBOX), Some(Event::InboxCalled));
        assert_eq!(claude.event(CLAUDE_BASH), Some(Event::ShellStarted));
        assert_eq!(claude.event(CLAUDE_TEXT), None);
        assert_eq!(
            claude.event(CLAUDE_RESULT),
            Some(Event::Reply("tok-7f3a91".to_owned()))
        );
        assert_eq!(claude.event("not JSON"), None);
        // Codex's events mean nothing to Claude Code.
        assert_eq!(claude.event(CODEX_REPLY), None);
    }

    #[test]
    fn reads_codex_events() {
        let codex = Agent::Codex;
        assert_eq!(codex.event(CODEX_INBOX), Some(Event::InboxCalled));
        assert_eq!(codex.event(CODEX_INBOX_DONE), None);
        assert_eq!(codex.event(CODEX_OTHER_TOOL), None);
        assert_eq!(codex.event(CODEX_SHELL), Some(Event::ShellStarted));
        assert_eq!(
            codex.event(CODEX_REPLY),
            Some(Event::Reply("tok-c0d3x2".to_owned()))
        );
        assert_eq!(codex.event(CODEX_ERROR), None);
        assert_eq!(codex.event(CODEX_DONE), None);
        assert_eq!(codex.event(CLAUDE_RESULT), None);
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
    fn claude_code_session_uses_project_config() {
        let project = tempfile::tempdir().unwrap();
        let args = Agent::Claude
            .session(
                project.path(),
                Path::new("/bin/inbox"),
                Path::new("/tmp/source"),
                "Call inbox.",
                Some("haiku"),
            )
            .unwrap();
        assert_eq!(
            args,
            [
                "-p",
                "Call inbox.",
                "--output-format",
                "stream-json",
                "--verbose",
                "--model",
                "haiku"
            ]
        );
        let servers: Value =
            serde_json::from_slice(&fs::read(project.path().join(".mcp.json")).unwrap()).unwrap();
        assert_eq!(
            servers,
            json!({ "mcpServers": { "skald": { "command": "/bin/inbox", "args": ["/tmp/source"] } } })
        );
        let settings: Value = serde_json::from_slice(
            &fs::read(project.path().join(".claude/settings.local.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(settings["enableAllProjectMcpServers"], true);
        assert_eq!(
            settings["permissions"]["allow"],
            json!(["mcp__skald__inbox", "Bash(sleep:*)"])
        );

        let args = Agent::Claude
            .session(
                project.path(),
                Path::new("/bin/inbox"),
                Path::new("/s"),
                "p",
                None,
            )
            .unwrap();
        assert!(!args.contains(&"--model".to_owned()), "{args:?}");
    }

    #[test]
    fn codex_session_sets_config_by_arguments() {
        let project = tempfile::tempdir().unwrap();
        let args = Agent::Codex
            .session(
                project.path(),
                Path::new("/bin/my inbox"),
                Path::new("/tmp/\"source\""),
                "Call inbox.",
                Some("gpt-5.5"),
            )
            .unwrap();
        let project = project.path().to_string_lossy().into_owned();
        assert_eq!(
            args,
            [
                "exec",
                "--json",
                "--skip-git-repo-check",
                "--ephemeral",
                "-C",
                &project,
                "-c",
                r#"mcp_servers.skald.command="/bin/my inbox""#,
                "-c",
                r#"mcp_servers.skald.args=["/tmp/\"source\""]"#,
                "-c",
                r#"mcp_servers.skald.default_tools_approval_mode="approve""#,
                "-m",
                "gpt-5.5",
                "Call inbox.",
            ]
        );
        assert_eq!(
            fs::read_dir(&project).unwrap().count(),
            0,
            "no files in the project"
        );
    }

    #[test]
    fn parses_options() {
        assert_eq!(Options::parse(&[]), Some(Options::default()));
        assert_eq!(
            Options::parse(&["--keep"]),
            Some(Options {
                model: None,
                keep: true
            })
        );
        assert_eq!(
            Options::parse(&["--model", "gpt-5.5"]),
            Some(Options {
                model: Some("gpt-5.5"),
                keep: false
            })
        );
        assert_eq!(
            Options::parse(&["--keep", "--model", "haiku"]),
            Some(Options {
                model: Some("haiku"),
                keep: true
            })
        );
        assert_eq!(Options::parse(&["--model"]), None);
        assert_eq!(Options::parse(&["--keep", "--verbose"]), None);
        assert_eq!(Options::parse(&["gpt-5.5"]), None);
    }

    /// A session that called `inbox` and replied `reply`.
    fn replied(reply: &str) -> Session {
        let mut session = Session::new(1_000);
        session.record(&Event::InboxCalled);
        session.record(&Event::Reply(reply.to_owned()));
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
        session.record(&Event::Reply("working".to_owned()));
        session.record(&Event::InboxCalled);
        session.record(&Event::Reply("tok-1".to_owned()));
        assert_eq!(
            session,
            Session {
                reply: "tok-1".to_owned(),
                sign_in: Some("none".to_owned()),
                inbox_connected: Some(false),
                inbox_called: true,
                started_ms: 1_000,
            }
        );
    }

    #[test]
    fn a_session_receives_a_message_it_repeats_after_the_handover() {
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
    fn the_other_session_misses_a_message_after_calling_inbox() {
        assert!(replied("NONE").missed("tok-1"));
        assert!(!replied("tok-1").missed("tok-1"));
        assert!(!Session::new(1_000).missed("tok-1"));
    }

    #[test]
    fn summarizes_a_session() {
        assert_eq!(replied(" tok-1\n").summary(), r#"reply "tok-1""#);
        let mut session = Session::new(1_000);
        session.record(&Event::Started {
            sign_in: Some("none".to_owned()),
            inbox_connected: false,
        });
        assert_eq!(
            session.summary(),
            r#"reply "", apiKeySource none, the inbox server did not connect, inbox not called"#
        );
        session.record(&Event::Started {
            sign_in: None,
            inbox_connected: true,
        });
        assert_eq!(session.summary(), r#"reply "", inbox not called"#);
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
            model: None,
            keep: true,
        };
        assert!(!Options::default().keeps_logs(true));
        assert!(Options::default().keeps_logs(false));
        assert!(keep.keeps_logs(true));
        assert!(keep.keeps_logs(false));
    }

    #[test]
    fn finds_the_inbox_server_next_to_live_check() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("live-check");
        let inbox = dir.path().join("agent-inbox");
        assert_eq!(
            inbox_server(&exe),
            Err(format!(
                "{} is missing: build the workspace",
                inbox.display()
            ))
        );
        fs::write(&inbox, "").unwrap();
        assert_eq!(inbox_server(&exe), Ok(inbox));
    }

    #[test]
    fn prepares_a_session_in_its_own_directory() {
        let work = tempfile::tempdir().unwrap();
        let inbox = Path::new("/bin/inbox");
        let run = Run::new(work.path(), "a", Agent::Codex, inbox, "p", Some("m")).unwrap();
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
                    .session(&project, inbox, &source, "p", Some("m"))
                    .unwrap(),
                project,
                source,
                log: dir.join("session.jsonl"),
                stderr: dir.join("session.stderr"),
            }
        );
        let claude = Run::new(work.path(), "b", Agent::Claude, inbox, "p", None).unwrap();
        assert_eq!(claude.args[..2], ["-p", "p"]);
        assert!(claude.project.join(".mcp.json").is_file());
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
    fn prompts_name_the_inbox_tool() {
        assert_eq!(
            idle_prompt(),
            "Call the inbox tool of the skald MCP server and reply with only the check token \
             from the message, or NONE if there is none."
        );
        assert_eq!(
            busy_prompt(),
            "Run the shell command `sleep 20`. Then call the inbox tool of the skald MCP server \
             and reply with only the check token from the message, or NONE if there is none."
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
            r#"  idle session, one of two: pass — reply "tok-1"; handed over 4.9 s after the session started"#
        );
        assert_eq!(
            check.other(&replied("NONE"), "tok-1"),
            r#"  the other idle session: pass — reply "NONE"; the message did not reach it"#
        );
        assert_eq!(
            check.busy(&replied("tok-2"), "tok-2", Some(3_000), handed_over),
            r#"  busy session: pass — reply "tok-2"; added while the shell command ran, handed over 2.9 s later"#
        );
        assert!(check.pass());
        assert_eq!(check.verdict(Agent::Codex), "inbox on codex: pass");

        // One failed check fails the path.
        assert_eq!(
            check.busy(&replied("NONE"), "tok-2", None, None),
            r#"  busy session: fail — reply "NONE"; the agent ran no shell command"#
        );
        assert!(!check.pass());
        assert_eq!(check.verdict(Agent::Claude), "inbox on claude: fail");

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
