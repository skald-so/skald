//! What the tests of `skald recv --wait` share: Ada's machine with `~/work/acme` bound to `acme`,
//! its runs through a helper, and how a test watches a run that waits.

use std::thread;
use std::time::{Duration, Instant};

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
/// The setting of how long a connection on SSE or long-poll waits before it tries WebSocket again,
/// in seconds.
pub const RETRY_SETTING: &str = "SKALD_WEBSOCKET_RETRY";

/// How soon a waiting session shows a message once the server has stored it.
pub const ARRIVAL: Duration = Duration::from_secs(1);
/// How soon a run whose messages are already waiting exits, whatever the load on the machine.
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

/// Fails unless `run` keeps running for [`IDLE_WATCH`].
#[track_caller]
pub fn assert_keeps_waiting(run: &mut Run) {
    let until = Instant::now() + IDLE_WATCH;
    while Instant::now() < until {
        run.assert_running();
        thread::sleep(Duration::from_millis(50));
    }
}

/// Waits for up to `limit` from `from` until `run` has shown `text`; fails if it has not by then.
#[track_caller]
pub fn assert_shown_within(run: &Run, text: &str, (from, limit): (Instant, Duration)) {
    while !run.shown().contains(text) {
        assert!(
            from.elapsed() <= limit,
            "{text:?} not shown within {limit:?}: {}",
            run.shown()
        );
        thread::sleep(Duration::from_millis(10));
    }
}

/// Waits while `run` goes on until the server has seen `count` WebSocket connections from `since`
/// on; fails if `run` exits first.
#[track_caller]
pub fn await_connections(talk: &Talk, run: &mut Run, (since, count): (Instant, usize)) {
    await_running(run, "connection to the server", || {
        talk.cloud().count_sent(since, WEBSOCKET) >= count
    });
}

/// Fails unless `finished` succeeded and shows `text` once, with `msg_id`.
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
