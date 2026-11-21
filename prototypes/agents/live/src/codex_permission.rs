//! The live checks of the paths that tell that a Codex session waits on Codex's own permission
//! prompt, one per candidate: the app-server's `item/commandExecution/requestApproval` on an
//! app-server the terminal UI joins with `--remote` and on the background app-server, the
//! `PermissionRequest` hook, and the terminal UI's `approval-requested` notification. How a session
//! is set up, and what its recorded hook runs, app-server messages and terminal notifications tell;
//! the binary runs Codex.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::channel::Files;
use crate::hook::{Timed, timed_hook, timed_runs};
use crate::permission::{came, carries, prompted_details, text, told_details};
use crate::{Agent, Call, Checks, Options, PROMPTED, codex_config_flags, codex_hook, model_flag};

/// The app-server's request that a command waits on approval.
pub const REQUEST: &str = "item/commandExecution/requestApproval";

/// The app-server's notification that a request it sent was answered or dropped.
pub const RESOLVED: &str = "serverRequest/resolved";

/// The app-server's notification of a thread's new status.
pub const STATUS: &str = "thread/status/changed";

/// The flag of a thread's status while it waits on an approval.
pub const WAITING: &str = "waitingOnApproval";

/// The model stub's script: [`PAUSE`](crate::PAUSE), which runs in the sandbox unasked and gives the check's
/// watcher time to subscribe to the thread once its first turn saved it, then the shell command,
/// asking to run it outside the sandbox, which under [`APPROVAL`] waits on the person's approval.
pub fn script() -> &'static [Call] {
    let input = json!({
        "cmd": PROMPTED,
        "sandbox_permissions": "require_escalated",
        "justification": "May I create prompted.txt?",
        // Codex waits for the command up to `yield_time_ms`, 10 s by default.
        "yield_time_ms": 30_000,
    });
    Call::script(vec![Call::Pause, Call::tool("exec_command", &input)])
}

/// What Codex's terminal UI shows in the dialog of a command that waits on approval.
pub const DIALOG: &str = "Would you like to run the following command";

/// The approval policy and sandbox of every session: commands run in the read-only sandbox, and one
/// the agent asks to run outside it waits on the person's approval. Codex 0.162.1 refuses
/// `untrusted`, which asked before each command not known to be safe.
pub const APPROVAL: [&str; 2] = [
    "approval_policy=\"on-request\"",
    "sandbox_mode=\"read-only\"",
];

/// The hook events a session records: the one that tells of a prompt, and those that tell the
/// session, the call and its answer.
pub const EVENTS: [&str; 5] = [
    "SessionStart",
    "PreToolUse",
    "PermissionRequest",
    "PostToolUse",
    "Stop",
];

/// Where the check's app-server watcher writes each message it gets, with when it came.
pub const WATCH: &str = "watch.jsonl";

/// Where the check writes each notification the terminal UI emits, with when it came.
pub const NOTIFIED: &str = "notified.jsonl";

/// The directory of the session files in a config home.
pub const SESSIONS: &str = "sessions";

/// The longest a candidate may take to tell for its check to pass.
pub const WITHIN_MS: u64 = 10_000;

/// The key that answers the dialog "Yes, proceed".
pub const YES: &[u8] = b"y";

/// The key that answers the dialog "No, and tell Codex what to do differently".
pub const NO: &[u8] = b"\x1b";

/// The prompt of a session that runs the command.
pub fn prompt() -> String {
    format!("Run the shell command `{PROMPTED}`. Then reply with only OK.")
}

/// A path that may tell that a session waits on its permission prompt; each has a check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Candidate {
    /// The app-server's [`REQUEST`], on an app-server the check starts on a Unix socket, which the
    /// terminal UI joins with `--remote`.
    Remote,
    /// The app-server's [`REQUEST`], on the background app-server that a terminal UI without
    /// `--no-daemon` starts in its config home.
    Daemon,
    /// The `PermissionRequest` hook.
    Hook,
    /// The terminal UI's `approval-requested` notification, an OSC 9 sequence.
    Notification,
}

impl Candidate {
    pub const ALL: [Self; 4] = [Self::Remote, Self::Daemon, Self::Hook, Self::Notification];

    /// The candidate of `live-check`'s argument.
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|candidate| candidate.check() == name)
    }

    /// The check's name as `live-check` takes it.
    pub fn check(self) -> &'static str {
        match self {
            Self::Remote => "permission-remote",
            Self::Daemon => "permission-daemon",
            Self::Hook => "permission-hook",
            Self::Notification => "permission-notification",
        }
    }

    /// The candidate's name in the output.
    pub fn name(self) -> &'static str {
        match self {
            Self::Remote => "app-server request, --remote",
            Self::Daemon => "app-server request, background app-server",
            Self::Hook => "PermissionRequest hook",
            Self::Notification => "approval-requested notification",
        }
    }

    /// Whether the check runs with `--real`: the background app-server runs only in the check's
    /// own config home, which has no sign-in.
    pub fn takes_real(self) -> bool {
        self != Self::Daemon
    }

    /// Whether the check watches an app-server.
    pub fn watches(self) -> bool {
        matches!(self, Self::Remote | Self::Daemon)
    }
}

/// The shell command of a hook that records each run with `live-check timed-hook` into `runs`: its
/// handler, `true`, prints nothing, so every decision stays with Codex and the person.
pub fn recorder(live: &Path, runs: &Path) -> String {
    timed_hook(live, runs, &[Path::new("true")])
}

/// The config values that run `recorder` on each of [`EVENTS`].
fn hooks(recorder: &str) -> Vec<String> {
    EVENTS
        .iter()
        .map(|event| codex_hook(event, recorder))
        .collect()
}

/// The config values the session's Codex gets for `candidate`, on the model stub at `stub` if
/// given: the hooks that record into `runs` with `live`, `live-check`; `projects` trusted; no update
/// check or notification program; [`APPROVAL`]; and for [`Candidate::Notification`] the terminal
/// UI's notification of an approval as OSC 9, whether or not the terminal has focus.
fn config(
    candidate: Candidate,
    live: &Path,
    runs: &Path,
    projects: &[&Path],
    stub: Option<&str>,
) -> Vec<String> {
    let mut config = hooks(&recorder(live, runs));
    config.extend([
        trusted(projects),
        "check_for_update_on_startup=false".to_owned(),
        "notify=[]".to_owned(),
    ]);
    config.extend(APPROVAL.map(String::from));
    if candidate == Candidate::Notification {
        config.extend([
            "tui.notifications=[\"approval-requested\"]".to_owned(),
            "tui.notification_method=\"osc9\"".to_owned(),
            "tui.notification_condition=\"always\"".to_owned(),
        ]);
    }
    codex_config_flags(config, stub)
}

/// The arguments of the terminal UI of the `candidate` session in `files.project`, which waits for
/// its prompt, on the model stub at `stub` if given; its hook runs go to `runs`. Only the arguments
/// and, for the background app-server, its config home's `config.toml` ([`daemon_config`]) change
/// Codex's config: no plugins; hooks without the trust Codex otherwise asks a person for; for
/// [`Candidate::Remote`] the app-server on `socket`, which has the config; for
/// [`Candidate::Daemon`] the background app-server; otherwise one of its own.
pub fn session(
    candidate: Candidate,
    files: &Files,
    runs: &Path,
    live: &Path,
    socket: Option<&Path>,
    model: Option<&str>,
    stub: Option<&str>,
) -> Vec<String> {
    let project = files.project.to_string_lossy().into_owned();
    if candidate == Candidate::Daemon {
        // A terminal UI with any config override runs an app-server of its own: the background
        // app-server's config is its config home's ([`daemon_config`]).
        return vec!["-C".to_owned(), project];
    }
    let mut args: Vec<String> = Vec::new();
    match (candidate, socket) {
        (Candidate::Remote, Some(socket)) => {
            args.extend([
                "--remote".to_owned(),
                format!("unix://{}", socket.display()),
            ]);
        }
        _ => args.push("--no-daemon".to_owned()),
    }
    args.extend(
        [
            "--disable",
            "plugins",
            "--dangerously-bypass-hook-trust",
            "-C",
        ]
        .map(String::from),
    );
    args.push(project);
    args.extend(config(candidate, live, runs, &[&files.project], stub));
    args.extend(model_flag("-m", model));
    args
}

/// The arguments of the app-server that listens on `socket` for [`Candidate::Remote`]: the
/// session's config ([`session`]) with `projects` trusted: the app-server's trust is the one its
/// terminal UIs get.
pub fn app_server(
    socket: &Path,
    projects: &[&Path],
    runs: &Path,
    live: &Path,
    model: Option<&str>,
    stub: Option<&str>,
) -> Vec<String> {
    let mut args = vec![
        "app-server".to_owned(),
        "--listen".to_owned(),
        format!("unix://{}", socket.display()),
        "--disable".to_owned(),
        "plugins".to_owned(),
    ];
    args.extend(config(Candidate::Remote, live, runs, projects, stub));
    if let Some(model) = model {
        args.extend(["-c".to_owned(), format!("model={}", json!(model))]);
    }
    args
}

/// The config value that trusts `projects`, in one table: a second `-c projects=…` replaces the
/// first, and TOML takes a key once.
fn trusted(projects: &[&Path]) -> String {
    let trusted: Vec<String> = projects
        .iter()
        .map(|project| {
            format!(
                "{}={{trust_level=\"trusted\"}}",
                json!(project.to_string_lossy())
            )
        })
        .collect();
    format!("projects={{{}}}", trusted.join(","))
}

/// The text of the background app-server's `config.toml`: the config of [`Candidate::Daemon`]
/// ([`config`]), one value a line, with `projects` trusted, no plugins and the model `model` if
/// given.
pub fn daemon_config(
    live: &Path,
    runs: &Path,
    projects: &[&Path],
    model: Option<&str>,
    stub: Option<&str>,
) -> String {
    let mut values: Vec<String> = config(Candidate::Daemon, live, runs, projects, stub)
        .chunks(2)
        .map(|flag| flag[1].clone())
        .collect();
    values.push("features.plugins=false".to_owned());
    values.extend(model.map(|model| format!("model={}", json!(model))));
    values.iter().map(|value| format!("{value}\n")).collect()
}

/// The background app-server's socket in the config home `codex`, once it listens.
pub fn daemon_socket(codex: &Path) -> Option<PathBuf> {
    let mut dirs = vec![codex.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        for entry in fs::read_dir(&dir).ok()?.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if path.is_dir() {
                dirs.push(path);
            } else if name.starts_with("app-server") && name.ends_with(".sock") {
                return Some(path);
            }
        }
    }
    None
}

/// A message the app-server watcher got.
#[derive(Debug, Clone, PartialEq)]
pub struct Watched {
    /// When it came, in milliseconds since the Unix epoch.
    pub at_ms: u64,
    pub message: Value,
}

/// The line of the [`WATCH`] file for `message`, which came at `at_ms`.
pub fn watched_line(at_ms: u64, message: &Value) -> String {
    json!({ "at_ms": at_ms, "message": message }).to_string()
}

/// The messages in the text of a [`WATCH`] file; a line that is not one is left out.
pub fn watched(text: &str) -> Vec<Watched> {
    text.lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter_map(|line| {
            Some(Watched {
                at_ms: line["at_ms"].as_u64()?,
                message: line["message"].clone(),
            })
        })
        .collect()
}

/// A notification the terminal UI emitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notified {
    /// When it came, in milliseconds since the Unix epoch.
    pub at_ms: u64,
    pub text: String,
}

/// The notifications in the text of a [`NOTIFIED`] file; a line that is not one is left out.
pub fn notified(text: &str) -> Vec<Notified> {
    text.lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter_map(|line| {
            Some(Notified {
                at_ms: line["at_ms"].as_u64()?,
                text: line["text"].as_str()?.to_owned(),
            })
        })
        .collect()
}

/// The line of the [`NOTIFIED`] file for `notified`.
pub fn notified_line(notified: &Notified) -> String {
    json!({ "at_ms": notified.at_ms, "text": notified.text }).to_string()
}

/// The entries in the text of a session file; a line that is not one is left out.
pub fn session_file(text: &str) -> Vec<Value> {
    text.lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

/// Finds the OSC 9 notifications, `ESC ] 9 ; <text>` ended by BEL or `ESC \`, in a terminal's
/// output fed to it in chunks.
#[derive(Debug, Default)]
pub struct Osc9 {
    state: OscState,
    text: Vec<u8>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum OscState {
    #[default]
    Text,
    /// After ESC.
    Escape,
    /// After `ESC ]`.
    Osc,
    /// After `ESC ] 9`.
    Nine,
    /// In the notification's text.
    Notification,
    /// ESC in the notification's text: `\` ends it.
    End,
}

impl Osc9 {
    /// Adds the terminal's output `bytes` and returns the text of each notification they end.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<String> {
        let mut found = Vec::new();
        for &byte in bytes {
            self.state = match (self.state, byte) {
                (OscState::Text | OscState::Escape, 0x1b) => OscState::Escape,
                (OscState::Escape, b']') => OscState::Osc,
                (OscState::Osc, b'9') => OscState::Nine,
                (OscState::Nine, b';') => {
                    self.text.clear();
                    OscState::Notification
                }
                (OscState::Notification, 0x07) | (OscState::End, b'\\') => {
                    found.push(String::from_utf8_lossy(&self.text).into_owned());
                    OscState::Text
                }
                (OscState::Notification, 0x1b) => OscState::End,
                (OscState::Notification, byte) => {
                    self.text.push(byte);
                    OscState::Notification
                }
                (OscState::End, byte) => {
                    self.text.extend([0x1b, byte]);
                    OscState::Notification
                }
                _ => OscState::Text,
            };
        }
        found
    }
}

/// What a session that waited on its permission prompt recorded, and when its dialog showed and
/// when the check answered it.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Observed {
    /// The hook runs of every session that records into the same file.
    pub runs: Vec<Timed>,
    /// The app-server messages the watcher got, of every thread it watches.
    pub watched: Vec<Watched>,
    pub notified: Vec<Notified>,
    /// The entries of each session file in the session's config home.
    pub sessions: Vec<Vec<Value>>,
    /// The session's thread: the `session_id` of its hook runs.
    pub thread: Option<String>,
    pub shown_ms: Option<u64>,
    pub answered_ms: Option<u64>,
}

impl Observed {
    /// Reads the hook runs of `runs`, the app-server messages of `watch`, and the notifications and
    /// the session files of the config home `home/.codex` in the session's directory `dir`; none of
    /// a missing file.
    pub fn read(runs: &Path, watch: &Path, dir: &Path) -> Self {
        let text = |path: &Path| fs::read_to_string(path).unwrap_or_default();
        let mut files = vec![];
        let mut dirs = vec![dir.join("home").join(".codex").join(SESSIONS)];
        while let Some(dir) = dirs.pop() {
            for path in fs::read_dir(&dir)
                .into_iter()
                .flatten()
                .flatten()
                .map(|entry| entry.path())
            {
                if path.is_dir() {
                    dirs.push(path);
                } else if path
                    .extension()
                    .is_some_and(|extension| extension == "jsonl")
                {
                    files.push(path);
                }
            }
        }
        Self {
            runs: timed_runs(&text(runs)),
            watched: watched(&text(watch)),
            notified: notified(&text(&dir.join(NOTIFIED))),
            sessions: files.iter().map(|file| session_file(&text(file))).collect(),
            ..Self::default()
        }
    }

    /// What this session's file holds between the agent's call of the prompted command and its
    /// end: the type of each entry, such as `token_usage_record` or `event_msg/exec_approval_request`;
    /// `None` without the file or the call.
    pub fn session_between(&self) -> Option<Vec<String>> {
        let entries = self.sessions.iter().find(|entries| {
            entries
                .iter()
                .any(|entry| entry["type"] == "session_meta" && self.ours(&entry["payload"]["id"]))
        })?;
        let call = entries.iter().position(|entry| {
            entry["payload"]["type"] == "function_call"
                && text(&entry["payload"]["arguments"]).contains(PROMPTED)
        })?;
        let id = &entries[call]["payload"]["call_id"];
        let ends = |entry: &Value| {
            let payload = &entry["payload"];
            (payload["type"] == "function_call_output" && &payload["call_id"] == id)
                || (payload["type"] == "item_completed" && &payload["item"]["id"] == id)
        };
        Some(
            entries[call + 1..]
                .iter()
                .take_while(|entry| !ends(entry))
                .map(|entry| match entry["payload"]["type"].as_str() {
                    Some(kind) => format!("{}/{kind}", text(&entry["type"])),
                    None => text(&entry["type"]),
                })
                .collect(),
        )
    }

    /// The thread of the session in `project`: the `session_id` of the first hook run there, or
    /// the first thread there that the watcher learned of.
    pub fn thread_in(&self, project: &Path) -> Option<String> {
        let canonical = fs::canonicalize(project).ok();
        let here = |cwd: &Value| {
            cwd.as_str().is_some_and(|cwd| {
                Path::new(cwd) == project || Some(Path::new(cwd)) == canonical.as_deref()
            })
        };
        let hooked = self.runs.iter().find_map(|run| {
            here(&run.input["cwd"])
                .then(|| run.input["session_id"].as_str().map(String::from))
                .flatten()
        });
        hooked.or_else(|| {
            self.watched_threads()
                .into_iter()
                // Codex starts an ephemeral thread of its own in the project for a session's title.
                .find(|thread| here(&thread["cwd"]) && thread["ephemeral"] != true)
                .and_then(|thread| thread["id"].as_str().map(String::from))
        })
    }

    /// The threads the watcher learned of: those of the app-server's `thread/started`
    /// notifications and of its `thread/resume` results.
    pub fn watched_threads(&self) -> Vec<Value> {
        self.watched
            .iter()
            .filter_map(|watched| {
                let message = &watched.message;
                match message.get("method") {
                    Some(method) if method == "thread/started" => {
                        message["params"].get("thread").cloned()
                    }
                    Some(_) => None,
                    None => message["result"].get("thread").cloned(),
                }
            })
            .collect()
    }

    /// Whether `value` names this session's thread.
    fn ours(&self, value: &Value) -> bool {
        self.thread.is_some() && value.as_str() == self.thread.as_deref()
    }

    /// When the agent called the prompted command: its `PreToolUse` run, or else the app-server's
    /// `item/started` of the command.
    pub fn called_ms(&self) -> Option<u64> {
        let hooked = self.runs.iter().find(|run| {
            run.event == "PreToolUse" && self.ours(&run.input["session_id"]) && prompted(&run.input)
        });
        let started = || {
            self.watched.iter().find(|watched| {
                let params = &watched.message["params"];
                watched.message["method"] == "item/started"
                    && self.ours(&params["threadId"])
                    && params["item"]["type"] == "commandExecution"
                    && text(&params["item"]["command"]).contains(PROMPTED)
            })
        };
        hooked
            .map(|run| run.started_ms)
            .or_else(|| started().map(|watched| watched.at_ms))
    }

    /// When `candidate` first told of this session's prompt, and what it carried: the names of its
    /// fields and the values that say what waits.
    pub fn told(&self, candidate: Candidate) -> Option<(u64, String)> {
        match candidate {
            Candidate::Remote | Candidate::Daemon => {
                let watched = self.watched.iter().find(|watched| {
                    waits(&watched.message) && self.ours(&watched.message["params"]["threadId"])
                })?;
                let (method, params) = (&watched.message["method"], &watched.message["params"]);
                let says = if method == REQUEST {
                    format!("{REQUEST}, command: {}", text(&params["command"]))
                } else {
                    format!("{STATUS} {}", status(&params["status"]))
                };
                let request = self.watched.iter().any(|watched| {
                    watched.message["method"] == REQUEST
                        && self.ours(&watched.message["params"]["threadId"])
                });
                Some((
                    watched.at_ms,
                    format!(
                        "{}; {REQUEST} {} the watcher",
                        carries(params, &says),
                        if request { "reached" } else { "never reached" }
                    ),
                ))
            }
            Candidate::Hook => {
                let run = self.runs.iter().find(|run| {
                    run.event == "PermissionRequest"
                        && self.ours(&run.input["session_id"])
                        && prompted(&run.input)
                })?;
                let says = format!(
                    "{}: {}",
                    text(&run.input["tool_name"]),
                    text(&run.input["tool_input"]["command"])
                );
                Some((run.started_ms, carries(&run.input, &says)))
            }
            Candidate::Notification => {
                let notified = self.notified.first()?;
                Some((notified.at_ms, format!("the text \"{}\"", notified.text)))
            }
        }
    }

    /// Whether `candidate` told of a prompt of another session: what it named.
    pub fn others(&self, candidate: Candidate) -> Vec<String> {
        let other = |value: &Value| {
            value
                .as_str()
                .is_some_and(|id| !self.ours(value) && !id.is_empty())
        };
        match candidate {
            Candidate::Remote | Candidate::Daemon => self
                .watched
                .iter()
                .filter(|watched| waits(&watched.message))
                .filter(|watched| other(&watched.message["params"]["threadId"]))
                .map(|watched| text(&watched.message["params"]["threadId"]))
                .collect(),
            Candidate::Hook => self
                .runs
                .iter()
                .filter(|run| run.event == "PermissionRequest" && other(&run.input["session_id"]))
                .map(|run| text(&run.input["session_id"]))
                .collect(),
            Candidate::Notification => Vec::new(),
        }
    }

    /// What came once the prompt was answered, after the check answered it or, without that time,
    /// after the agent called the command, with how long after that time each came: the app-server's
    /// [`RESOLVED`], status changes and the command's end for this session's thread, and its
    /// `PostToolUse` and `Stop` hook runs.
    pub fn after_answer(&self) -> Vec<(String, u64)> {
        let Some(from) = self.answered_ms.or(self.called_ms()) else {
            return Vec::new();
        };
        let messages = self.watched.iter().filter_map(|watched| {
            let message = &watched.message;
            let params = &message["params"];
            if !self.ours(&params["threadId"]) {
                return None;
            }
            let what = match message["method"].as_str()? {
                RESOLVED => RESOLVED.to_owned(),
                STATUS => format!("{STATUS} {}", status(&params["status"])),
                "item/completed" if params["item"]["type"] == "commandExecution" => format!(
                    "item/completed commandExecution {}",
                    text(&params["item"]["status"])
                ),
                _ => return None,
            };
            Some((what, watched.at_ms))
        });
        let runs = self
            .runs
            .iter()
            .filter(|run| {
                self.ours(&run.input["session_id"])
                    && (run.event == "Stop" || (run.event == "PostToolUse" && prompted(&run.input)))
            })
            .map(|run| (format!("{} hook", run.event), run.started_ms));
        let mut after: Vec<(String, u64)> = messages
            .chain(runs)
            .filter(|(_, at_ms)| *at_ms >= from)
            .map(|(what, at_ms)| (what, at_ms - from))
            .collect();
        after.sort_by_key(|(_, delay)| *delay);
        after
    }
}

/// Whether an app-server `message` tells that a thread waits on an approval: the [`REQUEST`]
/// itself, or a [`STATUS`] with [`WAITING`].
fn waits(message: &Value) -> bool {
    message["method"] == REQUEST
        || (message["method"] == STATUS
            && message["params"]["status"]["activeFlags"]
                .as_array()
                .is_some_and(|flags| flags.iter().any(|flag| flag == WAITING)))
}

/// A thread's status as text: its type and flags, such as `active [waitingOnApproval]`.
fn status(status: &Value) -> String {
    let flags: Vec<String> = status["activeFlags"]
        .as_array()
        .into_iter()
        .flatten()
        .map(text)
        .collect();
    if flags.is_empty() {
        text(&status["type"])
    } else {
        format!("{} [{}]", text(&status["type"]), flags.join(", "))
    }
}

/// Whether a hook's `input` is about the check's prompted shell command.
fn prompted(input: &Value) -> bool {
    input["tool_name"] == "Bash"
        && input["tool_input"]["command"]
            .as_str()
            .is_some_and(|command| command.contains(PROMPTED))
}

/// How the check answers the terminal's permission dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Yes,
    No,
}

impl Answer {
    pub fn keys(self) -> &'static [u8] {
        match self {
            Self::Yes => YES,
            Self::No => NO,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Yes => "yes",
            Self::No => "no",
        }
    }
}

/// A check's results: each returns its lines in the output.
#[derive(Debug)]
pub struct PermissionCheck {
    candidate: Candidate,
    checks: Checks,
}

impl PermissionCheck {
    pub fn new(candidate: Candidate) -> Self {
        Self {
            candidate,
            checks: Checks::default(),
        }
    }

    /// The session `name`, which `summary` sums up, whose dialog the check answered `answer`:
    /// whether it waited on its prompt, then whether the candidate told of it within
    /// [`WITHIN_MS`] and named its thread, and what came once the prompt was answered.
    pub fn prompted(
        &mut self,
        name: &str,
        observed: &Observed,
        answer: Answer,
        summary: &str,
    ) -> Vec<String> {
        let called = observed.called_ms();
        let (waited, details) = prompted_details(summary, called, observed.shown_ms, true);
        let mut lines = vec![
            self.checks
                .report(&format!("{name}, prompt"), waited, &details),
        ];
        let told = observed.told(self.candidate);
        let from = observed.shown_ms.or(called);
        let pass = matches!((&told, from), (Some((at, _)), Some(from)) if at.saturating_sub(from) <= WITHIN_MS);
        lines.push(self.checks.report(
            &format!("{name}, {}", self.candidate.name()),
            pass,
            &told_details(told.as_ref(), called, observed.shown_ms),
        ));
        lines.push(format!(
            "  {name}, after answering {} in the terminal: {}",
            answer.name(),
            came(&observed.after_answer())
        ));
        let between = match observed.session_between() {
            Some(entries) if entries.is_empty() => "nothing".to_owned(),
            Some(entries) => entries.join(", "),
            None => "no session file with the call".to_owned(),
        };
        lines.push(format!(
            "  {name}, session file between the call and its end: {between}"
        ));
        lines
    }

    /// The session `name` without a prompt, which `quiet` observed next to the session `beside`
    /// observed with its prompt: the candidate must name no session but `beside`'s, and must have
    /// had the chance to name `name`'s: the hooks or the app-server's watcher knew its thread.
    pub fn quiet(
        &mut self,
        name: &str,
        beside: &Observed,
        quiet: &Observed,
        summary: &str,
    ) -> String {
        let named = match self.candidate {
            Candidate::Notification => quiet.notified.iter().map(|n| n.text.clone()).collect(),
            _ => beside.others(self.candidate),
        };
        // The watcher must have known its thread; the hooks run only once a session has a turn.
        let known = !self.candidate.watches() || quiet.thread.is_some();
        let details = format!(
            "{summary}; its thread {}; {}",
            if known { "known" } else { "unknown" },
            if named.is_empty() {
                "named no other session".to_owned()
            } else {
                format!("named {}", named.join(", "))
            }
        );
        self.checks.report(
            &format!("{name}, without a prompt, {}", self.candidate.name()),
            known && named.is_empty(),
            &details,
        )
    }

    /// The check broke off with `error`: it fails.
    pub fn broke(&mut self, error: &str) -> String {
        self.checks
            .report("the check", false, &format!("broke off: {error}"))
    }

    pub fn pass(&self) -> bool {
        self.checks.pass()
    }

    /// The last line of the output, with the model of `options`.
    pub fn verdict(&self, options: &Options) -> String {
        self.checks
            .verdict(self.candidate.check(), Agent::Codex, options)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Recorded on Codex 0.162.1 by `live-check permission-hook codex --all` on the model stub,
    /// paths shortened: the hook runs of the session answered yes, `a`, then of the session answered
    /// no, `b`; and their session files, the model's instructions left out.
    const HOOK_RUNS: &str = include_str!("../tests/codex_permission/hook-runs.jsonl");
    const SESSION_YES: &str = include_str!("../tests/codex_permission/session-yes.jsonl");
    const SESSION_NO: &str = include_str!("../tests/codex_permission/session-no.jsonl");
    /// Recorded by `live-check permission-remote codex --all`: the watcher's messages next to the
    /// session answered yes, `a`, and the session without a prompt, `c`; then next to the session
    /// answered no; the lists of loaded threads and refused subscriptions left out.
    const WATCH_YES: &str = include_str!("../tests/codex_permission/watch-yes.jsonl");
    const WATCH_NO: &str = include_str!("../tests/codex_permission/watch-no.jsonl");
    /// Recorded by `live-check permission-notification codex --all`: the notification of the
    /// session answered no, read from its terminal.
    const NOTIFIED_NO: &str = include_str!("../tests/codex_permission/notified.jsonl");
    const A: &str = "01a1fcb4-1193-7453-8453-f71a429fde5e";
    const B: &str = "01a1fcb4-70eb-7240-a8a3-634cd5161296";

    /// The session in `/tmp/skald-live/<name>/project` that recorded `runs`, `watched` and
    /// `notified`, with its session files.
    fn observed(name: &str, runs: &str, watched_text: &str, notified_text: &str) -> Observed {
        let mut observed = Observed {
            runs: timed_runs(runs),
            watched: watched(watched_text),
            notified: notified(notified_text),
            sessions: vec![session_file(SESSION_YES), session_file(SESSION_NO)],
            ..Observed::default()
        };
        observed.thread = observed.thread_in(Path::new(&format!("/tmp/skald-live/{name}/project")));
        observed
    }

    fn only(runs: &str, session: &str) -> String {
        runs.lines()
            .filter(|line| line.contains(session))
            .map(|line| format!("{line}\n"))
            .collect()
    }

    /// What `after_answer` of `observed` gives, answered at `answered_ms`.
    fn after(observed: &mut Observed, answered_ms: u64) -> Vec<(String, u64)> {
        observed.answered_ms = Some(answered_ms);
        observed.after_answer()
    }

    fn pairs(pairs: &[(&str, u64)]) -> Vec<(String, u64)> {
        pairs
            .iter()
            .map(|(what, delay)| ((*what).to_owned(), *delay))
            .collect()
    }

    #[test]
    fn the_hook_finds_a_pending_approval_with_its_session_and_its_end() {
        let mut a = observed("a", HOOK_RUNS, "", "");
        assert_eq!(a.thread.as_deref(), Some(A));
        // The `PreToolUse` run, then the `PermissionRequest` run 64 ms later.
        assert_eq!(a.called_ms(), Some(1_795_241_022_534));
        let (told, carries) = a.told(Candidate::Hook).unwrap();
        assert_eq!(told, 1_795_241_022_598);
        assert_eq!(
            carries,
            "cwd, hook_event_name, model, permission_mode, session_id, tool_input, tool_name, \
             transcript_path, turn_id (Bash: touch prompted.txt)"
        );
        assert_eq!(
            after(&mut a, 1_795_241_032_900),
            pairs(&[("PostToolUse hook", 11), ("Stop hook", 67)])
        );

        // The session answered no: the hook told, and no hook runs after the answer.
        let mut b = observed("b", HOOK_RUNS, "", "");
        assert_eq!(b.thread.as_deref(), Some(B));
        assert_eq!(b.told(Candidate::Hook).unwrap().0, 1_795_241_046_532);
        assert!(after(&mut b, 1_795_241_046_600).is_empty());
    }

    #[test]
    fn the_hook_names_no_session_without_a_pending_approval() {
        // `b`'s prompt is another session's for `a`.
        let a = observed("a", HOOK_RUNS, "", "");
        assert_eq!(a.others(Candidate::Hook), [B]);
        let mut check = PermissionCheck::new(Candidate::Hook);
        let quiet = observed("c", HOOK_RUNS, "", "");
        assert_eq!(quiet.thread, None);
        assert_eq!(quiet.told(Candidate::Hook), None);
        assert!(
            check
                .quiet("c", &a, &quiet, "s")
                .contains(": fail — s; its thread known; named")
        );
        // Next to `a` alone, the session without a prompt is named nowhere.
        let alone = observed("a", &only(HOOK_RUNS, A), "", "");
        assert!(check.quiet("c", &alone, &quiet, "s").contains(": pass — "));
    }

    #[test]
    fn the_app_server_finds_a_pending_approval_with_its_session_and_its_end() {
        let mut a = observed("a", "", WATCH_YES, "");
        assert_eq!(
            a.thread.as_deref(),
            Some("01a1fceb-da19-7203-8bb9-807cd2c5b8f6")
        );
        // The `item/started` of the command, not that of `sleep 2` before it.
        assert_eq!(a.called_ms(), Some(1_795_244_681_115));
        assert_eq!(
            a.told(Candidate::Remote),
            Some((
                1_795_244_681_115,
                format!(
                    "status, threadId ({STATUS} active [waitingOnApproval]); {REQUEST} reached the \
                     watcher"
                )
            ))
        );
        assert_eq!(
            after(&mut a, 1_795_244_691_300),
            pairs(&[
                (RESOLVED, 11),
                ("thread/status/changed active", 12),
                ("item/completed commandExecution completed", 65),
                ("thread/status/changed idle", 176),
            ])
        );

        let mut b = observed("b", "", WATCH_NO, "");
        assert_eq!(
            b.thread.as_deref(),
            Some("01a1fcec-419d-7810-b269-b17720bf30e1")
        );
        assert_eq!(b.told(Candidate::Daemon).unwrap().0, 1_795_244_707_169);
        assert_eq!(
            after(&mut b, 1_795_244_717_300),
            pairs(&[
                (RESOLVED, 9),
                ("thread/status/changed active", 9),
                ("item/completed commandExecution declined", 9),
                ("thread/status/changed idle", 48),
            ])
        );
    }

    #[test]
    fn the_app_server_names_no_session_without_a_pending_approval() {
        let a = observed("a", "", WATCH_YES, "");
        assert!(a.others(Candidate::Remote).is_empty());
        let quiet = observed("c", "", WATCH_YES, "");
        assert_eq!(
            quiet.thread.as_deref(),
            Some("01a1fceb-daba-78e0-ae06-894f97fe26f7")
        );
        assert_eq!(quiet.told(Candidate::Remote), None);
        let mut check = PermissionCheck::new(Candidate::Remote);
        assert!(
            check
                .quiet("c", &a, &quiet, "s")
                .contains(": pass — s; its thread known")
        );
        // Seen from the session without a prompt, `a`'s prompt is another session's.
        assert_eq!(
            quiet.others(Candidate::Remote).len(),
            2,
            "the status and the request"
        );
        // A watcher that never learned of the session's thread could not have named it.
        let unknown = observed("d", "", WATCH_YES, "");
        assert!(
            check
                .quiet("d", &a, &unknown, "s")
                .contains(": fail — s; its thread unknown")
        );
    }

    #[test]
    fn the_notification_comes_from_the_terminal_of_the_session_that_waits() {
        let b = observed("b", "", "", NOTIFIED_NO);
        let (_, carries) = b.told(Candidate::Notification).unwrap();
        assert_eq!(
            carries,
            "the text \"Approval requested: /bin/zsh -lc 'touch prompte...\""
        );
        let quiet = observed("c", "", "", "");
        assert_eq!(quiet.told(Candidate::Notification), None);
        let mut check = PermissionCheck::new(Candidate::Notification);
        assert!(check.quiet("c", &b, &quiet, "s").contains(": pass — "));
        let noisy = observed("c", "", "", NOTIFIED_NO);
        assert!(
            check
                .quiet("c", &b, &noisy, "s")
                .contains(": fail — s; its thread known; named Approval requested")
        );
    }

    #[test]
    fn the_session_file_holds_no_entry_while_the_approval_waits() {
        let a = observed("a", HOOK_RUNS, "", "");
        assert_eq!(a.session_between().unwrap(), ["token_usage_record"]);
        let b = observed("b", HOOK_RUNS, "", "");
        assert_eq!(b.session_between().unwrap(), ["token_usage_record"]);
        let quiet = observed("c", HOOK_RUNS, "", "");
        assert_eq!(quiet.session_between(), None);
    }

    #[test]
    fn a_session_asks_before_the_escalated_command_and_records_on_the_stub() {
        let files = Files::new(Path::new("/tmp/skald-live/a"));
        let (runs, live) = (Path::new("/tmp/runs.jsonl"), Path::new("/bin/live-check"));
        let stub = Some("http://127.0.0.1:9");
        for candidate in Candidate::ALL {
            let socket = Path::new("/tmp/a.sock");
            let args = session(candidate, &files, runs, live, Some(socket), Some("m"), stub);
            let text = args.join(" ");
            assert!(!text.contains("never"), "{text}");
            assert_eq!(
                text.contains("--remote unix:///tmp/a.sock"),
                candidate == Candidate::Remote
            );
            assert_eq!(text.contains("--no-daemon"), !candidate.watches());
            assert_eq!(
                text.contains("approval-requested"),
                candidate == Candidate::Notification
            );
            if candidate == Candidate::Daemon {
                assert_eq!(
                    args,
                    ["-C", "/tmp/skald-live/a/project"],
                    "no config override"
                );
                continue;
            }
            assert!(text.contains("approval_policy=\"on-request\""), "{text}");
            assert!(
                text.contains("base_url=\"http://127.0.0.1:9/v1\""),
                "{text}"
            );
            assert!(text.contains("hooks.PermissionRequest="), "{text}");
        }
        let quiet = Path::new("/tmp/skald-live/c/project");
        let toml = daemon_config(live, runs, &[&files.project, quiet], Some("m"), stub);
        assert!(
            toml.contains("model=\"m\"\n") && toml.contains("features.plugins=false\n"),
            "{toml}"
        );
        for value in [
            "approval_policy=\"on-request\"\n",
            "base_url=\"http://127.0.0.1:9/v1\"\n",
            "hooks.PermissionRequest=",
        ] {
            assert!(toml.contains(value), "{value} in {toml}");
        }
        assert_eq!(toml.matches("projects=").count(), 1, "{toml}");
        assert!(
            toml.contains("\"/tmp/skald-live/c/project\"={trust_level=\"trusted\"}"),
            "{toml}"
        );
        let server = app_server(
            Path::new("/tmp/a.sock"),
            &[&files.project, quiet],
            runs,
            live,
            None,
            stub,
        );
        let text = server.join(" ");
        assert!(
            text.starts_with("app-server --listen unix:///tmp/a.sock"),
            "{text}"
        );
        assert_eq!(text.matches("projects=").count(), 1, "{text}");
        assert!(
            text.contains("\"/tmp/skald-live/c/project\"={trust_level"),
            "{text}"
        );
        let Call::Tool { name, input } = script()[1] else {
            panic!("{:?}", script());
        };
        let input: Value = serde_json::from_str(input).unwrap();
        assert_eq!((name, &input["cmd"]), ("exec_command", &json!(PROMPTED)));
        assert_eq!(input["sandbox_permissions"], "require_escalated");
    }

    #[test]
    fn each_candidate_has_a_check_of_its_own() {
        for candidate in Candidate::ALL {
            assert_eq!(Candidate::parse(candidate.check()), Some(candidate));
        }
        assert_eq!(Candidate::parse("permission"), None);
        let names: Vec<_> = Candidate::ALL
            .map(|candidate| (candidate.name(), candidate.takes_real()))
            .into();
        assert_eq!(
            names,
            [
                ("app-server request, --remote", true),
                ("app-server request, background app-server", false),
                ("PermissionRequest hook", true),
                ("approval-requested notification", true),
            ]
        );
        assert!(Candidate::Remote.watches() && Candidate::Daemon.watches());
        assert!(!Candidate::Hook.watches() && !Candidate::Notification.watches());
        assert_eq!(
            prompt(),
            "Run the shell command `touch prompted.txt`. Then reply with only OK."
        );
        assert_eq!(
            [Answer::Yes, Answer::No].map(|answer| (answer.keys(), answer.name())),
            [(b"y".as_slice(), "yes"), (b"\x1b".as_slice(), "no")]
        );
        assert_eq!(
            recorder(Path::new("/bin/live-check"), Path::new("/tmp/runs.jsonl")),
            "'/bin/live-check' 'timed-hook' '/tmp/runs.jsonl' 'true'"
        );
    }

    /// A hook run's line of `event` in the session `session` at `ms`, about the shell command
    /// `command` if given.
    fn run(event: &str, ms: u64, session: &str, command: Option<&str>) -> String {
        let mut input = json!({ "session_id": session, "cwd": "/tmp/skald-live/a/project" });
        if let Some(command) = command {
            input["tool_name"] = json!("Bash");
            input["tool_input"] = json!({ "command": command });
        }
        format!(
            "{}\n",
            json!({ "hook_event_name": event, "started_ms": ms, "input": input })
        )
    }

    #[test]
    fn only_the_prompted_command_of_its_own_session_counts() {
        let touch = Some(PROMPTED);
        let runs = [
            run("SessionStart", 1, A, None),
            run("PreToolUse", 2, A, Some("sleep 2")),
            run("PreToolUse", 3, B, touch),
            run("PermissionRequest", 4, B, touch),
            run("PostToolUse", 5, A, touch),
            run("PreToolUse", 6, A, touch),
            run("PermissionRequest", 7, A, Some("sleep 2")),
            run("PermissionRequest", 8, A, touch),
            run("PreToolUse", 20, A, touch),
            run("PostToolUse", 21, A, Some("sleep 2")),
            run("PostToolUse", 22, A, touch),
            run("Stop", 23, B, None),
            run("Stop", 24, A, None),
        ]
        .concat();
        let mut a = observed("a", &runs, "", "");
        assert_eq!(a.thread.as_deref(), Some(A));
        assert_eq!(a.called_ms(), Some(6));
        assert_eq!(a.told(Candidate::Hook).unwrap().0, 8);
        assert_eq!(a.others(Candidate::Hook), [B]);
        assert_eq!(
            after(&mut a, 10),
            pairs(&[("PostToolUse hook", 12), ("Stop hook", 14)])
        );
        // Without the answer's time, from the call.
        a.answered_ms = None;
        assert_eq!(a.after_answer()[0], ("PostToolUse hook".to_owned(), 16));
        assert!(!prompted(
            &json!({ "tool_name": "Edit", "tool_input": { "command": PROMPTED } })
        ));
    }

    /// An app-server message's line of the watcher's file.
    fn message(at_ms: u64, method: &str, params: &Value) -> String {
        format!(
            "{}\n",
            watched_line(at_ms, &json!({ "method": method, "params": params }))
        )
    }

    #[test]
    fn only_the_messages_of_its_own_thread_count() {
        let thread =
            |id: &str| json!({ "thread": { "id": id, "cwd": "/tmp/skald-live/a/project" } });
        let item = |id: &str, kind: &str, command: &str, status: &str| json!({ "threadId": id, "item": { "type": kind, "command": command, "status": status } });
        let waiting = json!({ "type": "active", "activeFlags": [WAITING] });
        let watch = [
            // Another method that carries a thread is no thread of the watcher's.
            message(1, "thread/archived", &thread(B)),
            message(2, "thread/started", &thread(A)),
            message(
                3,
                "item/started",
                &item(B, "commandExecution", PROMPTED, ""),
            ),
            message(
                4,
                "item/started",
                &item(A, "commandExecution", "sleep 2", ""),
            ),
            message(5, "item/started", &item(A, "agentMessage", PROMPTED, "")),
            message(6, REQUEST, &json!({ "threadId": B, "command": PROMPTED })),
            message(
                7,
                STATUS,
                &json!({ "threadId": A, "status": { "type": "active" } }),
            ),
            message(
                8,
                "item/started",
                &item(A, "commandExecution", PROMPTED, ""),
            ),
            message(9, STATUS, &json!({ "threadId": A, "status": waiting })),
            message(20, RESOLVED, &json!({ "threadId": A })),
            message(
                21,
                "item/completed",
                &item(A, "agentMessage", "", "completed"),
            ),
            message(
                22,
                "item/completed",
                &item(A, "commandExecution", "", "declined"),
            ),
            message(23, RESOLVED, &json!({ "threadId": B })),
        ]
        .concat();
        let mut a = observed("a", "", &watch, "");
        assert_eq!(a.watched_threads().len(), 1);
        assert_eq!(a.thread.as_deref(), Some(A));
        assert_eq!(a.called_ms(), Some(8));
        assert_eq!(
            a.told(Candidate::Daemon),
            Some((
                9,
                format!(
                    "status, threadId ({STATUS} active [waitingOnApproval]); {REQUEST} never \
                     reached the watcher"
                )
            ))
        );
        assert_eq!(a.others(Candidate::Daemon), [B]);
        assert_eq!(
            after(&mut a, 10),
            pairs(&[
                (RESOLVED, 10),
                ("item/completed commandExecution declined", 12)
            ])
        );
    }

    /// The entries of a session file of the thread `A`, with a call of `sleep 2`, `call_0`, then of
    /// the prompted command, `call_1`, and `between` after it.
    fn session_of(between: &[Value]) -> Vec<Value> {
        let call = |id: &str, command: &str| {
            json!({ "type": "response_item", "payload": {
                "type": "function_call", "call_id": id, "arguments": json!({ "cmd": command }).to_string(),
            } })
        };
        let mut entries = vec![
            json!({ "type": "session_meta", "payload": { "id": A } }),
            call("call_0", "sleep 2"),
            json!({ "type": "response_item", "payload": { "type": "function_call_output", "call_id": "call_0" } }),
            call("call_1", PROMPTED),
        ];
        entries.extend_from_slice(between);
        entries
    }

    #[test]
    fn the_session_file_ends_the_call_at_its_output_or_its_completed_item() {
        let output = |id: &str| json!({ "type": "response_item", "payload": { "type": "function_call_output", "call_id": id } });
        let completed = |id: &str| json!({ "type": "event_msg", "payload": { "type": "item_completed", "item": { "id": id } } });
        let usage = json!({ "type": "token_usage_record" });
        let a = |between: &[Value]| Observed {
            sessions: vec![session_file(SESSION_NO), session_of(between)],
            thread: Some(A.to_owned()),
            ..Observed::default()
        };
        let other = [usage.clone(), output("call_0"), completed("call_0")];
        let mut ended = other.to_vec();
        ended.extend([completed("call_1"), usage.clone()]);
        assert_eq!(
            a(&ended).session_between().unwrap(),
            [
                "token_usage_record",
                "response_item/function_call_output",
                "event_msg/item_completed"
            ]
        );
        let mut ended = other.to_vec();
        ended.extend([output("call_1"), usage]);
        assert_eq!(a(&ended).session_between().unwrap().len(), 3);
        assert_eq!(a(&[]).session_between(), Some(vec![]));
        let mut check = PermissionCheck::new(Candidate::Hook);
        let lines = check.prompted("a", &a(&[]), Answer::Yes, "");
        assert_eq!(
            lines[3],
            "  a, session file between the call and its end: nothing"
        );
    }

    #[test]
    fn a_check_reports_each_line_from_the_recordings() {
        let mut a = observed("a", HOOK_RUNS, "", "");
        a.shown_ms = Some(1_795_241_022_700);
        a.answered_ms = Some(1_795_241_032_700);
        let mut check = PermissionCheck::new(Candidate::Hook);
        assert_eq!(
            check.prompted("a", &a, Answer::Yes, "replies []"),
            [
                "  a, prompt: pass — replies []; the agent called `touch prompted.txt`; the dialog \
                 showed 0.2 s",
                "  a, PermissionRequest hook: pass — told 0.1 s after the call, -0.1 s after the \
                 dialog showed; carries cwd, hook_event_name, model, permission_mode, session_id, \
                 tool_input, tool_name, transcript_path, turn_id (Bash: touch prompted.txt)",
                "  a, after answering yes in the terminal: PostToolUse hook 0.2 s, Stop hook 0.3 s",
                "  a, session file between the call and its end: token_usage_record",
            ]
        );
        assert!(check.pass());
        // A candidate that tells later than [`WITHIN_MS`] after the dialog showed fails.
        a.shown_ms = Some(1_795_241_022_598 - WITHIN_MS - 1);
        let lines = check.prompted("a", &a, Answer::Yes, "");
        assert!(lines[1].starts_with("  a, PermissionRequest hook: fail — "));
        assert!(!check.pass());

        let mut check = PermissionCheck::new(Candidate::Notification);
        let quiet = observed("c", "", "", "");
        let lines = check.prompted("c", &quiet, Answer::No, "s");
        assert_eq!(
            lines[..3],
            [
                "  c, prompt: fail — s; the agent never called `touch prompted.txt`; the dialog \
                 never showed",
                "  c, approval-requested notification: fail — did not tell",
                "  c, after answering no in the terminal: nothing",
            ]
        );
        assert_eq!(check.broke("lost"), "  the check: fail — broke off: lost");
        assert_eq!(
            check.verdict(&Options::default()),
            "permission-notification on codex with the model stub: fail"
        );
    }

    #[test]
    fn the_check_reads_its_recordings_from_the_session_directory() {
        let dir = tempfile::tempdir().unwrap();
        let (runs, watch) = (dir.path().join("runs.jsonl"), dir.path().join(WATCH));
        fs::write(&runs, HOOK_RUNS).unwrap();
        let line = watched_line(5, &json!({ "method": STATUS }));
        fs::write(&watch, format!("{line}\nnot json\n")).unwrap();
        let notification = Notified {
            at_ms: 7,
            text: "Approval requested".to_owned(),
        };
        fs::write(dir.path().join(NOTIFIED), notified_line(&notification)).unwrap();
        let day = dir
            .path()
            .join("home/.codex")
            .join(SESSIONS)
            .join("2026/11/21");
        fs::create_dir_all(&day).unwrap();
        fs::write(day.join("rollout-a.jsonl"), SESSION_YES).unwrap();
        fs::write(day.join("rollout-a.json"), SESSION_NO).unwrap();
        let read = Observed::read(&runs, &watch, dir.path());
        assert_eq!(read.runs, timed_runs(HOOK_RUNS));
        assert_eq!(
            read.watched,
            [Watched {
                at_ms: 5,
                message: json!({ "method": STATUS })
            }]
        );
        assert_eq!(read.notified, [notification]);
        assert_eq!(read.sessions, [session_file(SESSION_YES)]);
        let missing = Observed::read(&dir.path().join("none"), &dir.path().join("none"), &day);
        assert_eq!(missing, Observed::default());
    }

    #[test]
    fn the_check_finds_the_background_app_servers_socket() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(daemon_socket(dir.path()), None);
        let control = dir.path().join("app-server-control");
        fs::create_dir_all(&control).unwrap();
        for name in ["app-server-startup.lock", "other.sock"] {
            fs::write(control.join(name), "").unwrap();
        }
        assert_eq!(daemon_socket(dir.path()), None);
        let socket = control.join("app-server-control.sock");
        fs::write(&socket, "").unwrap();
        assert_eq!(daemon_socket(dir.path()), Some(socket));
        assert_eq!(daemon_socket(&dir.path().join("none")), None);
    }

    #[test]
    fn the_osc9_scanner_finds_each_notification_across_chunks() {
        let mut osc9 = Osc9::default();
        assert!(
            osc9.feed(b"text \x1b[1mbold\x1b]0;title\x07 \x1b]9;Appro")
                .is_empty()
        );
        assert_eq!(
            osc9.feed(b"val requested: touch\x07more"),
            ["Approval requested: touch"]
        );
        assert_eq!(
            osc9.feed(b"\x1b]9;two\x1b\\\x1b]9;three\x07"),
            ["two", "three"]
        );
        // A BEL alone, or another OSC, is none.
        assert!(osc9.feed(b"\x07\x1b]99;no\x07").is_empty());
        // ESC in the text without `\` stays in it.
        assert_eq!(osc9.feed(b"\x1b]9;a\x1bxb\x07"), ["a\x1bxb"]);
    }
}
