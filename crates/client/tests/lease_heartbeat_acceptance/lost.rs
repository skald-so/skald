//! A claim the heartbeat finds lost: said in the session's next tool result, and extended no more.

use std::time::{Duration, Instant};

use crate::claiming::{CANCELLED, LOST, STARTED, names_task, says};
use crate::queuing::{proved, tasks_path};
use crate::scene::{LATE, LEASE, Scene, THIRD, waits};

/// What the tests enqueue; no text is anywhere else.
const DEPLOY: &str = "heartbeat test: claude asks for a deploy ada cancels meanwhile";

/// How long the session's tool results are looked at for the lost claim: the next heartbeat and
/// room for a loaded machine.
const SAID_WITHIN: Duration = Duration::from_secs(LEASE / 3 + 20);

/// The second machine runs through a helper, and codex's `skald mcp` runs through it. Claude
/// enqueues a task and codex claims it; for a third of the lease it stays held. Ada's first device
/// cancels it through the API. Within the next heartbeat, a result of the session's `inbox` tool
/// says the claim is lost and names the task. From then on, for two thirds of the lease, the
/// machine sends no extend of it.
#[test]
fn claim_found_lost_is_said_in_the_session_s_next_tool_result_and_extended_no_more() {
    let scene = Scene::new(
        [4_812_051_001, 4_812_051_002],
        ["ada-cancels-the-deploy", "bob-sees-no-deploy"],
    );
    let helped = scene.helped();
    let mut session = scene.helped_session(&helped);
    let task = scene.claude_enqueues(DEPLOY);
    let (held, _) = scene.codex_holds(task);
    scene.assert_held_for(task, &held, STARTED, THIRD);
    let claims = &scene.claims;
    let path = format!("{}/{task}/cancel", tasks_path(claims.at, claims.queue));
    let (status, body) = proved(claims.cloud(), claims.ada(), ("POST", &path), None);
    assert!(
        (200..300).contains(&status),
        "not cancelled: {status} {body}"
    );
    assert_eq!(scene.claims.read(task).state, CANCELLED);

    let until = Instant::now() + SAID_WITHIN;
    let mut shown = Vec::new();
    loop {
        let text = session.inbox();
        if says(&text, LOST) && names_task(&text, task) {
            break;
        }
        shown.push(text);
        assert!(
            Instant::now() < until,
            "no tool result said the claim of {task} is lost within {SAID_WITHIN:?}: {shown:?}"
        );
        waits(Duration::from_secs(1));
    }

    let said = scene.claims.cloud().relay.mark();
    waits(THIRD * 2 + LATE);
    scene.assert_no_extend(said, task, "after the claim was found lost");
}
