//! One-shot sessions: with no helper, a one-shot `skald mcp` extends its agent's claim itself, and
//! once killed lets its lease end.

use std::time::Instant;

use crate::claiming::{QUEUED, STARTED};
use crate::scene::{
    LEASE, assert_extended, claude_enqueues, lease_ends, one_shot_codex_session, scene,
    stays_claimed,
};

/// What the test enqueues; no text is anywhere else.
const BACKUP: &str = "heartbeat test: claude asks for a backup a one-shot session keeps";

/// Codex's MCP session runs one-shot on the second machine, where no helper runs, and codex's
/// one-shot `skald claim` claims claude's task. For two leases the server gives the task
/// `claimed` with `started`, an extend coming at least each third of the lease. Once the session
/// is killed, the server's reads show the lease end.
#[test]
fn one_shot_session_keeps_its_agent_s_claim_and_once_killed_lets_it_end() {
    let claims = scene(
        [4_269_021_001, 4_269_021_002],
        ["ada-keeps-a-one-shot-claim", "bob-watches-the-backup"],
    );
    let session = one_shot_codex_session(&claims);
    let task = claude_enqueues(&claims, BACKUP);
    let since = claims.cloud().relay.mark();
    claims.codex_claimed(&[]);

    stays_claimed(&claims, task, STARTED, LEASE * 2);

    assert_extended(&claims, (since, Instant::now()), task, 4);
    drop(session);
    let read = lease_ends(&claims, task);
    assert_eq!(read.state, QUEUED, "{read:?}");
}
