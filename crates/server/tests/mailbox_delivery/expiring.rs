//! What the expiry tests share: the server's mailbox retention, time passing as the database shows
//! it, and a server started again on the same database.

use std::thread;
use std::time::Duration;

use skald_proto::Stored;

use crate::common::sign_in::{CLIENT_SECRET, SECRET_ENV, github_settings};
use crate::common::{Server, TestDb};
use crate::link::{Link, Sent, seqs};
use crate::scene::Scene;
use crate::wire::{Answer, Socket, assert_nowhere, envelope, query};

/// The server setting of the mailbox retention, in seconds; unset, there is none.
pub const RETENTION: &str = "--mailbox-retention";
/// The environment variable of the same setting.
pub const RETENTION_ENV: &str = "SKALD_MAILBOX_RETENTION";
/// The server setting of the protocol versions it accepts.
pub const VERSIONS: &str = "--protocol-versions";

pub const HOUR: u64 = 3_600;
pub const DAY: u64 = 24 * HOUR;
/// The retention the tests set.
pub const WEEK: u64 = 7 * DAY;
pub const YEAR: u64 = 365 * DAY;

/// How soon the server finds an expired entry by itself, with no connection of its device.
pub const EXPIRY: Duration = Duration::from_secs(30);

/// The tables whose times the tests move back.
const TIMED: [&str; 3] = ["envelopes", "sends", "entries"];

/// The scene on a server with a mailbox retention of [`WEEK`]; GitHub user IDs from `github` on.
pub fn retained(github: u64) -> Scene {
    Scene::with(github, &[RETENTION, &WEEK.to_string()])
}

/// Moves every time the server keeps of envelopes, sends and mailbox entries `seconds` back, as if
/// that much time had passed: each `timestamptz` column of the tables `envelopes`, `sends` and
/// `entries`.
pub fn move_back(db: &TestDb, seconds: u64) {
    let tables: Vec<String> = TIMED.iter().map(|&table| table.to_owned()).collect();
    let columns: Vec<(String, String)> = query(db, async |conn| {
        sqlx::query_as(
            "SELECT table_name::text, column_name::text FROM information_schema.columns
             WHERE table_schema = current_schema() AND table_name = ANY ($1)
               AND data_type = 'timestamp with time zone'",
        )
        .bind(tables)
        .fetch_all(conn)
        .await
    });
    assert!(!columns.is_empty(), "no times in {TIMED:?}");
    for (table, column) in columns {
        db.execute(&format!(
            r#"UPDATE "{table}" SET "{column}" = "{column}" - make_interval(secs => {seconds})"#
        ));
    }
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
    let stand = &mut s.fixture.team.flow.stand;
    stand.server.stop();
    let mut all = github_settings(&stand.public, &stand.github);
    all.extend_from_slice(args);
    let mut all_envs = vec![(SECRET_ENV, CLIENT_SECRET)];
    all_envs.extend_from_slice(envs);
    Server::launch(&stand.db, "127.0.0.1:0", &all, &all_envs)
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
