//! Without the helper: a one-shot `skald mcp` extends its agent's claims itself, and once killed
//! lets their leases end.

use crate::claiming::{COMPLETE, QUEUED, STARTED};
use crate::scene::{Scene, THREE_LEASES};

/// What the tests enqueue and answer; no text is anywhere else.
const AUDIT: &str = "heartbeat test: claude asks for an audit run without a helper";
const AUDITED: &str = "heartbeat test: codex audited after its one-shot session was killed";

/// No helper runs on the second machine; codex's `skald mcp` runs one-shot in `~/work/acme`.
/// Claude enqueues a task and codex claims it. For three leases the server's reads show it
/// `claimed` by the same claim with `started`, extended each third of the lease. Codex's `skald
/// mcp` is killed: within the lease and the server's look for ended leases, the reads show the task
/// no longer held by that claim, and codex's `skald complete <task> <text>` fails and says the
/// claim is lost; no result is sent.
#[test]
fn one_shot_mcp_session_keeps_its_agent_s_claim_and_once_killed_lets_it_end() {
    let mut scene = Scene::new(
        [4_812_031_001, 4_812_031_002],
        ["ada-audits-without-a-helper", "bob-gets-no-audit-result"],
    );
    let session = scene.one_shot_session();
    let task = scene.claude_enqueues(AUDIT);
    let (held, since) = scene.codex_holds(task);

    scene.assert_held_for(task, &held, STARTED, THREE_LEASES);

    scene.assert_heartbeat(since, task);
    drop(session);
    let ended = scene.await_lease_end(task, &held);
    assert_eq!(ended.state, QUEUED, "{ended:?}");
    scene.assert_codex_lost(COMPLETE, &[&task.to_string(), AUDITED]);
    scene
        .claims
        .assert_no_result(task, "a complete after the one-shot session was killed");
}
