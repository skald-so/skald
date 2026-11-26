//! What the tests share: Codex's hook inputs and the client name it gives at initialization,
//! recorded from the real Codex; Codex sessions, each a thread with its folder, its config home and
//! its agent; runs of the hook handler as Codex runs them, through the copy of `skald` of the tests
//! of the helper; what a run gives the session; and a stand-in Codex process, which runs a session's
//! hooks as its children until a test ends it.

use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::time::{Duration, Instant};
use std::{env, fs, sync::mpsc, thread};

use serde_json::{Value, json};
use skald_proto::MessageId;

use crate::cloud::WAIT;
use crate::helped::Helped;
use crate::hooks::{HOOK, Hooked, INBOX_WORD, Web, run_hook};
use crate::lines::{agent_lines, assert_no_uuid, lines_referring_to};
use crate::machine::{Machine, RUN_LIMIT};
use crate::senders::data_warnings;
use crate::world::AGENT;

/// The agent CLI `skald hook` takes for Codex.
pub const CODEX: &str = "codex";

/// The setting that names Codex's config home, which Codex's hooks inherit.
pub const CODEX_HOME: &str = "CODEX_HOME";

/// Codex's hook output limit is about 2,500 tokens. A token takes at least a byte, so an output
/// of at most this many bytes is within it however its text splits into tokens.
pub const LIMIT: usize = 2_500;

/// How long a test watches for what must not happen.
pub const QUIET: Duration = Duration::from_secs(3);

/// Hook inputs recorded from Codex 0.162.1 by the live checks of the delivery prototypes
/// (`docs/dev/agent-delivery.md`, "Codex hooks"), with the paths of their temporary directories.
const SESSION_START: &str = include_str!("inputs/session-start.json");
const POST_TOOL_USE: &str = include_str!("inputs/post-tool-use.json");
const USER_PROMPT_SUBMIT: &str = include_str!("inputs/user-prompt-submit.json");
const STOP: &str = include_str!("inputs/stop.json");
/// The `Stop` input in a turn a `Stop` hook kept going: `stop_hook_active` is true.
const STOP_ACTIVE: &str = include_str!("inputs/stop-active.json");

/// The `initialize` request Codex 0.162.1 sends an MCP server it starts.
const INITIALIZE: &str = include_str!("inputs/initialize.json");

/// The client name Codex gives an MCP server at initialization, as recorded.
pub fn codex_client() -> String {
    let request: Value = serde_json::from_str(INITIALIZE).expect("a recorded request is JSON");
    (request["params"]["clientInfo"]["name"].as_str())
        .expect("a client name")
        .to_owned()
}

/// A hook event, as Codex runs the handler on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    SessionStart,
    PostToolUse,
    UserPromptSubmit,
    /// `Stop`, the first in its turn.
    Stop,
    /// `Stop` in a turn a `Stop` hook kept going.
    StopActive,
}

impl Event {
    /// Every event, in the order of a session's first turn.
    pub const ALL: [Event; 5] = [
        Event::SessionStart,
        Event::UserPromptSubmit,
        Event::PostToolUse,
        Event::Stop,
        Event::StopActive,
    ];

    /// The events that give the signal while a message waits: the end of a turn, then the next
    /// turn's start and a tool call in it, so the session is busy after them.
    pub const TURN: [Event; 4] = [
        Event::Stop,
        Event::StopActive,
        Event::UserPromptSubmit,
        Event::PostToolUse,
    ];

    /// The event's `hook_event_name`.
    pub fn name(self) -> &'static str {
        match self {
            Event::SessionStart => "SessionStart",
            Event::PostToolUse => "PostToolUse",
            Event::UserPromptSubmit => "UserPromptSubmit",
            Event::Stop | Event::StopActive => "Stop",
        }
    }

    fn recorded(self) -> &'static str {
        match self {
            Event::SessionStart => SESSION_START,
            Event::PostToolUse => POST_TOOL_USE,
            Event::UserPromptSubmit => USER_PROMPT_SUBMIT,
            Event::Stop => STOP,
            Event::StopActive => STOP_ACTIVE,
        }
    }
}

/// A Codex session: its thread, its folder, its config home and the agent its hooks and its
/// `skald mcp` take with `--agent`, if any.
#[derive(Debug, Clone)]
pub struct Thread {
    pub id: String,
    pub dir: PathBuf,
    pub home: PathBuf,
    pub agent: Option<String>,
}

impl Thread {
    /// A new session in `dir` with the config home `home/.codex` of `machine`, as `agent`.
    pub fn new(machine: &Machine, dir: &Path, agent: Option<&str>) -> Self {
        let home = machine.root().join("home").join(".codex");
        fs::create_dir_all(&home).expect("make the config home");
        Thread {
            id: MessageId::generate().to_string(),
            dir: dir.to_owned(),
            home,
            agent: agent.map(str::to_owned),
        }
    }

    /// Its rollout, the session file in its config home that `SessionStart` names.
    pub fn rollout(&self) -> PathBuf {
        (self.home.join("sessions")).join(format!("rollout-2026-11-12T15-01-04-{}.jsonl", self.id))
    }

    /// The recorded input of `event` as this session's: its thread, its folder and its rollout.
    pub fn input(&self, event: Event) -> String {
        let mut input: Value =
            serde_json::from_str(event.recorded()).expect("a recorded input is JSON");
        input["session_id"] = json!(self.id);
        input["cwd"] = json!(self.dir.to_string_lossy());
        if input["transcript_path"].is_string() {
            input["transcript_path"] = json!(self.rollout().to_string_lossy());
        }
        input.to_string()
    }
}

/// The hook handler's command as Codex runs it for `of`: the copy of `helped`, in the session's
/// folder, with its config home and `settings`.
pub fn hook_command(helped: &Helped, of: &Thread, settings: &[(&str, &str)]) -> Command {
    let home = of.home.to_string_lossy().into_owned();
    let mut given = vec![(CODEX_HOME, home.as_str())];
    given.extend_from_slice(settings);
    let mut args: Vec<&str> = Vec::new();
    if let Some(agent) = &of.agent {
        args.extend([AGENT, agent.as_str()]);
    }
    args.extend([HOOK, CODEX]);
    helped.command_in(&of.dir, &given, &args)
}

/// What `hooked`, a run on `event`, gives the session: on `Stop`, the `reason` of
/// `decision: "block"`, which continues the turn with it; on another event, the
/// `additionalContext` of `hookSpecificOutput`, whose `hookEventName` is the event's. Fails unless
/// the run exited with success, printed nothing on its standard error, and its output has at most
/// [`LIMIT`] bytes.
#[track_caller]
pub fn given(hooked: &Hooked, event: Event) -> String {
    assert_eq!(hooked.code, Some(0), "the hook failed: {hooked:?}");
    assert!(hooked.stderr.is_empty(), "the hook printed {hooked:?}");
    assert!(
        hooked.stdout.len() <= LIMIT,
        "{} bytes, over the limit",
        hooked.stdout.len()
    );
    let output: Value = serde_json::from_str(hooked.stdout.trim())
        .unwrap_or_else(|err| panic!("not one JSON object ({err}): {hooked:?}"));
    let text = if matches!(event, Event::Stop | Event::StopActive) {
        assert_eq!(output["decision"].as_str(), Some("block"), "{output}");
        output["reason"].as_str()
    } else {
        let specific = &output["hookSpecificOutput"];
        assert_eq!(
            specific["hookEventName"].as_str(),
            Some(event.name()),
            "{output}"
        );
        specific["additionalContext"].as_str()
    };
    (text.unwrap_or_else(|| panic!("no text for the session: {output}"))).to_owned()
}

/// Fails unless `text`, what a hook gave, is the signal alone: one line that names
/// [`INBOX_WORD`]; no message line and no warning line; none of `texts`, nor their first 40
/// characters; none of `names`, the senders' handles and devices, in any case; no UUID; and no
/// `#<ref>` of any of `msg_ids`.
#[track_caller]
pub fn assert_bare(text: &str, texts: &[&str], msg_ids: &[MessageId], names: &[&str]) {
    assert_eq!(text.trim().lines().count(), 1, "not one line: {text}");
    assert!(text.contains(INBOX_WORD), "no {INBOX_WORD}: {text}");
    assert!(agent_lines(text).is_empty(), "a message line: {text}");
    assert!(data_warnings(text).is_empty(), "a warning line: {text}");
    for sent in texts {
        let head: String = sent.chars().take(40).collect();
        assert!(!text.contains(&head), "the text {head:?}: {text}");
    }
    let lower = text.to_lowercase();
    for name in names {
        assert!(!lower.contains(&name.to_lowercase()), "{name:?}: {text}");
    }
    assert_no_uuid(text);
    for &msg_id in msg_ids {
        assert!(
            lines_referring_to(text, msg_id).is_empty(),
            "the reference of {msg_id}: {text}"
        );
    }
}

/// Ada's machine alone in `clienta` with web's MCP session, as the tests of Claude Code's hooks
/// have it, and the copy of `skald` that Codex's hooks run without the one-shot setting, so a
/// session's `SessionStart` starts the helper.
pub struct Codex {
    /// Kills the helpers before the machine's directory goes.
    pub helped: Helped,
    pub web: Web,
}

impl Codex {
    /// Ada, GitHub user `id` with `login`, alone in `clienta`.
    pub fn new(id: u64, login: &str) -> Self {
        let web = Web::new(id, login);
        let helped = Helped::new(web.machine());
        Codex { helped, web }
    }

    pub fn machine(&self) -> &Machine {
        self.web.machine()
    }

    /// A new session of Ada's agent `agent`, `--agent <agent>`, in its folder
    /// `~/work/clientA/<agent>`.
    pub fn thread_of(&self, agent: &str) -> Thread {
        Thread::new(self.machine(), &self.web.solo.dir(agent), Some(agent))
    }

    /// `of`'s hook of `event`, run by Codex in this process, with `settings`.
    pub fn hook_with(&self, of: &Thread, event: Event, settings: &[(&str, &str)]) -> Hooked {
        run_hook(
            &mut hook_command(&self.helped, of, settings),
            &of.input(event),
        )
    }

    /// `of`'s hook of `event`, run by Codex in this process.
    pub fn hook(&self, of: &Thread, event: Event) -> Hooked {
        self.hook_with(of, event, &[])
    }

    /// `of`'s hook of `event`, run by the Codex process `codex`.
    pub fn hook_by(&self, codex: &mut CodexProcess, of: &Thread, event: Event) -> Hooked {
        codex.run(&hook_command(&self.helped, of, &[]), &of.input(event))
    }

    /// `of`'s `SessionStart`, run by Codex in this process; fails unless it prints nothing.
    #[track_caller]
    pub fn starts(&self, of: &Thread) {
        self.hook(of, Event::SessionStart)
            .assert_silent("SessionStart");
    }

    /// A turn of `of` starts, so the session is busy: its `UserPromptSubmit`, run by Codex in this
    /// process with nothing new waiting, must print nothing.
    #[track_caller]
    pub fn busy(&self, of: &Thread) {
        self.silent(of, Event::UserPromptSubmit, "the turn's start");
    }

    /// Fails unless `of`'s hook of `event` prints nothing.
    #[track_caller]
    pub fn silent(&self, of: &Thread, event: Event, what: &str) {
        let hooked = self.hook(of, event);
        hooked.assert_silent(&format!("{what}: {}", event.name()));
    }

    /// Runs `of`'s hook of `event` every half second until it gives the session something, for up
    /// to [`WAIT`]; each run before that must print nothing. Gives what it gave.
    #[track_caller]
    pub fn gives(&self, of: &Thread, event: Event) -> String {
        gives_by(event, || self.hook(of, event))
    }
}

/// Runs `hook`, a run of a session's hook of `event`, every half second until it gives the session
/// something, for up to [`WAIT`]; each run before that must print nothing. Gives what it gave.
#[track_caller]
pub fn gives_by(event: Event, mut hook: impl FnMut() -> Hooked) -> String {
    let deadline = Instant::now() + WAIT;
    let mut hooked = hook();
    while hooked.stdout.is_empty() {
        hooked.assert_silent(&format!("{} with nothing to give", event.name()));
        assert!(
            Instant::now() < deadline,
            "{} gave nothing within {WAIT:?}",
            event.name()
        );
        thread::sleep(Duration::from_millis(500));
        hooked = hook();
    }
    given(&hooked, event)
}

/// The setting that makes a run of this test binary play a Codex process.
const PLAY: &str = "SKALD_TEST_CODEX_PROCESS";

/// The test of this binary whose run plays it.
const PLAYER: &str = "codex::codex_process";

/// What starts each line in which the played process tells how a run went.
const TOLD: &str = "codex process ran: ";

/// A process that plays a Codex process: a run of this test binary, which runs each command it is
/// given as its own child, as Codex runs a session's hooks, and runs on until it is ended. Killed
/// on drop.
pub struct CodexProcess {
    child: Child,
    input: ChildStdin,
    told: mpsc::Receiver<String>,
}

impl CodexProcess {
    pub fn start() -> Self {
        CodexProcess::start_as(&env::current_exe().expect("this test binary"))
    }

    /// As [`CodexProcess::start`], run from `exe`, a copy of this test binary.
    pub fn start_as(exe: &Path) -> Self {
        let mut child = Command::new(exe)
            .args([PLAYER, "--exact", "--ignored", "--nocapture"])
            .env(PLAY, "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("run this test binary as a Codex process");
        let input = child.stdin.take().expect("stdin is piped");
        let output = child.stdout.take().expect("stdout is piped");
        let (tell, told) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(output).lines().map_while(Result::ok) {
                let Some(ran) = line.strip_prefix(TOLD) else {
                    continue;
                };
                if tell.send(ran.to_owned()).is_err() {
                    return;
                }
            }
        });
        CodexProcess { child, input, told }
    }

    /// Runs `command` as a child of the process, with `input` on its standard input; gives how it
    /// went.
    #[track_caller]
    pub fn run(&mut self, command: &Command, input: &str) -> Hooked {
        let lossy = |text: &std::ffi::OsStr| text.to_string_lossy().into_owned();
        let settings: Vec<[String; 2]> = (command.get_envs())
            .filter_map(|(name, value)| Some([lossy(name), lossy(value?)]))
            .collect();
        let asked = json!({
            "program": lossy(command.get_program()),
            "args": command.get_args().map(lossy).collect::<Vec<_>>(),
            "dir": command.get_current_dir().map(|dir| lossy(dir.as_os_str())),
            "settings": settings,
            "input": input,
        });
        writeln!(self.input, "{asked}").expect("ask the Codex process");
        self.input.flush().expect("ask the Codex process");
        let ran = (self.told.recv_timeout(RUN_LIMIT))
            .unwrap_or_else(|_| panic!("the Codex process told nothing within {RUN_LIMIT:?}"));
        let ran: Value = serde_json::from_str(&ran).expect("how the run went");
        let text = |field: &str| ran[field].as_str().unwrap_or_default().to_owned();
        Hooked {
            code: ran["code"]
                .as_i64()
                .and_then(|code| i32::try_from(code).ok()),
            stdout: text("stdout"),
            stderr: text("stderr"),
        }
    }

    /// Ends the process, as Codex exits, and waits until it no longer runs.
    pub fn end(mut self) {
        self.kill();
    }

    fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for CodexProcess {
    fn drop(&mut self) {
        self.kill();
    }
}

/// The Codex process [`CodexProcess`] runs: acts only in a run with [`PLAY`] set.
#[test]
#[ignore = "the Codex process the tests of a session's lifetime run; it acts only in their runs"]
fn codex_process() {
    if plays() {
        play();
    }
}

/// Whether this run of the test binary is one [`CodexProcess`] started.
pub fn plays() -> bool {
    env::var_os(PLAY).is_some()
}

/// Plays a Codex process: each line of its standard input asks it to run a command; it runs it as
/// its child and prints how it went.
pub fn play() {
    for line in io::stdin().lock().lines().map_while(Result::ok) {
        let asked: Value = serde_json::from_str(&line).expect("a command to run");
        let strings = |field: &str| -> Vec<String> {
            (asked[field].as_array().map(Vec::as_slice))
                .unwrap_or_default()
                .iter()
                .filter_map(|value| value.as_str().map(str::to_owned))
                .collect()
        };
        let mut command = Command::new(asked["program"].as_str().expect("a program"));
        command.args(strings("args")).env_clear();
        for pair in asked["settings"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default()
        {
            command.env(
                pair[0].as_str().unwrap_or_default(),
                pair[1].as_str().unwrap_or_default(),
            );
        }
        if let Some(dir) = asked["dir"].as_str() {
            command.current_dir(dir);
        }
        let mut child = (command.stdin(Stdio::piped()))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("run the command");
        let mut stdin = child.stdin.take().expect("stdin is piped");
        // A command that exits before it reads its input closes it.
        let _ = stdin.write_all(asked["input"].as_str().unwrap_or_default().as_bytes());
        drop(stdin);
        let output = child.wait_with_output().expect("wait for the command");
        let ran = json!({
            "code": output.status.code(),
            "stdout": String::from_utf8_lossy(&output.stdout),
            "stderr": String::from_utf8_lossy(&output.stderr),
        });
        println!("{TOLD}{ran}");
    }
}
