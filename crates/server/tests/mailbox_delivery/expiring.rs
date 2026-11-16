//! What the expiry tests share besides the retention and time passing of `common/times.rs`: a
//! server started again on the same database, and sends and retries of an envelope.

use std::thread;
use std::time::Duration;

use skald_proto::Stored;

use crate::common::Server;
use crate::common::times::{RETENTION, WEEK};
use crate::link::{Link, Sent, seqs};
use crate::scene::Scene;
use crate::wire::{Answer, Socket, assert_nowhere, envelope};

/// The server setting of the protocol versions it accepts.
pub const VERSIONS: &str = "--protocol-versions";

/// The scene on a server with a mailbox retention of [`WEEK`]; GitHub user IDs from `github` on.
pub fn retained(github: u64) -> Scene {
    Scene::with(github, &[RETENTION, &WEEK.to_string()])
}

/// Stores on alice's connection an envelope to bob with `ttl` and `report`; gives it and its
/// `stored`.
#[track_caller]
pub fn send_bob_with(s: &mut Scene, ttl: Option<u64>, report: bool) -> (Sent, Stored) {
    let mut header = s.fixture.to(s.bob);
    header.ttl = ttl;
    let sent = envelope(&header);
    let stored = s.alice.store(&header, &sent, report);
    ((header, sent), stored)
}

/// Stops the scene's server and starts another on its database and public address, with the extra
/// arguments `args` and the environment variables `envs`. The scene's connections end with the
/// first server; new ones go to the one this gives.
pub fn restart(s: &mut Scene, args: &[&str], envs: &[(&str, &str)]) -> Server {
    s.fixture.team.flow.stand.restart(args, envs)
}

/// Checks that a retry of `sent`, which bob's connection `link` has acked, gets the `stored` of its
/// first send, `first`, and stores nothing.
#[track_caller]
pub fn assert_retry_stores_nothing(s: &Scene, sent: &Sent, first: &Stored, link: &mut Link) {
    // Past a second, so that a `stored` made anew would have a later timestamp.
    thread::sleep(Duration::from_millis(1_100));
    let t = s.team();
    let mut again = Socket::of(&t.flow, &t.alice);
    again.send(&sent.1);
    let answer = again.answer();
    let Answer::Stored(stored) = answer else {
        panic!("stored, not {answer:?}");
    };
    assert_eq!(stored.send_key, first.send_key);
    // The time of the first send, as the database holds it, moved back with the rest: not a new one.
    assert!(
        stored.timestamp <= first.timestamp,
        "a stored made anew at {}, not the first one of {}",
        stored.timestamp,
        first.timestamp
    );
    link.assert_quiet();
    assert_eq!(seqs(s.db(), s.bob), Vec::<i64>::new());
    assert_nowhere(s.db(), &sent.1);
}
