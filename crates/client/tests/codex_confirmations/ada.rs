//! What the tests share: Ada's machine, as the tests of Codex's hooks have it, whose agents' Codex
//! sessions run in one Codex process, the stand-in `codex` of an app of its own; each session with
//! its config home, its rollout and its `skald mcp`, started as Codex starts it; the resend timeout
//! that the session's hooks, the helper they start and its `skald mcp` take.

use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use skald_proto::MessageId;

use crate::client::{MCP, Session};
use crate::codex::{
    Codex, CodexProcess, Event, Thread, codex_client, given, gives_by, hook_command,
};
use crate::helped::HELPER;
use crate::hooks::Hooked;
use crate::kit::received;
use crate::lines::{agent_lines, refers_to};
use crate::machine::{Machine, Run};
use crate::receipts::read_lines;
use crate::rollout::{Call, Rollout};
use crate::standin::{StandIn, session_of};
use crate::{
    devices::second_machine,
    fixtures::{SEND, TO},
    hooks::REVIEW,
    senders::LAPTOP,
    solo::WEB,
    talk::one_message_sent,
    world::{ADD_RULE, AGENT, CLIENT_A, Person, dir_of, path, run},
};

/// The setting of how long a message a path gave waits for its confirmation, in seconds, before it
/// goes by the next path.
pub const RESEND_SETTING: &str = "SKALD_RESEND_AFTER";

/// A resend timeout, in seconds, longer than any test runs: nothing goes again meanwhile.
pub const NEVER: u64 = 600;

/// How long a test watches for what must not happen.
pub const QUIET: Duration = Duration::from_secs(4);

/// How long a helper that starts must run on its own before a test counts on it.
const STEADY: Duration = Duration::from_secs(2);

/// Ada's machine and the Codex process that runs her agents' sessions.
pub struct Ada {
    /// Ends before the machine's directory goes.
    process: CodexProcess,
    /// The `codex` of the process.
    pub app: StandIn,
    /// The resend timeout, in seconds.
    resend: String,
    pub codex: Codex,
}

/// An agent's Codex session.
pub struct Agent {
    /// Its `skald mcp`, started as Codex starts it. Ends before the machine's directory goes.
    pub mcp: Session,
    pub thread: Thread,
    pub rollout: Rollout,
}

impl Ada {
    /// Ada, GitHub user `id` with `login`, alone in `clienta`, with the resend timeout `resend`, in
    /// seconds.
    pub fn new(id: u64, login: &str, resend: u64) -> Self {
        let codex = Codex::new(id, login);
        let app = StandIn::app(&codex, "cli");
        let process = CodexProcess::start_as(app.exe());
        Ada {
            process,
            app,
            resend: resend.to_string(),
            codex,
        }
    }

    pub fn machine(&self) -> &Machine {
        self.codex.machine()
    }

    fn settings(&self) -> [(&str, &str); 1] {
        [(RESEND_SETTING, self.resend.as_str())]
    }

    /// `agent`'s session starts in the Codex process, `--agent <agent>`, with a config home of its
    /// own and the rollout Codex begins: its `SessionStart` must print nothing. Its `skald mcp`
    /// starts. The session is idle.
    #[track_caller]
    pub fn start(&mut self, agent: &str) -> Agent {
        let thread = session_of(&self.codex, agent, agent);
        let rollout = Rollout::begin(&thread);
        self.started(agent, thread, rollout)
    }

    /// As [`Ada::start`], with a rollout Skald cannot parse.
    #[track_caller]
    pub fn start_unparsable(&mut self, agent: &str) -> Agent {
        let thread = session_of(&self.codex, agent, agent);
        let rollout = Rollout::unparsable(&thread);
        self.started(agent, thread, rollout)
    }

    #[track_caller]
    fn started(&mut self, agent: &str, thread: Thread, rollout: Rollout) -> Agent {
        (self.run_hook(&thread, Event::SessionStart)).assert_silent("SessionStart");
        let dir = self.codex.web.solo.dir(agent);
        let mut command = self.machine().command_in(&dir, &[AGENT, agent, MCP]);
        command.env(RESEND_SETTING, &self.resend);
        let mcp = Session::start(&mut command, &codex_client());
        Agent {
            mcp,
            thread,
            rollout,
        }
    }

    fn run_hook(&mut self, of: &Thread, event: Event) -> Hooked {
        let command = hook_command(&self.codex.helped, of, &self.settings());
        self.process.run(&command, &of.input(event))
    }

    /// `of`'s hook of `event`, run by the Codex process.
    pub fn hook(&mut self, of: &Agent, event: Event) -> Hooked {
        self.run_hook(&of.thread, event)
    }

    /// Fails unless `of`'s hook of `event` prints nothing.
    #[track_caller]
    pub fn silent(&mut self, of: &Agent, event: Event, what: &str) {
        let hooked = self.hook(of, event);
        hooked.assert_silent(&format!("{what}: {}", event.name()));
    }

    /// A turn of `of` starts, so the session is busy: its `UserPromptSubmit`, with nothing new
    /// waiting, must print nothing.
    #[track_caller]
    pub fn busy(&mut self, of: &Agent) {
        self.silent(of, Event::UserPromptSubmit, "the turn's start");
    }

    /// Runs `of`'s hook of `event` every half second until it gives the session something, for up
    /// to [`crate::cloud::WAIT`]; each run before that must print nothing. Gives what it gave.
    #[track_caller]
    pub fn gives(&mut self, of: &Agent, event: Event) -> String {
        gives_by(event, || self.hook(of, event))
    }

    /// Runs `of`'s hook of `event` once; fails unless it gives the session something. Gives it.
    #[track_caller]
    pub fn gives_now(&mut self, of: &Agent, event: Event) -> String {
        let hooked = self.hook(of, event);
        assert!(
            !hooked.stdout.is_empty(),
            "{} gave nothing: {hooked:?}",
            event.name()
        );
        given(&hooked, event)
    }

    /// A helper starts again, with the settings of the session's hooks, and one helper runs on its
    /// own for a while; killed on drop.
    #[track_caller]
    pub fn restart_helper(&self) -> Run {
        let helped = &self.codex.helped;
        let run = helped.start(&self.settings(), &[HELPER]);
        helped.await_one_steady(STEADY);
        run
    }

    /// Pauses the one helper that runs until what it gives drops; fails unless one runs. Only on
    /// Unix: elsewhere the helper runs on.
    #[track_caller]
    pub fn pause_helper(&self) -> Paused {
        let pid = self.codex.helped.one_helper();
        signal(pid, Pause::Stop);
        Paused(pid)
    }

    /// Ada's second machine, linked by hand, joins `clienta`'s channels, its `~/work/clientA`
    /// bound to `clienta`.
    #[track_caller]
    pub fn laptop(&self) -> Laptop {
        let talk = &self.codex.web.solo.talk;
        let person = second_machine(&talk.world, &talk.ada, LAPTOP);
        let folder = dir_of(&person.machine, "work/clientA");
        for machine in [self.machine(), &person.machine] {
            received(machine);
        }
        let bound = run(&person.machine, &[ADD_RULE, path(&folder), CLIENT_A]);
        assert!(bound.success, "the laptop's folder not bound: {bound:?}");
        Laptop { person, folder }
    }

    /// Kills the one helper that runs; fails unless one does.
    #[track_caller]
    pub fn kill_helper(&self) {
        let helped = &self.codex.helped;
        helped.kill(helped.one_helper());
        helped.assert_none();
    }
}

/// A helper paused by [`Ada::pause_helper`]; it runs on once this drops.
pub struct Paused(u32);

impl Drop for Paused {
    fn drop(&mut self) {
        signal(self.0, Pause::Continue);
    }
}

enum Pause {
    Stop,
    Continue,
}

#[cfg(unix)]
fn signal(pid: u32, pause: Pause) {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return;
    };
    let signal = match pause {
        Pause::Stop => libc::SIGSTOP,
        Pause::Continue => libc::SIGCONT,
    };
    // SAFETY: a signal to the helper of the copy, which the test started.
    unsafe {
        libc::kill(pid, signal);
    }
}

#[cfg(not(unix))]
fn signal(_: u32, _: Pause) {}

/// Ada's second machine and its folder bound to `clienta`.
pub struct Laptop {
    pub person: Person,
    folder: PathBuf,
}

impl Laptop {
    pub fn machine(&self) -> &Machine {
        &self.person.machine
    }

    /// Its agent review sends web `text`, at the level `own` from another device; gives its
    /// `msg_id`.
    #[track_caller]
    pub fn sends_web(&self, ada: &Ada, text: &str) -> MessageId {
        let solo = &ada.codex.web.solo;
        let mark = solo.mark();
        let to = solo.short(WEB);
        let args = [AGENT, REVIEW, SEND, TO, &to, text];
        let sent = self.machine().start_in(&self.folder, &args).finish();
        assert!(sent.success, "the laptop's send: {sent:?}");
        one_message_sent(&solo.talk.front, solo.default_channel(), mark)
    }
}

impl Agent {
    /// The model calls `inbox`, as the rollout then shows, and `skald mcp` answers. Gives the call
    /// and the text of its result.
    #[track_caller]
    pub fn inbox(&mut self) -> (Call, String) {
        let call = self.rollout.calls_inbox();
        let shown = self.mcp.inbox();
        (call, shown)
    }

    /// As [`Agent::inbox`], and its result goes to the model, as the rollout then shows. Gives the
    /// text of the result.
    #[track_caller]
    pub fn inbox_in_rollout(&mut self) -> String {
        let (call, shown) = self.inbox();
        self.rollout.mcp_result(&call, &shown);
        shown
    }
}

/// Fails unless `shown`, what `inbox` gave, shows one message: `msg_id`, by its reference, from
/// the start of its text `text`.
#[track_caller]
pub fn assert_gives(shown: &str, text: &str, msg_id: MessageId) {
    let lines = agent_lines(shown);
    let [line] = lines.as_slice() else {
        panic!("not one message: {shown}");
    };
    assert!(refers_to(&line.reference, msg_id), "not {msg_id}: {shown}");
    let head: String = text.chars().take(40).collect();
    assert!(line.text.starts_with(&head), "not {head:?}: {shown}");
}

/// Fails unless `shown`, what `inbox` gave, shows no message.
#[track_caller]
pub fn assert_gives_none(shown: &str, what: &str) {
    assert!(agent_lines(shown).is_empty(), "{what}: {shown}");
}

/// Runs the person's `skald recv` on `machine` every half second for `quiet`; fails if a run shows
/// `read` of `msg_id`.
#[track_caller]
pub fn no_read_for(machine: &Machine, msg_id: MessageId, quiet: Duration, when: &str) {
    let until = Instant::now() + quiet;
    loop {
        let shown = received(machine).shown();
        assert_eq!(read_lines(&shown, msg_id), 0, "{when}: {shown}");
        if Instant::now() >= until {
            return;
        }
        thread::sleep(Duration::from_millis(500));
    }
}
