//! A claim whose record does not open: the heartbeat leaves it out and goes on extending the
//! agent's other claims.

use std::time::{Duration, Instant};

use rusqlite::{Connection, params};

use crate::claiming::{QUEUED, STARTED};
use crate::helped::Helped;
use crate::scene::{
    LEASE, assert_extended, claude_enqueues, helped_claims, helped_codex_session, lease_ends,
    scene, stays_claimed,
};
use crate::senders::LAPTOP_AGENT;

/// What the test enqueues; no text is anywhere else.
const SPOILED: &str = "heartbeat test: claude asks for an audit whose claim record spoils";
const KEPT: &str = "heartbeat test: claude asks for a cleanup codex keeps";

/// Codex's MCP session runs through the helper and codex claims claude's two tasks. The second
/// machine's record of the first claim is overwritten with bytes that do not open. For two leases
/// the second task stays `claimed`, an extend of it coming at least each third of the lease; the
/// server's reads show the first one's lease end.
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

    let state = claims.laptop().machine.state_dir().join("state.sqlite3");
    let state = Connection::open(state).expect("open the second machine's local state");
    state
        .busy_timeout(Duration::from_secs(5))
        .expect("set a busy timeout");
    let changed = state
        .execute(
            "UPDATE task_claim SET sealed = X'00' WHERE task = ?1",
            params![spoiled.to_string()],
        )
        .expect("overwrite the record of the first claim");
    assert_eq!(changed, 1, "no record of the claim of {spoiled}");
    drop(state);
    let since = claims.cloud().relay.mark();

    stays_claimed(&claims, kept, STARTED, LEASE * 2);

    assert_extended(&claims, (since, Instant::now()), kept, 4);
    let read = lease_ends(&claims, spoiled);
    assert_eq!(read.state, QUEUED, "{read:?}");
}
