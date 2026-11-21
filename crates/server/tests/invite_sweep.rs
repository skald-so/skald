//! The server deletes an invite once it has ended at its expiry (`docs/protocol/membership.md`,
//! "Invite"), with no request for it. Against a `skald-server` as in the tests of storing sent
//! envelopes, whose fixture these tests share; the end of an issued invite ID is moved back in the
//! database.

use std::thread;
use std::time::{Duration, Instant};

#[path = "common/mod.rs"]
mod common;

// The spaces tests' accounts and revisions, and the fixture and frames of the tests of storing sent
// envelopes; this binary uses some of them.
#[allow(dead_code)]
#[path = "spaces/api.rs"]
mod api;
#[allow(dead_code)]
#[path = "mailbox/fixture.rs"]
mod fixture;
#[allow(dead_code)]
#[path = "spaces/rosters.rs"]
mod rosters;
#[allow(dead_code)]
#[path = "mailbox/wire.rs"]
mod wire;

use common::TestDb;
use common::device_flow::{json, text};
use fixture::Fixture;

/// Whether the database holds the invite ID `id`.
fn held(db: &TestDb, id: &str) -> bool {
    let id = id.to_owned();
    db.query(async |conn| {
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM invites WHERE id::text = $1)")
            .bind(id)
            .fetch_one(conn)
            .await
    })
    .expect("read the invites")
}

/// Of two invite IDs alice's session gets, the one whose end the database moved back is deleted
/// within a few sweeps; the other stays.
#[test]
fn an_invite_past_its_end_is_deleted() {
    let f = Fixture::new(3_817_600_001);
    let path = format!("/spaces/{}/invites", f.team.space);
    let issue = || {
        let response = f.team.alice.session.call("post", &path, None);
        assert_eq!(response.status, 201, "{}", text(&response));
        json(&response)["invite"]
            .as_str()
            .expect("an ID")
            .to_owned()
    };
    let (ended, open) = (issue(), issue());
    f.db().execute(&format!(
        "UPDATE invites SET expires_at = now() - interval '1 second' WHERE id = '{ended}'"
    ));

    let deadline = Instant::now() + Duration::from_secs(30);
    while held(f.db(), &ended) {
        assert!(Instant::now() < deadline, "the ended invite is still held");
        thread::sleep(Duration::from_millis(200));
    }
    assert!(held(f.db(), &open), "the open invite");
}
