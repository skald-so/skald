//! What the tests of the lock share: runs on Ada's machine with a timeout of their own, one that
//! holds the lock while the front holds back its `auth`, and what the runs that wait for it do.

use std::thread;
use std::time::{Duration, Instant};

use skald_proto::MessageId;

use crate::fixtures::{SEND, await_running};
use crate::machine::{Finished, Run};
use crate::talk::{Talk, messages_sent};

/// The flag of a run's request timeout, in seconds.
pub const TIMEOUT_FLAG: &str = "--timeout";
/// The request timeout of a run that outlasts every wait of a test, in seconds.
pub const LONG: u64 = 60;
/// The request timeout of a run that gives up on the lock, in seconds.
pub const SHORT: u64 = 2;
/// How long a test watches a run wait for the lock.
pub const WATCHED: Duration = Duration::from_secs(4);
/// What a run that waited for the lock past its timeout names.
pub const LOCK: &str = "lock";

/// Starts `skald --timeout <seconds> <args>` on Ada's machine.
pub fn start(talk: &Talk, seconds: u64, args: &[&str]) -> Run {
    let seconds = seconds.to_string();
    let mut all = vec![TIMEOUT_FLAG, seconds.as_str()];
    all.extend_from_slice(args);
    talk.ada.machine.start(&all)
}

/// The arguments of `skald send` of `text` to the talk's channel.
pub fn send_of(talk: &Talk, text: &str) -> [String; 2] {
    [talk.channel.to_string(), text.to_owned()]
}

/// Starts `skald send` of `text` to the talk's channel on Ada's machine with a timeout of
/// `seconds`.
pub fn start_send(talk: &Talk, seconds: u64, text: &str) -> Run {
    let [channel, text] = send_of(talk, text);
    start(
        talk,
        seconds,
        &[SEND, "--all", "--channel", &channel, &text],
    )
}

/// From now on, the front holds back the `auth` of Ada's device. Starts `args` with a timeout of
/// `seconds` and waits until the front holds back its `auth`: from then on, the run holds the lock
/// while it waits for the server. Gives the run and how long it took to get there.
pub fn holding(talk: &Talk, seconds: u64, args: &[&str]) -> (Run, Duration) {
    talk.front.hold_auth_of(Some(talk.ada.device));
    let held = talk.front.held_auths();
    let started = Instant::now();
    let mut run = start(talk, seconds, args);
    await_running(&mut run, "auth held back", || {
        talk.front.held_auths() > held
    });
    (run, started.elapsed())
}

/// As [`holding`], a `skald send` of `text` to the talk's channel with a timeout of [`LONG`].
pub fn send_holding(talk: &Talk, text: &str) -> Run {
    let [channel, text] = send_of(talk, text);
    holding(talk, LONG, &[SEND, "--all", "--channel", &channel, &text]).0
}

/// Fails unless `waiting` keeps running for [`WATCHED`] without reaching the server as its
/// device: the front gets no other `auth` of Ada's device meanwhile.
#[track_caller]
pub fn assert_waits(talk: &Talk, waiting: &mut Run) {
    let held = talk.front.held_auths();
    let until = Instant::now() + WATCHED;
    while Instant::now() < until {
        waiting.assert_running();
        assert_eq!(
            talk.front.held_auths(),
            held,
            "a run waiting for the lock reached the server: {}",
            waiting.shown()
        );
        thread::sleep(Duration::from_millis(50));
    }
}

/// The front forwards the `auth` of Ada's device from now on; gives how `runs` finished, in
/// order. Fails unless each succeeds.
#[track_caller]
pub fn release(talk: &Talk, runs: Vec<Run>) -> Vec<Finished> {
    talk.front.hold_auth_of(None);
    runs.into_iter()
        .map(|run| {
            let finished = run.finish();
            assert!(finished.success, "{finished:?}");
            finished
        })
        .collect()
}

/// Fails unless `finished` failed with a message that names the lock.
#[track_caller]
pub fn assert_failed_on_the_lock(finished: &Finished) {
    assert!(
        !finished.success,
        "it did not wait for the lock: {finished:?}"
    );
    assert!(
        finished.shown().to_lowercase().contains(LOCK),
        "no message naming the lock: {finished:?}"
    );
}

/// The `msg_id` of each message envelope in the talk's channel the front saw sent after its first
/// `from` sends, in the order each was first sent.
pub fn sent_ids(talk: &Talk, from: usize) -> Vec<MessageId> {
    messages_sent(&talk.front, talk.channel, from)
        .iter()
        .map(|sent| sent.envelope.envelope().header.msg_id)
        .collect()
}
