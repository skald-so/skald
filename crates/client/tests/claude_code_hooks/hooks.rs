//! What the tests share: Ada's agent web with its MCP session in `~/work/clientA/web`, the hook
//! inputs recorded from Claude Code, runs of the hook handler on them, and what a run gave.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;

use serde_json::{Value, json};
use skald_proto::{ChannelId, MessageId};

use crate::client::Session;
use crate::fixtures::{SEND, TO};
use crate::lines::{
    AgentLine, agent_line, agent_lines, assert_handed_line, assert_says_sent, refers_to,
};
use crate::machine::{Finished, Machine, RUN_LIMIT};
use crate::senders::data_warnings;
use crate::solo::{API, EVERYONE, Solo, WEB};
use crate::talk::messages_sent;
use crate::world::{AGENT, Person, dir_of};

/// The hook handler's command and its agent CLI.
pub const HOOK: &str = "hook";
pub const CLAUDE_CODE: &str = "claude";

/// Claude Code's hook output limit: what a hook puts into a session at once has at most this many
/// characters.
pub const LIMIT: usize = 10_000;

/// The word the signal and the line after a cut message name.
pub const INBOX_WORD: &str = "inbox";

/// The argument of the `inbox` tool that names one message.
pub const MESSAGE_ARG: &str = "message";

/// Bob's agent in his `~/work/clientA/review`.
pub const REVIEW: &str = "review";

/// Hook inputs recorded from Claude Code 2.1.322, with the paths of their temporary directories.
const POST_TOOL_USE: &str = include_str!("inputs/post-tool-use.json");
const STOP: &str = include_str!("inputs/stop.json");
/// The `Stop` input in a turn a `Stop` hook kept going: `stop_hook_active` is true.
const STOP_ACTIVE: &str = include_str!("inputs/stop-active.json");

/// A hook event, as Claude Code runs the handler on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    PostToolUse,
    /// `Stop`, the first in its turn.
    Stop,
    /// `Stop` in a turn a `Stop` hook kept going.
    StopActive,
}

impl Event {
    /// The event's `hook_event_name`.
    pub fn name(self) -> &'static str {
        match self {
            Self::PostToolUse => "PostToolUse",
            Self::Stop | Self::StopActive => "Stop",
        }
    }

    /// The recorded input of the event, its `cwd` and `transcript_path` in `dir`.
    pub fn input(self, dir: &Path) -> String {
        let recorded = match self {
            Self::PostToolUse => POST_TOOL_USE,
            Self::Stop => STOP,
            Self::StopActive => STOP_ACTIVE,
        };
        let mut input: Value = serde_json::from_str(recorded).expect("a recorded input is JSON");
        input["cwd"] = json!(dir.to_string_lossy());
        let session = input["session_id"]
            .as_str()
            .expect("a session ID")
            .to_owned();
        input["transcript_path"] = json!(dir.join(format!("{session}.jsonl")).to_string_lossy());
        input.to_string()
    }
}

/// What a run of the hook handler gave.
#[derive(Debug)]
pub struct Hooked {
    /// Its exit code, if it exited with one.
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl Hooked {
    /// Fails unless the run exited with success and printed nothing, on neither output.
    #[track_caller]
    pub fn assert_silent(&self, what: &str) {
        assert_eq!(self.code, Some(0), "{what}: {self:?}");
        assert!(
            self.stdout.is_empty() && self.stderr.is_empty(),
            "{what}: printed {self:?}"
        );
    }

    /// The text the run gives the session on `event`: its output is one JSON object whose
    /// `hookSpecificOutput` names `event` and holds `additionalContext`. Fails unless the run
    /// exited with success, gave such an object, and the text has at most [`LIMIT`] characters.
    #[track_caller]
    pub fn context(&self, event: Event) -> String {
        assert_eq!(self.code, Some(0), "the hook failed: {self:?}");
        let output: Value = serde_json::from_str(self.stdout.trim())
            .unwrap_or_else(|err| panic!("not one JSON object ({err}): {self:?}"));
        let specific = &output["hookSpecificOutput"];
        assert_eq!(
            specific["hookEventName"].as_str(),
            Some(event.name()),
            "{output}"
        );
        let text = specific["additionalContext"]
            .as_str()
            .unwrap_or_else(|| panic!("no additionalContext: {output}"))
            .to_owned();
        assert!(
            text.chars().count() <= LIMIT,
            "{} characters, over the limit",
            text.chars().count()
        );
        text
    }
}

/// Runs `command`, a run of the hook handler, with `input` on its standard input, for up to
/// [`RUN_LIMIT`].
#[track_caller]
pub fn run_hook(command: &mut Command, input: &str) -> Hooked {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run skald hook");
    let mut stdin = child.stdin.take().expect("stdin is piped");
    // A handler that fails before it reads its input closes it: not the test's failure.
    let _ = stdin.write_all(input.as_bytes());
    drop(stdin);
    let (done, finished) = mpsc::channel();
    thread::spawn(move || {
        let _ = done.send(child.wait_with_output());
    });
    let output = finished
        .recv_timeout(RUN_LIMIT)
        .unwrap_or_else(|_| panic!("the hook ran for more than {RUN_LIMIT:?}"))
        .expect("wait for skald hook");
    Hooked {
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// The hook handler's command on `machine` in `dir`, as the agent `agent` if any.
pub fn hook_command(machine: &Machine, dir: &Path, agent: Option<&str>) -> Command {
    let mut args: Vec<&str> = agent.map(|agent| vec![AGENT, agent]).unwrap_or_default();
    args.extend([HOOK, CLAUDE_CODE]);
    machine.command_in(dir, &args)
}

/// Runs the hook handler on `machine` in `dir`, as the agent `agent` if any, on `event`'s
/// recorded input.
#[track_caller]
pub fn hook_in(machine: &Machine, dir: &Path, agent: Option<&str>, event: Event) -> Hooked {
    run_hook(&mut hook_command(machine, dir, agent), &event.input(dir))
}

/// Ada's machine alone in `clienta`, with web's MCP session running in `~/work/clientA/web`.
pub struct Web {
    /// `skald --agent web mcp` in web's folder. Ends before `solo` removes the machine's
    /// directory, which Windows does not remove while the session holds files in it.
    pub session: Session,
    pub solo: Solo,
}

impl Web {
    /// Ada, GitHub user `id` with `login`.
    pub fn new(id: u64, login: &str) -> Self {
        let solo = Solo::new(id, login);
        let session = solo.session(WEB);
        Web { solo, session }
    }

    pub fn machine(&self) -> &Machine {
        self.solo.machine()
    }

    /// web's folder, `~/work/clientA/web`.
    pub fn dir(&self) -> PathBuf {
        self.solo.dir(WEB)
    }

    /// Runs web's hook handler, `--agent web`, in its folder on `event`.
    #[track_caller]
    pub fn hook(&self, event: Event) -> Hooked {
        hook_in(self.machine(), &self.dir(), Some(WEB), event)
    }

    /// api sends web `text` with `skald --agent api send`; gives its `msg_id`.
    #[track_caller]
    pub fn own(&self, text: &str) -> MessageId {
        api_sends_web(&self.solo, text)
    }

    /// api's MCP session sends `*`, everyone in the channel, `text` with the send tool; gives its
    /// `msg_id`.
    #[track_caller]
    pub fn api_tells_everyone(&self, text: &str) -> MessageId {
        let mut api = self.solo.session(API);
        self.solo.sends(&mut api, EVERYONE, text)
    }

    /// api as `--to` and `to` take it.
    pub fn api(&self) -> String {
        self.solo.short(API)
    }

    /// Bob, GitHub user `id` with `login`, joins `clienta` by a fixture revision.
    pub fn with_bob(&self, id: u64, login: &str) -> Bob {
        Bob::join(&self.solo, id, login)
    }

    /// Bob's agent review sends web `text` from his machine; gives its `msg_id`.
    #[track_caller]
    pub fn peer(&self, bob: &Bob, text: &str) -> MessageId {
        bob.sends(&self.solo, WEB, text)
    }

    /// web's `inbox` with nothing waiting: fails unless it shows no message.
    #[track_caller]
    pub fn assert_inbox_empty(&mut self, what: &str) {
        let shown = self.session.inbox();
        assert!(agent_lines(&shown).is_empty(), "{what}: {shown}");
    }
}

/// Bob in `clienta`, and the folder of his agent review.
pub struct Bob {
    pub person: Person,
    pub dir: PathBuf,
}

impl Bob {
    /// Bob, GitHub user `id` with `login`, joins `solo`'s `clienta` by a fixture revision.
    pub fn join(solo: &Solo, id: u64, login: &str) -> Self {
        let (person, _) = solo.with_bob(id, login);
        let dir = dir_of(&person.machine, "work/clientA/review");
        Bob { person, dir }
    }

    /// review as `--to` and `to` take it.
    pub fn review(&self) -> String {
        format!("{}/{REVIEW}", self.person.handle)
    }

    /// Sends Ada's agent `agent` `text` with `skald --agent review send` on Bob's machine.
    /// Gives the ID of the one user message the front saw and the command reported.
    #[track_caller]
    pub fn sends(&self, solo: &Solo, agent: &str, text: &str) -> MessageId {
        let mark = solo.mark();
        let to = solo.short(agent);
        let args = [AGENT, REVIEW, SEND, TO, &to, text];
        let sent = self.person.machine.start_in(&self.dir, &args).finish();
        assert_user_send(solo, solo.default_channel(), mark, &sent)
    }
}

/// Sends `text` to the agent `to` with `skald --agent api send` in api's folder.
/// Gives the ID of the one user message the front saw and the command reported.
#[track_caller]
pub fn api_sends(solo: &Solo, to: &str, text: &str) -> MessageId {
    let mark = solo.mark();
    let args = [AGENT, API, SEND, TO, to, text];
    let sent = solo.machine().start_in(&solo.dir(API), &args).finish();
    assert_user_send(solo, solo.default_channel(), mark, &sent)
}

/// api sends web `text` in the default channel, as [`api_sends`] does.
#[track_caller]
pub fn api_sends_web(solo: &Solo, text: &str) -> MessageId {
    api_sends(solo, &solo.short(WEB), text)
}

/// The ID the send printed, checked against the one reportable message the front saw in the
/// channel after `mark`. The receiving session may send a receipt while the command runs.
#[track_caller]
pub fn assert_user_send(
    solo: &Solo,
    channel: ChannelId,
    mark: usize,
    sent: &Finished,
) -> MessageId {
    assert!(sent.success, "{sent:?}");
    let ids: Vec<_> = messages_sent(&solo.talk.front, channel, mark)
        .into_iter()
        .filter(|send| send.report)
        .map(|send| send.envelope.envelope().header.msg_id)
        .collect();
    let [msg_id] = ids.as_slice() else {
        panic!("not one reported message in {channel}: {ids:?}");
    };
    assert_says_sent(&sent.stdout, *msg_id);
    *msg_id
}

/// Fails unless `context` shows `text` once, as one line of the message `msg_id` from api, Ada's
/// machine, at the level `own`; gives that line.
#[track_caller]
pub fn assert_own_line(context: &str, text: &str, msg_id: MessageId, api: &str) -> AgentLine {
    let got = assert_handed_line(context, text, msg_id);
    assert_eq!(
        (got.from.as_str(), got.device.as_str(), got.level.as_str()),
        (api, crate::talk::ADA_HOST, crate::senders::OWN),
        "{context}"
    );
    got
}

/// Fails if `text` holds `long` whole, or unless its last line names [`INBOX_WORD`] and `#<ref>` of
/// `msg_id`: how to read the message whole.
#[track_caller]
pub fn assert_cut(text: &str, long: &str, msg_id: MessageId) {
    assert!(!text.contains(long), "the whole text went in");
    let last = text.lines().rev().find(|line| !line.trim().is_empty());
    assert!(
        last.is_some_and(|last| last.contains(INBOX_WORD) && names_reference(last, msg_id)),
        "the last line names no {INBOX_WORD} and #<ref>: {last:?}"
    );
}

/// Whether a word of `line` is `#<ref>` of `msg_id`.
fn names_reference(line: &str, msg_id: MessageId) -> bool {
    (line.split(|c: char| c != '#' && !c.is_ascii_alphanumeric()))
        .filter_map(|word| word.strip_prefix('#'))
        .any(|reference| refers_to(reference, msg_id))
}

/// Fails unless `context` is the signal alone and holds nothing of `msg_id` or its sender: no
/// message line, no warning line, a line that names [`INBOX_WORD`], and neither `text`, the
/// sender's handle or device, nor the message's reference.
#[track_caller]
pub fn assert_signal(context: &str, msg_id: MessageId, text: &str, bob: &Bob) {
    assert!(context.contains(INBOX_WORD), "no {INBOX_WORD}: {context}");
    assert!(agent_lines(context).is_empty(), "a message line: {context}");
    assert!(
        data_warnings(context).is_empty(),
        "a warning line: {context}"
    );
    assert!(!context.contains(text), "the text: {context}");
    let lower = context.to_lowercase();
    let handle = bob.person.handle.to_string().to_lowercase();
    assert!(!lower.contains(&handle), "the sender: {context}");
    assert!(
        !lower.contains(&bob.person.host.to_lowercase()),
        "the sender's device: {context}"
    );
    let words = context.split(|c: char| !c.is_ascii_alphanumeric());
    assert!(
        !words.into_iter().any(|word| refers_to(word, msg_id)),
        "the reference: {context}"
    );
}

/// The message lines of `context` from a level other than `own`.
pub fn others(context: &str) -> usize {
    (context.lines().filter_map(agent_line))
        .filter(|line| line.level != crate::senders::OWN)
        .count()
}
