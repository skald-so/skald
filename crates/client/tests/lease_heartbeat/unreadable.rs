//! A claim whose record does not open: the heartbeat leaves it out and goes on extending the
//! agent's other claims.

use std::thread;
use std::time::{Duration, Instant};

use rusqlite::{Connection, params};
use skald_proto::TaskId;

use crate::claiming::{Claims, QUEUED, STARTED};
use crate::helped::Helped;
use crate::scene::{
    GAP, LEASE, assert_extended, claude_enqueues, extends_since, helped_claims,
    helped_codex_session, lease_ends, scene, stays_claimed,
};
use crate::senders::LAPTOP_AGENT;

/// What the test enqueues; no text is anywhere else.
const SPOILED: &str = "heartbeat test: claude asks for an audit whose claim record spoils";
const KEPT: &str = "heartbeat test: claude asks for a cleanup codex keeps";

/// The bytes of a claim record that does not open.
const SPOILT: [u8; 1] = [0];

/// Codex's MCP session runs through the helper and codex claims claude's two tasks. The second
/// machine's record of the first claim is overwritten with bytes that do not open ([`spoil`]). For
/// two leases the second task stays `claimed`, an extend of it coming at least each third of the
/// lease; the server's reads show the first one's lease end.
#[test]
fn claim_that_does_not_open_leaves_the_agent_s_other_claims_extended() {
    let claims = scene(
        [4_269_061_001, 4_269_061_002],
        ["ada-spoils-a-claim-record", "bob-watches-the-cleanup"],
    );
    let helped = Helped::new(&claims.laptop().machine);
    let _session = helped_codex_session(&claims, &helped);
    let spoiled = claude_enqueues(&claims, SPOILED);
    helped_claims(&claims, &helped, Some(LAPTOP_AGENT), spoiled);
    let kept = claude_enqueues(&claims, KEPT);
    helped_claims(&claims, &helped, Some(LAPTOP_AGENT), kept);

    let since = spoil(&claims, spoiled, kept);

    stays_claimed(&claims, kept, STARTED, LEASE * 2);

    assert_extended(&claims, (since, Instant::now()), kept, 4);
    let read = lease_ends(&claims, spoiled);
    assert_eq!(read.state, QUEUED, "{read:?}");
}

/// Overwrites the second machine's record of the claim of `spoiled` with [`SPOILT`] until a whole
/// heartbeat after it leaves it so, and gives when the last overwrite came. A heartbeat that read
/// the record before an overwrite writes it again once its extend is answered; the heartbeat after
/// it, which reads it after the overwrite, leaves out a record that does not open, and so does
/// each one after. The extends of `kept`, which each heartbeat sends after `spoiled`'s, tell the
/// heartbeats apart: the second one after the overwrite comes from a heartbeat that started after
/// it.
#[track_caller]
fn spoil(claims: &Claims, spoiled: TaskId, kept: TaskId) -> Instant {
    let state = claims.laptop().machine.state_dir().join("state.sqlite3");
    let state = Connection::open(state).expect("open the second machine's local state");
    state
        .busy_timeout(Duration::from_secs(5))
        .expect("set a busy timeout");
    let record = |state: &Connection| -> Vec<u8> {
        state
            .query_row(
                "SELECT sealed FROM task_claim WHERE task = ?1",
                params![spoiled.to_string()],
                |row| row.get(0),
            )
            .expect("read the record of the first claim")
    };
    for _ in 0..5 {
        let changed = state
            .execute(
                "UPDATE task_claim SET sealed = ?1 WHERE task = ?2",
                params![SPOILT.as_slice(), spoiled.to_string()],
            )
            .expect("overwrite the record of the first claim");
        assert_eq!(changed, 1, "no record of the claim of {spoiled}");
        let since = claims.cloud().relay.mark();
        let deadline = since + GAP * 3;
        while extends_since(claims, since, kept).len() < 2 {
            assert!(
                Instant::now() < deadline,
                "no two extends of {kept} within {:?} of the overwrite",
                GAP * 3
            );
            thread::sleep(Duration::from_millis(100));
        }
        if record(&state) == SPOILT {
            return since;
        }
    }
    panic!("each heartbeat wrote the record of the claim of {spoiled} again");
}
