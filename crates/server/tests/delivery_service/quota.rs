//! A channel's `mls` envelopes and the space's monthly limit (`docs/dev/architecture.md`, "Cloud
//! limits"): an application message counts, while a handshake message, a Commit or a Welcome, is no
//! message of the space's and does not.

use crate::common::TestDb;
use crate::scene::{Scene, empty};
use crate::wire::query;

/// The number of messages the server counted.
fn counted(db: &TestDb) -> i64 {
    query(db, async |conn| {
        sqlx::query_scalar("SELECT count(*) FROM counted_messages")
            .fetch_one(conn)
            .await
    })
}

#[test]
fn a_handshake_message_does_not_count_towards_the_monthly_limit() {
    let (mut s, _) = Scene::formed_with(3_121_470_601, &["--monthly-messages-limit", "5"]);
    let f = &s.fixture;
    s.bob.commits(f, empty);
    assert_eq!(counted(f.db()), 0, "Commits and a Welcome");

    let message = (s.alice.mls.encrypt(f.general, b"counted")).expect("a message");
    s.alice.stores(f, &message, &[]);
    assert_eq!(counted(f.db()), 1, "an application message");
}
