//! What the live checks need from each CLI: how to attach a path to a session in a temporary project,
//! and how to read the session's events; and the checks' options and verdicts.

use std::fs;
use std::io;
use std::path::Path;

use agent_source::State;
use serde_json::{Value, json};

/// The name of the MCP server in each session's config; Claude Code names its tool
/// `mcp__skald__inbox`.
pub const SERVER: &str = "skald";

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
                    "permissions": { "allow": [format!("mcp__{SERVER}__inbox"), "Bash(sleep:*)"] },
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
                let config = [
                    format!("mcp_servers.{SERVER}.command={}", json!(inbox)),
                    format!("mcp_servers.{SERVER}.args={}", json!([source])),
                    // Without it, `codex exec` refuses the tool call: it cannot ask a person.
                    format!("mcp_servers.{SERVER}.default_tools_approval_mode=\"approve\""),
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
                        if item["server"] == SERVER && item["tool"] == "inbox" =>
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

fn inbox_tool() -> String {
    format!("mcp__{SERVER}__inbox")
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

/// A check's line in the output.
pub fn report(check: &str, pass: bool, details: &str) -> String {
    format!("  {check}: {} — {details}", verdict(pass))
}

pub fn verdict(pass: bool) -> &'static str {
    if pass { "pass" } else { "fail" }
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
