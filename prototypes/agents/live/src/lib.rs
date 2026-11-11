//! What the live checks need from each CLI: how to attach a path to a session in a temporary project,
//! and how to read the session's events.

use std::fs;
use std::io;
use std::path::Path;

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
                Some("gpt-6-luna"),
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
                "gpt-6-luna",
                "Call inbox.",
            ]
        );
        assert_eq!(
            fs::read_dir(&project).unwrap().count(),
            0,
            "no files in the project"
        );
    }
}
