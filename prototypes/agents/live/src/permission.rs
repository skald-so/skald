//! The live checks of the paths that tell that a Claude Code session waits on its own permission
//! prompt: the `PermissionRequest` and `Notification` hooks, on the CLI and the [GUI
//! surfaces](crate::gui), and the Channels permission relay on the CLI. How a session is set up,
//! and what its recorded hook runs and channel notifications tell; the binary runs the CLI.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use agent_channel::{INITIALIZED, PERMISSION_REQUEST};
use serde_json::{Value, json};

use crate::channel::{ENTER, Files};
use crate::gui::{Surface, UserClaude};
use crate::hook::{HOOK_RUNS, Timed, timed_hook, timed_runs};
use crate::{
    Agent, Call, Checks, Options, PROMPTED, SERVER, claude_flags, model_flag, pretty, seconds,
};

/// Where the session's channel writes each notification Claude Code sends it.
pub const NOTICES: &str = "notices.jsonl";

/// The GUI check's file: the surface and its version.
pub const CHECK: &str = "check.json";

/// The hook events a session records: those that may tell of a prompt or its answer, and those
/// that tell the session and its turn.
pub const EVENTS: [&str; 8] = [
    "SessionStart",
    "PreToolUse",
    "PermissionRequest",
    "Notification",
    "PostToolUse",
    "PostToolUseFailure",
    "PermissionDenied",
    "Stop",
];

/// The hook events that may tell that the prompt was answered.
const ANSWER_EVENTS: [&str; 3] = ["PostToolUse", "PostToolUseFailure", "PermissionDenied"];

/// The model stub's script: the shell command no settings allow.
pub const SCRIPT: &[Call] = &[Call::Prompted];

/// What Claude Code's terminal UI shows in a permission dialog.
pub const DIALOG: &str = "Do you want to proceed?";

/// The key that dismisses a dialog, which the permission dialog's "No" option names.
pub const ESC: &[u8] = b"\x1b";

/// The longest a candidate may take to tell for its check to pass.
pub const WITHIN_MS: u64 = 10_000;

/// The prompt of each session: the real model runs the command too.
pub fn prompt() -> String {
    format!("Run the shell command `{PROMPTED}`. Then reply with only OK.")
}

/// How the check answers the terminal's permission dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Yes,
    No,
}

impl Answer {
    /// The keys: Enter on the dialog's first option, "Yes"; Esc, its "No".
    pub fn keys(self) -> &'static [u8] {
        match self {
            Self::Yes => ENTER,
            Self::No => ESC,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Yes => "yes",
            Self::No => "no",
        }
    }
}

/// The shell command of a hook that records each run with `live-check timed-hook` into `runs`: its
/// handler, `true`, prints nothing, so every decision stays with Claude Code and the person.
pub fn recorder(live: &Path, runs: &Path) -> String {
    timed_hook(live, runs, &[Path::new("true")])
}

/// The `hooks` setting that runs `recorder` on each of [`EVENTS`] and, if given, `turns` on
/// `PreToolUse` and `Stop`, which tell the check the agent's turns.
pub fn hooks(recorder: &str, turns: Option<&str>) -> Value {
    let handler = |command: &str| json!({ "type": "command", "command": command });
    let hooks = EVENTS.map(|event| {
        let mut handlers = vec![handler(recorder)];
        if let Some(turns) = turns.filter(|_| matches!(event, "PreToolUse" | "Stop")) {
            handlers.push(handler(turns));
        }
        (event.to_owned(), json!([{ "hooks": handlers }]))
    });
    Value::Object(hooks.into_iter().collect())
}

/// Writes the session's MCP and settings files into `files.dir` and returns the arguments of an
/// interactive session, which waits for [`prompt`] until its channel is ready: the channel `channel`, which records the notifications
/// Claude Code sends it in [`NOTICES`]; the hook runs recorded by `live`, `live-check`, in
/// [`HOOK_RUNS`]; `Bash` its only tool; nothing allowed, in the `default` permission mode, so the
/// command waits on its dialog.
pub fn session(
    files: &Files,
    live: &Path,
    channel: &Path,
    model: Option<&str>,
) -> io::Result<Vec<String>> {
    fs::create_dir_all(&files.project)?;
    let servers = files.dir.join("mcp.json");
    let notices = files.dir.join(NOTICES);
    let args = json!([files.source, "--permission-log", notices]);
    fs::write(
        &servers,
        pretty(&json!({ "mcpServers": { SERVER: { "command": channel, "args": args } } })),
    )?;
    let settings = files.dir.join("settings.json");
    // Prompt suggestions would cost a model request after each turn.
    fs::write(
        &settings,
        pretty(&json!({
            "promptSuggestionEnabled": false,
            "hooks": hooks(&recorder(live, &files.dir.join(HOOK_RUNS)), Some(&files.record())),
        })),
    )?;
    let config = [
        "--mcp-config",
        &servers.to_string_lossy(),
        "--settings",
        &settings.to_string_lossy(),
    ];
    let mut args = claude_flags(&config, false, "Bash", false);
    // Claude Code 2.1.328 starts in auto mode, whose classifier approves the call without asking.
    args.extend(["--permission-mode".to_owned(), "default".to_owned()]);
    args.extend(model_flag("--model", model));
    args.extend([
        "--dangerously-load-development-channels".to_owned(),
        format!("server:{SERVER}"),
    ]);
    Ok(args)
}

/// A notification Claude Code sent the channel.
#[derive(Debug, Clone, PartialEq)]
pub struct Notice {
    pub method: String,
    /// When it came, in milliseconds since the Unix epoch.
    pub at_ms: u64,
    pub params: Value,
}

/// The notifications in the text of a [`NOTICES`] file; a line that is not one is left out.
pub fn notices(text: &str) -> Vec<Notice> {
    text.lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter_map(|notice| {
            Some(Notice {
                method: notice["method"].as_str()?.to_owned(),
                at_ms: notice["at_ms"].as_u64()?,
                params: notice["params"].clone(),
            })
        })
        .collect()
}

/// What a session that waited on its permission prompt recorded and, on the CLI, when its dialog
/// showed and when the check answered it.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Observed {
    pub runs: Vec<Timed>,
    pub notices: Vec<Notice>,
    pub shown_ms: Option<u64>,
    pub answered_ms: Option<u64>,
}

impl Observed {
    /// Reads the hook runs and notifications recorded in `dir`; none if a file is missing.
    pub fn read(dir: &Path) -> Self {
        let text = |name: &str| fs::read_to_string(dir.join(name)).unwrap_or_default();
        Self {
            runs: timed_runs(&text(HOOK_RUNS)),
            notices: notices(&text(NOTICES)),
            ..Self::default()
        }
    }

    /// When the agent called the prompted command: its `PreToolUse` run.
    pub fn called_ms(&self) -> Option<u64> {
        self.runs
            .iter()
            .find(|run| run.event == "PreToolUse" && prompted(&run.input))
            .map(|run| run.started_ms)
    }

    /// When the channel's MCP handshake ended: from then on Claude Code may relay a prompt to it.
    pub fn channel_ready_ms(&self) -> Option<u64> {
        self.notices
            .iter()
            .find(|notice| notice.method == INITIALIZED)
            .map(|notice| notice.at_ms)
    }

    /// The session's ID, as its first hook run gives it.
    pub fn session_id(&self) -> Option<&str> {
        self.runs
            .iter()
            .find_map(|run| run.input["session_id"].as_str())
    }

    /// What came once the prompt was answered, after the check answered it or, without that time,
    /// after the agent called the command: the prompted call's [`ANSWER_EVENTS`] runs and each
    /// notification but a relayed prompt and the handshake's end; with how long after that time
    /// each came.
    pub fn after_answer(&self) -> Vec<(String, u64)> {
        let Some(from) = self.answered_ms.or(self.called_ms()) else {
            return Vec::new();
        };
        let runs = self
            .runs
            .iter()
            .filter(|run| ANSWER_EVENTS.contains(&run.event.as_str()) && prompted(&run.input))
            .map(|run| (format!("{} hook", run.event), run.started_ms));
        let notices = self
            .notices
            .iter()
            .filter(|notice| ![PERMISSION_REQUEST, INITIALIZED].contains(&notice.method.as_str()))
            .map(|notice| (notice.method.clone(), notice.at_ms));
        let mut after: Vec<(String, u64)> = runs
            .chain(notices)
            .filter(|(_, at_ms)| *at_ms >= from)
            .map(|(name, at_ms)| (name, at_ms - from))
            .collect();
        after.sort_by_key(|(_, delay)| *delay);
        after
    }
}

/// Whether a hook's `input` is about the check's prompted shell command.
fn prompted(input: &Value) -> bool {
    input["tool_name"] == "Bash"
        && input["tool_input"]["command"]
            .as_str()
            .is_some_and(|command| command.contains(PROMPTED))
}

/// A path that may tell that a session waits on its permission prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Candidate {
    /// The `PermissionRequest` hook, for the prompted call.
    PermissionRequest,
    /// The `Notification` hook of type `permission_prompt`.
    Notification,
    /// The Channels permission relay: a `permission_request` notification.
    Relay,
}

impl Candidate {
    /// The candidates on the CLI, where Channels load.
    pub const CLI: [Self; 3] = [Self::PermissionRequest, Self::Notification, Self::Relay];

    /// The candidates on the desktop app and VS Code, which load no channel.
    pub const GUI: [Self; 2] = [Self::PermissionRequest, Self::Notification];

    pub fn name(self) -> &'static str {
        match self {
            Self::PermissionRequest => "PermissionRequest hook",
            Self::Notification => "Notification hook, permission_prompt",
            Self::Relay => "Channels permission relay",
        }
    }

    /// When the candidate first told of the prompt, and what it carried: the names of its fields
    /// and the values that say what waits.
    pub fn told(self, observed: &Observed) -> Option<(u64, String)> {
        let (at_ms, fields, says) = match self {
            Self::PermissionRequest => {
                let run = observed
                    .runs
                    .iter()
                    .find(|run| run.event == "PermissionRequest" && prompted(&run.input))?;
                let says = format!(
                    "{}: {}",
                    text(&run.input["tool_name"]),
                    text(&run.input["tool_input"]["command"])
                );
                (run.started_ms, &run.input, says)
            }
            Self::Notification => {
                let run = observed.runs.iter().find(|run| {
                    run.event == "Notification"
                        && run.input["notification_type"] == "permission_prompt"
                })?;
                (run.started_ms, &run.input, text(&run.input["message"]))
            }
            Self::Relay => {
                let notice = observed
                    .notices
                    .iter()
                    .find(|notice| notice.method == PERMISSION_REQUEST)?;
                let params = &notice.params;
                let says = format!(
                    "{}: {}; {}",
                    text(&params["tool_name"]),
                    text(&params["description"]),
                    text(&params["input_preview"])
                );
                (notice.at_ms, params, says)
            }
        };
        Some((at_ms, carries(fields, &says)))
    }
}

/// The names of `fields` and what `says`, such as `threadId, command (command: touch …)`.
pub(crate) fn carries(fields: &Value, says: &str) -> String {
    let names: Vec<&str> = fields
        .as_object()
        .map(|fields| fields.keys().map(String::as_str).collect())
        .unwrap_or_default();
    format!("{} ({says})", names.join(", "))
}

/// Whether a session waited on its prompt, and the details of its line: `summary`, whether the
/// agent called the prompted `command`, at `called`, and when the dialog showed, `shown`, which the
/// check waited for if `dialog`.
pub(crate) fn prompted_details(
    summary: &str,
    command: &str,
    called: Option<u64>,
    shown: Option<u64>,
    dialog: bool,
) -> (bool, String) {
    let showed = match (called, shown) {
        (Some(called), Some(shown)) => format!("; the dialog showed {}", signed(shown, called)),
        _ if dialog => "; the dialog never showed".to_owned(),
        _ => String::new(),
    };
    let waited = called.is_some() && (!dialog || shown.is_some());
    let details = format!(
        "{summary}{}the agent {} `{command}`{showed}",
        if summary.is_empty() { "" } else { "; " },
        if called.is_some() {
            "called"
        } else {
            "never called"
        }
    );
    (waited, details)
}

/// The details of a candidate's line: when it `told` and what it carried, after the call at
/// `called` and the dialog's showing at `shown`.
pub(crate) fn told_details(
    told: Option<&(u64, String)>,
    called: Option<u64>,
    shown: Option<u64>,
) -> String {
    match (told, called) {
        (Some((at_ms, carries)), Some(called)) => {
            let dialog = shown.map_or_else(String::new, |shown| {
                format!(", {} after the dialog showed", signed(*at_ms, shown))
            });
            format!(
                "told {} after the call{dialog}; carries {carries}",
                signed(*at_ms, called)
            )
        }
        (Some((_, carries)), None) => format!("told without the call; carries {carries}"),
        (None, _) => "did not tell".to_owned(),
    }
}

/// What came once a prompt was answered, each with its delay, or `nothing`.
pub(crate) fn came(after: &[(String, u64)]) -> String {
    if after.is_empty() {
        return "nothing".to_owned();
    }
    after
        .iter()
        .map(|(what, delay)| format!("{what} {}", seconds(*delay)))
        .collect::<Vec<_>>()
        .join(", ")
}

/// A JSON value as text: a string as it is, anything else as JSON.
pub(crate) fn text(value: &Value) -> String {
    value
        .as_str()
        .map_or_else(|| value.to_string(), String::from)
}

/// `to_ms` less `from_ms`, in seconds with a sign, such as `-0.1 s`.
pub(crate) fn signed(to_ms: u64, from_ms: u64) -> String {
    // Less than what rounds to 0.1 s is no sign.
    if to_ms + 50 > from_ms {
        seconds(to_ms.saturating_sub(from_ms))
    } else {
        format!("-{}", seconds(from_ms - to_ms))
    }
}

/// The permission prompt checks: each returns its lines in the output.
#[derive(Debug, Default)]
pub struct PermissionCheck {
    checks: Checks,
}

impl PermissionCheck {
    /// The session `name`, which `summary` sums up, and, on the CLI, whose dialog the check
    /// answered `answer`: whether it waited on its prompt, then a line per candidate and what came
    /// once the prompt was answered.
    pub fn session(
        &mut self,
        name: &str,
        observed: &Observed,
        candidates: &[Candidate],
        answer: Option<Answer>,
        summary: &str,
    ) -> Vec<String> {
        let called = observed.called_ms();
        let (waited, details) = prompted_details(
            summary,
            PROMPTED,
            called,
            observed.shown_ms,
            answer.is_some(),
        );
        let mut lines = vec![
            self.checks
                .report(&format!("{name}, prompt"), waited, &details),
        ];
        for candidate in candidates {
            let told = candidate.told(observed);
            let pass = matches!((told.as_ref(), called), (Some((at, _)), Some(called)) if at.saturating_sub(called) <= WITHIN_MS);
            lines.push(self.checks.report(
                &format!("{name}, {}", candidate.name()),
                pass,
                &told_details(told.as_ref(), called, observed.shown_ms),
            ));
        }
        let from = match answer {
            Some(answer) => format!("after answering {} in the terminal", answer.name()),
            None => "after the call".to_owned(),
        };
        let came = came(&observed.after_answer());
        lines.push(format!("  {name}, {from}: {came}"));
        lines
    }

    pub fn pass(&self) -> bool {
        self.checks.pass()
    }

    /// The last line of the output, on the CLI with the model of `options`.
    pub fn verdict(&self, options: &Options) -> String {
        self.checks
            .verdict("permission prompts", Agent::Claude, options)
    }
}

/// Writes the GUI check of `surface`, whose version line is `version`, into `work` and returns the
/// project to open: its local settings run the hook recorder of `live`, `live-check`, on each of
/// [`EVENTS`] into `work`'s [`HOOK_RUNS`], cap the effort, turn off the `user`'s plugins, connectors and
/// MCP servers' tools, and allow nothing. Only files in the project configure the surface.
pub fn prepare_gui(
    work: &Path,
    surface: Surface,
    version: &str,
    live: &Path,
    user: &UserClaude,
) -> io::Result<PathBuf> {
    if surface.agent() != Agent::Claude {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} runs no Claude Code", surface.name()),
        ));
    }
    let project = work.join("project");
    fs::create_dir_all(project.join(".claude"))?;
    let mut settings = json!({
        "hooks": hooks(&recorder(live, &work.join(HOOK_RUNS)), None),
        "maxEffortLevel": "low",
    });
    user.turn_off(&mut settings);
    fs::write(
        project.join(".claude/settings.local.json"),
        pretty(&settings),
    )?;
    fs::write(
        work.join(CHECK),
        pretty(&json!({ "surface": surface.name(), "version": version })),
    )?;
    Ok(project)
}

/// The surface and version line of the GUI check prepared in `work`.
pub fn gui_check(work: &Path) -> io::Result<(Surface, String)> {
    let check: Value = serde_json::from_str(&fs::read_to_string(work.join(CHECK))?)?;
    let surface = check["surface"].as_str().and_then(Surface::parse);
    match (surface, check["version"].as_str()) {
        (Some(surface), Some(version)) => Ok((surface, version.to_owned())),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{} has no surface or version", work.join(CHECK).display()),
        )),
    }
}

/// The desktop app's link to the session of the GUI check in `work`, which waits on its prompt: the
/// app's own ID of the session its hooks recorded, which the app's record in `home` maps to Claude
/// Code's.
pub fn needs_input(work: &Path, home: &Path) -> io::Result<String> {
    let invalid = |message: &str| io::Error::new(io::ErrorKind::InvalidData, message.to_owned());
    let observed = Observed::read(work);
    let session = observed
        .session_id()
        .ok_or_else(|| invalid("no hook ran yet: send the prompt first"))?;
    let app = crate::gui::desktop_session(home, session)?
        .ok_or_else(|| invalid("the Claude app has no record of the session"))?;
    Ok(needs_input_link(&app))
}

/// The desktop app's link for a session waiting on input, with the app's own ID of the session.
pub fn needs_input_link(app_session: &str) -> String {
    format!(
        "claude://code/needs-input?session={}",
        crate::gui::encode(app_session)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The hook runs and channel notifications recorded on Claude Code 2.1.328, paths shortened: of
    /// the session of `live-check permission claude --real`, answered yes; of the session answered
    /// no of `live-check permission claude --all`, on the model stub.
    const YES: &str = include_str!("../tests/permission/yes.jsonl");
    const YES_NOTICES: &str = include_str!("../tests/permission/yes-notices.jsonl");
    const NO: &str = include_str!("../tests/permission/no.jsonl");
    const NO_NOTICES: &str = include_str!("../tests/permission/no-notices.jsonl");

    fn observed(runs: &str, notices_text: &str) -> Observed {
        Observed {
            runs: timed_runs(runs),
            notices: notices(notices_text),
            ..Observed::default()
        }
    }

    #[test]
    fn a_session_records_each_event_and_runs_the_prompted_command_unallowed() {
        let dir = tempfile::tempdir().unwrap();
        let files = Files::new(dir.path());
        let (live, channel) = (
            Path::new("/bin/live-check"),
            Path::new("/bin/agent-channel"),
        );
        let args = session(&files, live, channel, Some("m")).unwrap();
        assert!(prompt().contains(PROMPTED));
        assert!(!args.contains(&prompt()), "typed once the channel is ready");
        let flag = |name: &str| {
            let at = args.iter().position(|arg| arg == name).unwrap();
            args[at + 1].clone()
        };
        assert_eq!(flag("--tools"), "Bash");
        assert_eq!(flag("--model"), "m");
        assert_eq!(
            flag("--dangerously-load-development-channels"),
            "server:skald"
        );
        assert_eq!(flag("--permission-mode"), "default");
        assert!(
            !args
                .iter()
                .any(|arg| arg == "--allowedTools" || arg.contains("dangerously-skip")),
            "nothing skips the prompt: {args:?}"
        );

        let read = |name: &str| -> Value {
            serde_json::from_str(&fs::read_to_string(dir.path().join(name)).unwrap()).unwrap()
        };
        let server = &read("mcp.json")["mcpServers"]["skald"];
        assert_eq!(server["command"], "/bin/agent-channel");
        assert_eq!(
            server["args"],
            json!([
                dir.path().join("source"),
                "--permission-log",
                dir.path().join(NOTICES)
            ])
        );
        let settings = read("settings.json");
        assert!(settings.get("permissions").is_none(), "nothing allowed");
        let hooks = settings["hooks"].as_object().unwrap();
        assert_eq!(hooks.len(), EVENTS.len());
        let recorder = format!(
            "'/bin/live-check' 'timed-hook' '{}' 'true'",
            dir.path().join(HOOK_RUNS).display()
        );
        for event in EVENTS {
            let handlers = &hooks[event][0]["hooks"];
            assert_eq!(handlers[0]["command"], recorder.as_str(), "{event}");
            let turns = matches!(event, "PreToolUse" | "Stop");
            assert_eq!(handlers.as_array().unwrap().len(), 1 + usize::from(turns));
            if turns {
                assert_eq!(handlers[1]["command"], files.record());
            }
        }
    }

    #[test]
    fn the_answers_press_yes_or_dismiss() {
        assert_eq!(Answer::Yes.keys(), b"\r");
        assert_eq!(Answer::No.keys(), b"\x1b");
        assert_eq!(SCRIPT, [Call::Prompted]);
    }

    #[test]
    fn reads_the_recorded_runs_and_notices() {
        let yes = observed(YES, YES_NOTICES);
        let events: Vec<&str> = yes.runs.iter().map(|run| run.event.as_str()).collect();
        assert_eq!(
            events,
            [
                "SessionStart",
                "PreToolUse",
                "PermissionRequest",
                "Notification",
                "PostToolUse",
                "Stop"
            ]
        );
        assert!(yes.called_ms().is_some());
        assert_eq!(
            yes.session_id(),
            Some("94288116-c540-4fdf-9190-31915b97792f")
        );
        let methods: Vec<&str> = yes.notices.iter().map(|n| n.method.as_str()).collect();
        assert_eq!(methods, [INITIALIZED, PERMISSION_REQUEST]);
        assert_eq!(yes.channel_ready_ms(), Some(1_795_122_136_128));
        assert!(yes.channel_ready_ms() < yes.called_ms());
        assert_eq!(observed(YES, "").channel_ready_ms(), None);
        assert!(timed_runs("not json\n{}\n").is_empty());
        assert!(notices("{\"method\":\"m\"}\n").is_empty());
    }

    #[test]
    fn each_candidate_tells_of_the_prompt_with_what_waits() {
        let yes = observed(YES, YES_NOTICES);
        let called = yes.called_ms().unwrap();
        let (at, carries) = Candidate::PermissionRequest.told(&yes).unwrap();
        assert!(at >= called && at - called < 1_000, "{}", at - called);
        assert!(carries.contains("tool_input") && carries.contains("Bash: touch prompted.txt"));
        let (at, carries) = Candidate::Notification.told(&yes).unwrap();
        assert!(
            (5_000..WITHIN_MS).contains(&(at - called)),
            "{}",
            at - called
        );
        assert!(carries.contains("notification_type"), "{carries}");
        let (at, carries) = Candidate::Relay.told(&yes).unwrap();
        assert!(at >= called && at - called < 1_000, "{}", at - called);
        assert!(
            carries.starts_with("description, input_preview, request_id, tool_name (Bash: "),
            "{carries}"
        );
        assert!(carries.contains(PROMPTED), "{carries}");
    }

    #[test]
    fn a_delay_has_a_sign_only_from_what_rounds_to_a_tenth() {
        assert_eq!(signed(1_000, 1_000), "0.0 s");
        assert_eq!(signed(1_600, 1_000), "0.6 s");
        assert_eq!(signed(951, 1_000), "0.0 s");
        assert_eq!(signed(950, 1_000), format!("-{}", seconds(50)));
        assert_eq!(signed(100, 1_000), "-0.9 s");
    }

    #[test]
    fn a_candidate_that_did_not_run_tells_nothing() {
        let mut yes = observed(YES, "");
        assert_eq!(Candidate::Relay.told(&yes), None);
        yes.runs.retain(|run| run.event != "Notification");
        assert_eq!(Candidate::Notification.told(&yes), None);
        // Another hook's run of the prompted call is not the PermissionRequest hook's.
        let mut other = observed(YES, "");
        other.runs.retain(|run| run.event != "PermissionRequest");
        assert_eq!(Candidate::PermissionRequest.told(&other), None);
        // A notification of another type is not the prompt's.
        let mut idle = observed(YES, "");
        for run in &mut idle.runs {
            if run.event == "Notification" {
                run.input["notification_type"] = "idle_prompt".into();
            }
        }
        assert_eq!(Candidate::Notification.told(&idle), None);
        // A run about another command is not the prompt's.
        for run in &mut yes.runs {
            run.input["tool_input"]["command"] = "ls".into();
        }
        assert_eq!(Candidate::PermissionRequest.told(&yes), None);
        assert_eq!(yes.called_ms(), None);
    }

    #[test]
    fn after_an_answer_only_the_answer_events_count() {
        let mut yes = observed(YES, YES_NOTICES);
        let names = |observed: &Observed| -> Vec<String> {
            observed
                .after_answer()
                .into_iter()
                .map(|(name, _)| name)
                .collect()
        };
        assert_eq!(names(&yes), ["PostToolUse hook"]);
        let post = yes
            .runs
            .iter()
            .find(|run| run.event == "PostToolUse")
            .unwrap()
            .started_ms;
        yes.answered_ms = Some(post - 100);
        assert_eq!(yes.after_answer(), [("PostToolUse hook".to_owned(), 100)]);
        yes.answered_ms = Some(post + 1);
        assert!(yes.after_answer().is_empty());

        let no = observed(NO, NO_NOTICES);
        assert!(
            names(&no).iter().all(|name| name != "PostToolUse hook"),
            "{:?}",
            names(&no)
        );
        assert!(Candidate::PermissionRequest.told(&no).is_some());

        let mut other = observed(YES, YES_NOTICES);
        other.notices.push(Notice {
            method: "notifications/claude/channel/other".to_owned(),
            at_ms: post,
            params: Value::Null,
        });
        assert!(names(&other).contains(&"notifications/claude/channel/other".to_owned()));
    }

    #[test]
    fn the_check_passes_when_each_candidate_tells_within_seconds() {
        let mut yes = observed(YES, YES_NOTICES);
        let called = yes.called_ms().unwrap();
        yes.shown_ms = Some(called + 200);
        let mut check = PermissionCheck::default();
        let lines = check.session("a", &yes, &Candidate::CLI, Some(Answer::Yes), "replies []");
        assert!(check.pass(), "{lines:#?}");
        assert_eq!(
            check.verdict(&Options::default()),
            "permission prompts on claude with the model stub: pass"
        );
        assert_eq!(lines.len(), 5);
        assert!(lines[0].starts_with("  a, prompt: pass — replies []; the agent called"));
        assert!(
            lines[0].ends_with("the dialog showed 0.2 s"),
            "{}",
            lines[0]
        );
        assert!(
            lines[1].contains("PermissionRequest hook: pass — told "),
            "{}",
            lines[1]
        );
        assert!(lines[3].contains("after the dialog showed"), "{}", lines[3]);
        assert!(
            lines[4].starts_with("  a, after answering yes in the terminal: PostToolUse hook ")
        );

        // Without the relay's notification, the relay fails, and so the check.
        let mut check = PermissionCheck::default();
        let lines = check.session("a", &observed(YES, ""), &Candidate::CLI, None, "s");
        assert!(lines[3].contains("Channels permission relay: fail — did not tell"));
        assert!(!check.pass());

        // A GUI session has no relay and no terminal.
        let mut check = PermissionCheck::default();
        let lines = check.session("a", &observed(YES, ""), &Candidate::GUI, None, "s");
        assert!(check.pass(), "{lines:#?}");
        assert!(
            lines[0].ends_with("the agent called `touch prompted.txt`"),
            "{}",
            lines[0]
        );
        assert!(lines[3].starts_with("  a, after the call: PostToolUse hook "));
    }

    #[test]
    fn a_candidate_that_tells_late_or_a_dialog_that_never_showed_fails() {
        let mut late = observed(YES, YES_NOTICES);
        for run in &mut late.runs {
            if run.event == "Notification" {
                run.started_ms += WITHIN_MS;
            }
        }
        let mut check = PermissionCheck::default();
        let lines = check.session("a", &late, &Candidate::GUI, None, "s");
        assert!(
            lines[2].contains("Notification hook, permission_prompt: fail"),
            "{}",
            lines[2]
        );
        assert!(!check.pass());

        let mut check = PermissionCheck::default();
        let lines = check.session("a", &observed(YES, ""), &[], Some(Answer::No), "s");
        assert!(lines[0].contains("prompt: fail") && lines[0].ends_with("the dialog never showed"));
        assert!(!check.pass());
    }

    #[test]
    fn a_gui_check_records_each_event_and_allows_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let user = UserClaude {
            plugins: vec!["p@m".to_owned()],
            mcp_servers: vec!["mine".to_owned()],
        };
        let live = Path::new("/bin/live-check");
        let project =
            prepare_gui(dir.path(), Surface::ClaudeDesktop, "Claude 1", live, &user).unwrap();
        assert_eq!(project, dir.path().join("project"));
        let settings: Value = serde_json::from_str(
            &fs::read_to_string(project.join(".claude/settings.local.json")).unwrap(),
        )
        .unwrap();
        let recorder = recorder(live, &dir.path().join(HOOK_RUNS));
        for event in EVENTS {
            assert_eq!(
                settings["hooks"][event][0]["hooks"][0]["command"],
                recorder.as_str()
            );
            assert_eq!(
                settings["hooks"][event][0]["hooks"]
                    .as_array()
                    .unwrap()
                    .len(),
                1
            );
        }
        assert_eq!(settings["maxEffortLevel"], "low");
        assert_eq!(settings["enabledPlugins"], json!({ "p@m": false }));
        assert_eq!(settings["permissions"], json!({ "deny": ["mcp__mine"] }));
        assert_eq!(
            gui_check(dir.path()).unwrap(),
            (Surface::ClaudeDesktop, "Claude 1".to_owned())
        );
        assert!(prepare_gui(dir.path(), Surface::CodexDesktop, "v", live, &user).is_err());
    }

    #[test]
    fn the_needs_input_link_carries_the_apps_session() {
        assert_eq!(
            needs_input_link("local_1 2"),
            "claude://code/needs-input?session=local_1%202"
        );
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(Observed::read(dir.path()), Observed::default());
        fs::write(dir.path().join(HOOK_RUNS), YES).unwrap();
        fs::write(dir.path().join(NOTICES), YES_NOTICES).unwrap();
        assert_eq!(Observed::read(dir.path()), observed(YES, YES_NOTICES));
    }
}
