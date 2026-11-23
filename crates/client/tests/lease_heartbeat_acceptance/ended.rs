//! A session that stops: its claims get no more extends, their leases end, another device claims
//! the task, and the stopped agent's complete says the claim is lost and sends no result.

use crate::claiming::{CLAIMED, COMPLETE, QUEUED, STARTED};
use crate::scene::{LEASE, Scene};

/// What the tests enqueue and answer; no text is anywhere else.
const EXPORT: &str = "heartbeat test: claude asks for an export whose session is killed";
const EXPORTED: &str = "heartbeat test: codex exported after its session was killed";

/// The second machine runs through a helper, and codex's `skald mcp` runs through it. Claude
/// enqueues a task and codex claims it; for two leases it stays held. Codex's `skald mcp` is killed
/// and the helper keeps running. Within the lease and the server's look for ended leases, the
/// server's reads show the task no longer held by codex's claim. Once its backoff passes, the
/// person on Ada's first machine claims it: the server's reads show it `claimed` by that device,
/// in a later generation. Codex's `skald complete <task> <text>` fails and says the claim is lost;
/// no result is sent.
#[test]
fn killed_mcp_session_lets_the_lease_end_and_the_stopped_agent_s_complete_is_lost() {
    let mut scene = Scene::new(
        [4_812_021_001, 4_812_021_002],
        ["ada-kills-the-export-session", "bob-gets-no-export-result"],
    );
    let helped = scene.helped();
    let session = scene.helped_session(&helped);
    let task = scene.claude_enqueues(EXPORT);
    let (held, _) = scene.codex_holds(task);
    let two_leases = std::time::Duration::from_secs(2 * LEASE);
    scene.assert_held_for(task, &held, STARTED, two_leases);

    drop(session);

    assert!(
        !helped.helpers().is_empty(),
        "the helper exited with the session"
    );
    let ended = scene.await_lease_end(task, &held);
    assert_eq!(ended.state, QUEUED, "{ended:?}");
    scene.backoff_passes(task);
    scene.ada_claims();
    let again = scene.claims.read(task);
    assert_eq!(again.state, CLAIMED, "{again:?}");
    let other = again.held();
    assert_eq!(other.device, scene.claims.ada().device, "{again:?}");
    assert!(other.generation > held.generation, "{again:?}");
    scene.assert_codex_lost(COMPLETE, &[&task.to_string(), EXPORTED]);
    scene
        .claims
        .assert_no_result(task, "a complete after the session was killed");
}
