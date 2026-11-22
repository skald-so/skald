//! The agent's presence while it holds a claim: `busy` with `started`, `waiting` with
//! `awaiting-approval`, and `available` again once it holds none.

use skald_proto::AgentState;

use crate::claiming::{AWAITING_APPROVAL, COMPLETE, EXTEND, PROGRESS_FLAG};
use crate::scene::Scene;

/// What the tests enqueue and answer; no text is anywhere else.
const REVIEW: &str = "heartbeat test: claude asks codex to review the billing change";
const REVIEWED: &str = "heartbeat test: codex reviewed the billing change";

/// The second machine runs through a helper, and codex's `skald mcp` runs through it: its value in
/// the talk's channel, as Bob's device opens it, lists codex `available`. Claude enqueues a task;
/// once codex claims it, the value lists codex `busy`; after codex's `skald extend --progress
/// awaiting-approval <task>`, `waiting`; after its `skald complete <task> <text>`, `available`.
#[test]
fn agent_shows_busy_while_it_holds_a_claim_then_waiting_then_available() {
    let mut scene = Scene::new(
        [4_812_061_001, 4_812_061_002],
        ["ada-watches-codex-review", "bob-reads-codex-presence"],
    );
    let helped = scene.helped();
    let _session = scene.helped_session(&helped);
    scene.await_codex_shown(AgentState::Available);
    let task = scene.claude_enqueues(REVIEW);
    let id = task.to_string();

    scene.codex_holds(task);
    scene.await_codex_shown(AgentState::Busy);

    scene
        .claims
        .codex_ran(EXTEND, &[PROGRESS_FLAG, AWAITING_APPROVAL, &id]);
    scene.await_codex_shown(AgentState::Waiting);

    scene.claims.codex_ran(COMPLETE, &[&id, REVIEWED]);
    scene.await_codex_shown(AgentState::Available);
}
