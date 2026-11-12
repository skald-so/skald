//! The live checks of the paths that wake an idle Claude Code session: its cross-session inbox
//! socket ([`agent_socket`]), a background `asyncRewake` hook that waits for a message, and a mod
//! that starts a turn with `$.prompt.submit`. Each session runs Claude Code's terminal UI in a
//! pseudo-terminal, as the Channels check does; the socket's bypass case runs Claude Code as the
//! desktop app and VS Code run it, `-p --input-format stream-json`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::channel::{Files, PtySession, Sent, turn_of};
use crate::hook::Hook;
use crate::{Agent, Checks, Kind, Options, pretty};

/// A path that wakes an idle session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wake {
    /// The session's inbox socket, which the `SessionStart` hook registers.
    Socket,
    /// An `asyncRewake` hook on `SessionStart` and `Stop` that waits for a message.
    Rewake,
    /// The mod in `prototypes/agents/mod`, loaded with `--plugin-dir`.
    Mod,
}

impl Wake {
    pub fn parse(name: &str) -> Option<Self> {
        [Self::Socket, Self::Rewake, Self::Mod]
            .into_iter()
            .find(|wake| wake.name() == name)
    }

    /// The path's name as `live-check` takes it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Socket => "socket",
            Self::Rewake => "rewake",
            Self::Mod => "mod",
        }
    }

    /// The prompt of a `kind` session: none for an idle one, which only the path wakes; a busy one
    /// runs `sleep 20`, during which the message comes.
    pub fn prompt(self, kind: Kind) -> Option<String> {
        (kind == Kind::Busy).then(|| Hook::PostToolUse.prompt(Kind::Busy))
    }
}

/// The binaries a wake session's config runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WakeBinaries {
    /// `agent-hook`: the `SessionStart` registration, the `asyncRewake` wait and the mod's helper.
    pub hook: PathBuf,
    /// The mod's plugin directory.
    pub plugin: PathBuf,
}

/// How long the `asyncRewake` hook waits for a message before it ends without waking the session.
pub const REWAKE_WAIT_S: u64 = 600;

/// A settings hook that runs `agent-hook` with some arguments: in the exec form on the CLI, in the
/// shell form on the desktop app and in VS Code.
pub type Handler<'a> = &'a dyn Fn(&[&str]) -> Value;

/// The exec form of a settings hook that runs `hook` with its arguments.
pub fn exec_form(hook: &Path) -> impl Fn(&[&str]) -> Value + '_ {
    move |args| json!({ "type": "command", "command": hook, "args": args })
}

/// The settings hooks of a session of `wake` on `source`, each running `handler`: `SessionStart`
/// registers the session; for `asyncRewake`, `SessionStart` and `Stop` also start the background
/// wait. `extra` are more hooks of an event, such as those that record the agent's turns.
pub fn hooks(wake: Wake, handler: Handler, source: &Path, extra: &[(&str, Value)]) -> Value {
    let source = source.to_string_lossy();
    let register = handler(&[&source]);
    let mut wait = handler(&["--wait", &source, &REWAKE_WAIT_S.to_string()]);
    wait["asyncRewake"] = true.into();
    // Claude Code ends an asyncRewake hook at its timeout.
    wait["timeout"] = (REWAKE_WAIT_S + 60).into();
    let rewake = wake == Wake::Rewake;
    let mut events: Vec<(&str, Vec<Value>)> = vec![
        (
            "SessionStart",
            [register]
                .into_iter()
                .chain(rewake.then(|| wait.clone()))
                .collect(),
        ),
        ("Stop", rewake.then_some(wait).into_iter().collect()),
    ];
    for (event, hook) in extra {
        match events.iter_mut().find(|(name, _)| name == event) {
            Some((_, list)) => list.push(hook.clone()),
            None => events.push((event, vec![hook.clone()])),
        }
    }
    let mut hooks = serde_json::Map::new();
    for (event, list) in events.into_iter().filter(|(_, list)| !list.is_empty()) {
        hooks.insert(event.to_owned(), json!([{ "hooks": list }]));
    }
    Value::Object(hooks)
}

/// Writes the mod's config into `project`, `.skald/mod.json`: the helper and the source.
pub fn write_mod_config(project: &Path, hook: &Path, source: &Path) -> io::Result<()> {
    fs::create_dir_all(project.join(".skald"))?;
    fs::write(
        project.join(".skald/mod.json"),
        pretty(&json!({ "handler": hook, "source": source })),
    )
}

/// Writes the settings of an interactive `kind` session of `wake` into `files.dir` and returns
/// its arguments. Only these files and the arguments change Claude Code's config: no project or
/// user settings, no MCP server, no skills, the lowest effort, and no tool but the busy session's
/// `sleep`, without asking; `Stop` and `PreToolUse` hooks record the agent's turns. A mod session
/// also loads the mod's plugin directory, with its config in the project.
pub fn session(
    files: &Files,
    wake: Wake,
    kind: Kind,
    bins: &WakeBinaries,
    model: Option<&str>,
) -> io::Result<Vec<String>> {
    fs::create_dir_all(&files.project)?;
    if wake == Wake::Mod {
        write_mod_config(&files.project, &bins.hook, &files.source)?;
    }
    let record = json!({ "type": "command", "command": files.record() });
    let settings = files.dir.join("settings.json");
    // Prompt suggestions would cost a model request after each turn.
    fs::write(
        &settings,
        pretty(&json!({
            "promptSuggestionEnabled": false,
            "hooks": hooks(
                wake,
                &exec_form(&bins.hook),
                &files.source,
                &[("Stop", record.clone()), ("PreToolUse", record)],
            ),
        })),
    )?;
    let mut args: Vec<String> = wake.prompt(kind).into_iter().collect();
    let tools = if kind == Kind::Busy { "Bash" } else { "" };
    args.extend(
        [
            "--setting-sources",
            "local",
            "--strict-mcp-config",
            "--settings",
            &settings.to_string_lossy(),
            "--disable-slash-commands",
            "--tools",
            tools,
            "--permission-mode",
            "dontAsk",
            "--effort",
            "low",
        ]
        .map(String::from),
    );
    if kind == Kind::Busy {
        args.extend(["--allowedTools".to_owned(), "Bash(sleep:*)".to_owned()]);
    }
    if wake == Wake::Mod {
        args.extend([
            "--plugin-dir".to_owned(),
            bins.plugin.to_string_lossy().into_owned(),
        ]);
    }
    if let Some(model) = model {
        args.extend(["--model".to_owned(), model.to_owned()]);
    }
    Ok(args)
}

/// The arguments of the socket's bypass session, as the desktop app and VS Code run Claude Code:
/// its input stays open for messages; it skips permission prompts, so a peer message that does not
/// prove it comes from the session's own child is held. Writes its settings, the `SessionStart`
/// hook that registers it, into `files.dir`.
pub fn bypass_session(
    files: &Files,
    bins: &WakeBinaries,
    model: Option<&str>,
) -> io::Result<Vec<String>> {
    fs::create_dir_all(&files.project)?;
    let settings = files.dir.join("settings.json");
    fs::write(
        &settings,
        pretty(
            &json!({ "hooks": hooks(Wake::Socket, &exec_form(&bins.hook), &files.source, &[]) }),
        ),
    )?;
    let mut args: Vec<String> = [
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--setting-sources",
        "local",
        "--strict-mcp-config",
        "--settings",
        &settings.to_string_lossy(),
        "--disable-slash-commands",
        "--tools",
        "",
        "--effort",
        "low",
        "--no-session-persistence",
        "--permission-mode",
        "bypassPermissions",
        "--allow-dangerously-skip-permissions",
    ]
    .map(String::from)
    .to_vec();
    if let Some(model) = model {
        args.extend(["--model".to_owned(), model.to_owned()]);
    }
    Ok(args)
}

/// Whether a line of a stream-json session's output tells that the session held a peer message
/// for a person's approval: `{"type":"system","subtype":"peer_message_hold","state":"held"}`.
pub fn held_notice(line: &str) -> bool {
    serde_json::from_str::<Value>(line).is_ok_and(|event| {
        event["type"] == "system"
            && event["subtype"] == "peer_message_hold"
            && event["state"] == "held"
    })
}

/// A wake path's checks: each returns its line in the output.
#[derive(Debug)]
pub struct WakeCheck {
    wake: Wake,
    checks: Checks,
}

impl WakeCheck {
    pub fn new(wake: Wake) -> Self {
        Self {
            wake,
            checks: Checks::default(),
        }
    }

    /// The `check` of a session and its message, if one came: handed over and reached the model.
    /// The details tell when, after the adding, and for a `busy` session in which turn; `extra`
    /// ends them.
    pub fn received(
        &mut self,
        check: &str,
        session: &PtySession,
        sent: Option<&Sent>,
        busy: bool,
        extra: &str,
    ) -> String {
        let Some(sent) = sent else {
            return self
                .checks
                .none(check, session, &format!("no message came{extra}"));
        };
        let turn = turn_of(session, &sent.token)
            .filter(|_| busy)
            .map_or_else(String::new, |turn| format!(", {turn}"));
        let details = format!("{}{turn}{extra}", sent.arrival(session, "handed over"));
        self.checks.report(check, sent.arrived(session), &details)
    }

    /// The other of two sessions: it registered and the first session's `token` did not reach it.
    pub fn other(&mut self, session: &PtySession, registered: bool, token: &str) -> String {
        let pass = session.clean() && registered && !session.got(token);
        let details = format!(
            "{}; registered: {registered}; the first session's token {} it",
            session.summary(),
            if session.got(token) {
                "reached"
            } else {
                "did not reach"
            }
        );
        self.checks
            .report("the other of two idle sessions", pass, &details)
    }

    /// The bypass session: its message without the token was `held` (the receipt, and the notice
    /// to the host) and did not reach the model; the same message with the token did.
    pub fn bypass(&mut self, outcome: &Bypass) -> String {
        let pass = outcome.receipt.as_deref() == Some("held")
            && outcome.notice
            && !outcome.reached_held
            && outcome.reached_with_token;
        let details = format!(
            "without the token: receipt {:?}, the host's notice {}, the token {} the model; with \
             the token: it {} the model{}",
            outcome.receipt,
            if outcome.notice {
                "came"
            } else {
                "did not come"
            },
            if outcome.reached_held {
                "reached"
            } else {
                "did not reach"
            },
            if outcome.reached_with_token {
                "reached"
            } else {
                "did not reach"
            },
            outcome.model_use
        );
        self.checks.report("bypass session", pass, &details)
    }

    pub fn pass(&self) -> bool {
        self.checks.pass()
    }

    /// The last line of the output.
    pub fn verdict(&self, options: &Options) -> String {
        self.checks
            .verdict(self.wake.name(), Agent::Claude, options)
    }
}

/// What the socket's bypass session did with its message.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Bypass {
    /// The status of the first receipt the reply address got.
    pub receipt: Option<String>,
    /// Whether the session's output told the host it held a message.
    pub notice: bool,
    /// Whether the token reached the model before the message went again with the token.
    pub reached_held: bool,
    /// Whether it reached the model after.
    pub reached_with_token: bool,
    /// The end of the details: the stub's requests or the tokens used.
    pub model_use: String,
}

#[cfg(test)]
mod tests {
    use crate::channel::HookEvent;
    use crate::tests::read_json;
    use crate::{StubLog, Usage};

    use super::*;

    fn bins() -> WakeBinaries {
        WakeBinaries {
            hook: "/bin/agent-hook".into(),
            plugin: "/p/mod".into(),
        }
    }

    #[test]
    fn parses_the_paths_and_prompts_only_the_busy_case() {
        for wake in [Wake::Socket, Wake::Rewake, Wake::Mod] {
            assert_eq!(Wake::parse(wake.name()), Some(wake));
            assert_eq!(wake.prompt(Kind::Idle), None);
            assert_eq!(
                wake.prompt(Kind::Busy).unwrap(),
                Hook::PostToolUse.prompt(Kind::Busy)
            );
        }
        assert_eq!(Wake::parse("channel"), None);
    }

    #[test]
    fn registers_the_session_and_waits_in_the_background_for_rewake() {
        let source = Path::new("/w/source");
        let record = json!({ "type": "command", "command": "cat >> h" });
        let register =
            json!({ "type": "command", "command": "/bin/agent-hook", "args": ["/w/source"] });
        let handler = exec_form(Path::new("/bin/agent-hook"));
        assert_eq!(
            hooks(Wake::Socket, &handler, source, &[("Stop", record.clone())]),
            json!({
                "SessionStart": [{ "hooks": [register] }],
                "Stop": [{ "hooks": [record] }],
            })
        );
        let wait = json!({
            "type": "command", "command": "/bin/agent-hook",
            "args": ["--wait", "/w/source", "600"], "asyncRewake": true, "timeout": 660,
        });
        assert_eq!(
            hooks(
                Wake::Rewake,
                &handler,
                source,
                &[("PreToolUse", record.clone()), ("Stop", record.clone())]
            ),
            json!({
                "SessionStart": [{ "hooks": [register, wait] }],
                "Stop": [{ "hooks": [wait, record] }],
                "PreToolUse": [{ "hooks": [record] }],
            })
        );
        assert_eq!(
            hooks(Wake::Mod, &handler, source, &[]),
            json!({ "SessionStart": [{ "hooks": [register] }] })
        );
    }

    #[test]
    fn a_session_loads_only_its_own_config() {
        let tmp = tempfile::tempdir().unwrap();
        let files = Files::new(tmp.path());
        let settings = tmp.path().join("settings.json");
        let common = |tools: &str| -> Vec<String> {
            [
                "--setting-sources",
                "local",
                "--strict-mcp-config",
                "--settings",
                settings.to_str().unwrap(),
                "--disable-slash-commands",
                "--tools",
                tools,
                "--permission-mode",
                "dontAsk",
                "--effort",
                "low",
            ]
            .map(String::from)
            .to_vec()
        };
        let idle = session(&files, Wake::Socket, Kind::Idle, &bins(), None).unwrap();
        assert_eq!(idle, common(""));
        assert_eq!(
            fs::read_dir(&files.project).unwrap().count(),
            0,
            "the project is empty"
        );
        let written = read_json(&settings);
        assert_eq!(written["promptSuggestionEnabled"], false);
        assert_eq!(
            written["hooks"]["PreToolUse"][0]["hooks"][0]["command"],
            files.record()
        );

        let busy = session(&files, Wake::Mod, Kind::Busy, &bins(), Some("haiku")).unwrap();
        let prompt = Hook::PostToolUse.prompt(Kind::Busy);
        let expected: Vec<String> = [vec![prompt], common("Bash")].concat();
        assert_eq!(busy[..expected.len()], expected);
        assert_eq!(
            busy[expected.len()..],
            [
                "--allowedTools",
                "Bash(sleep:*)",
                "--plugin-dir",
                "/p/mod",
                "--model",
                "haiku"
            ]
        );
        assert_eq!(
            read_json(files.project.join(".skald/mod.json")),
            json!({ "handler": "/bin/agent-hook", "source": files.source })
        );
    }

    #[test]
    fn the_bypass_session_runs_as_the_desktop_app_and_vs_code_run_claude_code() {
        let tmp = tempfile::tempdir().unwrap();
        let files = Files::new(tmp.path());
        let args = bypass_session(&files, &bins(), Some("m")).unwrap();
        assert_eq!(args[..3], ["-p", "--input-format", "stream-json"]);
        for flag in [
            "bypassPermissions",
            "--allow-dangerously-skip-permissions",
            "--no-session-persistence",
        ] {
            assert!(args.iter().any(|arg| arg == flag), "{flag}: {args:?}");
        }
        assert_eq!(args[args.len() - 2..], ["--model", "m"]);
        let settings = read_json(tmp.path().join("settings.json"));
        assert_eq!(
            settings["hooks"],
            hooks(
                Wake::Socket,
                &exec_form(Path::new("/bin/agent-hook")),
                &files.source,
                &[]
            )
        );
    }

    #[test]
    fn reads_the_hosts_notice_of_a_held_message() {
        // Recorded from Claude Code 2.1.322, shortened.
        let held = r#"{"type":"system","subtype":"peer_message_hold","state":"held","lane":"socket","cause":"no-mode-asserted"}"#;
        assert!(held_notice(held));
        assert!(!held_notice(
            &held.replace("\"held\",\"lane", "\"released\",\"lane")
        ));
        assert!(!held_notice(&held.replace("peer_message_hold", "init")));
        assert!(!held_notice("not JSON"));
    }

    fn session_with(replies: &[&str], stub: Option<&[&str]>) -> PtySession {
        PtySession {
            started_ms: 1_000,
            hooks: replies
                .iter()
                .map(|reply| HookEvent::Stopped((*reply).to_owned()))
                .collect(),
            stub: stub.map(|requests| StubLog {
                requests: requests.iter().map(|r| (*r).to_owned()).collect(),
                credentials: Vec::new(),
            }),
            reached_ms: vec![("tok-1".to_owned(), 3_000)],
            usage: Some(Usage::default()),
            channel_listening: None,
        }
    }

    #[test]
    fn judges_each_case() {
        let sent = Sent {
            token: "tok-1".to_owned(),
            added_at_ms: 1_500,
            state: Some(agent_source::State::HandedOver { at_ms: 1_600 }),
        };
        let mut check = WakeCheck::new(Wake::Socket);
        let line = check.received(
            "idle session",
            &session_with(&["tok-1"], None),
            Some(&sent),
            false,
            "",
        );
        assert_eq!(
            line,
            "  idle session: pass — replies [\"tok-1\"], tokens: input 0 (cached 0), output 0; \
             turns ended: 1; handed over 0.1 s and reached the model 1.5 s after adding"
        );
        assert!(check.pass());
        let later = session_with(&["NONE", "tok-1"], None);
        assert!(
            check
                .received("busy session", &later, Some(&sent), true, "")
                .ends_with("after adding, in a later turn")
        );
        assert!(check.pass());
        let missed = session_with(&["NONE"], None);
        assert!(
            check
                .received("busy session", &missed, Some(&sent), true, "")
                .contains(": fail — ")
        );
        assert!(!check.pass());

        let mut check = WakeCheck::new(Wake::Rewake);
        let none = check.received(
            "idle session",
            &session_with(&[], Some(&[])),
            None,
            false,
            ", x",
        );
        assert!(none.ends_with("no message came, x"), "{none}");
        assert_eq!(
            check.verdict(&Options::default()),
            "rewake on claude with the model stub: fail"
        );

        let mut check = WakeCheck::new(Wake::Mod);
        assert!(
            check
                .other(&session_with(&[], Some(&["{}"])), true, "tok-1")
                .contains(": pass — ")
        );
        assert!(
            check
                .other(&session_with(&[], Some(&["tok-1"])), true, "tok-1")
                .contains(": fail — ")
        );
        let mut check = WakeCheck::new(Wake::Mod);
        assert!(
            check
                .other(&session_with(&[], Some(&["{}"])), false, "tok-1")
                .contains(": fail — ")
        );

        let mut check = WakeCheck::new(Wake::Socket);
        let held = Bypass {
            receipt: Some("held".to_owned()),
            notice: true,
            reached_held: false,
            reached_with_token: true,
            model_use: String::new(),
        };
        assert!(check.bypass(&held).contains(": pass — "));
        for broken in [
            Bypass {
                receipt: None,
                ..held.clone()
            },
            Bypass {
                notice: false,
                ..held.clone()
            },
            Bypass {
                reached_held: true,
                ..held.clone()
            },
            Bypass {
                reached_with_token: false,
                ..held.clone()
            },
        ] {
            let mut check = WakeCheck::new(Wake::Socket);
            assert!(check.bypass(&broken).contains(": fail — "), "{broken:?}");
        }
    }
}
