//! What the tests of `skald recv --wait` share: Ada's machine with `~/work/acme` bound to `acme`,
//! its runs through a helper, and how a test watches a run that waits.

use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use skald_proto::MessageId;

use crate::agents::{Agents, assert_handed_once};
use crate::cloud::WAIT;
use crate::fixtures::await_running;
use crate::helped::{Helped, WEBSOCKET};
use crate::kit::RECV;
use crate::machine::{Finished, Run};
use crate::talk::Talk;
use crate::world::AGENT;

/// The flag of `skald recv` that waits for what is to come.
pub const WAIT_FLAG: &str = "--wait";
/// The setting that has a session say when each step of the helper's intake ran.
pub const TRACE_SETTING: &str = "SKALD_WAIT_TRACE";
/// How each line of the trace starts.
pub const TRACED: &str = "skald: trace: ";
/// The setting of how long a connection on SSE or long-poll waits before it tries WebSocket again,
/// in seconds.
pub const RETRY_SETTING: &str = "SKALD_WEBSOCKET_RETRY";

/// How soon a waiting session shows a message once the server has stored it.
pub const ARRIVAL: Duration = Duration::from_secs(1);
/// How soon a run whose messages are already waiting shows them, from its start, whatever the load
/// on the machine: well under a long-poll's wait of 30 s.
pub const SHOWN_WITHIN: Duration = Duration::from_secs(15);
/// How soon a run whose messages are already waiting exits once it has shown them, whatever the
/// load on the machine.
pub const AT_ONCE: Duration = Duration::from_secs(5);
/// How long a test watches a run wait while nothing comes for it.
pub const IDLE_WATCH: Duration = Duration::from_secs(2);

/// The talk of the tests of handing messages over, with Ada's agents in `~/work/acme`, and the runs
/// of the machine's own copy of `skald`, which may go through a helper.
pub struct Waits {
    /// Dropped first: it kills the processes of the copy before the machine's directory goes.
    pub helped: Helped,
    pub agents: Agents,
}

impl Waits {
    /// Ada, GitHub user `ids[0]` with `logins[0]`, and Bob, the next ones, in `acme`'s channel;
    /// `~/work/acme` bound to `acme`.
    pub fn new(ids: [u64; 2], logins: [&str; 2]) -> Self {
        let agents = Agents::new(ids, logins);
        Waits {
            helped: Helped::new(&agents.talk.ada.machine),
            agents,
        }
    }

    /// Starts `skald --agent <agent> recv --wait` in `~/work/acme`, with `settings`, through a
    /// helper.
    pub fn session_with(&self, settings: &[(&str, &str)], agent: &str) -> Run {
        let args = [AGENT, agent, RECV, WAIT_FLAG];
        self.helped.start_in(&self.agents.folder, settings, &args)
    }

    /// As [`Waits::session_with`], without settings.
    pub fn session(&self, agent: &str) -> Run {
        self.session_with(&[], agent)
    }

    /// Runs `skald --agent <agent> recv` in `~/work/acme` through a helper; fails unless it
    /// succeeds.
    #[track_caller]
    pub fn helped_as(&self, agent: &str) -> Finished {
        let finished = (self.helped).run_in(&self.agents.folder, &[AGENT, agent, RECV]);
        assert!(finished.success, "{agent}: {finished:?}");
        finished
    }
}

/// How long the server sees nothing of Ada's machine before a test takes its runs as settled.
pub const QUIET: Duration = Duration::from_secs(1);

/// Waits for up to [`WAIT`] while `runs` go on until the server has seen no request for
/// [`QUIET`]; then fails unless they keep waiting for [`IDLE_WATCH`] each, the server seeing no
/// request meanwhile: while nothing comes, a run that waits reaches the server for nothing.
#[track_caller]
pub fn assert_idle(talk: &Talk, runs: &mut [&mut Run]) {
    let relay = &talk.cloud().relay;
    let heads = |from| -> Vec<String> {
        (relay.sent_since(from).iter())
            .filter_map(|sent| sent.head.lines().next().map(str::to_owned))
            .collect()
    };
    let deadline = Instant::now() + WAIT;
    loop {
        let from = relay.mark();
        thread::sleep(QUIET);
        for run in runs.iter_mut() {
            run.assert_running();
        }
        let sent = heads(from);
        if sent.is_empty() {
            break;
        }
        assert!(Instant::now() < deadline, "never settled: {sent:?}");
    }
    let from = relay.mark();
    for run in runs.iter_mut() {
        assert_keeps_waiting(run);
    }
    let sent = heads(from);
    assert!(sent.is_empty(), "requests while nothing came: {sent:?}");
}

/// Fails unless `run` keeps running for [`IDLE_WATCH`].
#[track_caller]
pub fn assert_keeps_waiting(run: &mut Run) {
    let until = Instant::now() + IDLE_WATCH;
    while Instant::now() < until {
        run.assert_running();
        thread::sleep(Duration::from_millis(50));
    }
}

/// Fails unless `run` shows `text` within `limit` from `from`. It waits for up to [`WAIT`], so a
/// failure says how long the text took.
#[track_caller]
pub fn assert_shown_within(run: &Run, text: &str, (from, limit): (Instant, Duration)) {
    let since = SystemTime::now() - from.elapsed();
    let since = since
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    while !run.shown().contains(text) && from.elapsed() <= WAIT {
        thread::sleep(Duration::from_millis(10));
    }
    let took = from.elapsed();
    assert!(
        run.shown().contains(text),
        "{text:?} not shown within {WAIT:?}: {}",
        run.shown()
    );
    assert!(
        took <= limit,
        "{text:?} shown after {} ms from {since}, over {} ms: {}",
        took.as_millis(),
        limit.as_millis(),
        run.shown()
    );
}

/// Waits while `run` goes on until the server has seen `count` WebSocket connections from `since`
/// on; fails if `run` exits first.
#[track_caller]
pub fn await_connections(talk: &Talk, run: &mut Run, (since, count): (Instant, usize)) {
    await_running(run, "connection to the server", || {
        talk.cloud().count_sent(since, WEBSOCKET) >= count
    });
}

/// Fails unless `finished` succeeded and shows `text` once, by the reference of `msg_id`.
#[track_caller]
pub fn assert_got(finished: &Finished, text: &str, msg_id: MessageId) {
    assert!(finished.success, "{finished:?}");
    assert_handed_once(&finished.shown(), text, msg_id);
}

/// The processes of the helped copy running now but `runs`: its helpers.
pub fn helpers_beside(helped: &Helped, runs: &[&Run]) -> Vec<u32> {
    let runs: Vec<u32> = runs.iter().map(|run| run.id()).collect();
    (helped.helpers().into_iter())
        .filter(|pid| !runs.contains(pid))
        .collect()
}

/// Waits for up to [`WAIT`] until one helper runs beside `runs`, the copy's other processes; gives
/// it.
#[track_caller]
pub fn await_helper_beside(helped: &Helped, runs: &[&Run]) -> u32 {
    let deadline = Instant::now() + WAIT;
    loop {
        if let [one] = helpers_beside(helped, runs).as_slice() {
            return *one;
        }
        assert!(Instant::now() < deadline, "no helper started");
        thread::sleep(Duration::from_millis(50));
    }
}
