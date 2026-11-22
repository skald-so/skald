//! `skald recv --wait` as the session: through the helper, it keeps its agent's claim, and its
//! output says when the heartbeat finds the claim lost; one-shot, it extends the claim itself and
//! once killed lets the lease end.

use std::thread;
use std::time::{Duration, Instant};

use crate::claiming::{LOST, QUEUED, STARTED, names_task, says};
use crate::helped::Helped;
use crate::kit::RECV;
use crate::scene::{
    LEASE, THIRD, claude_enqueues, helped_claims, laptop_folder, lease_ends, scene, stays_claimed,
};
use crate::senders::LAPTOP_AGENT;
use crate::world::AGENT;

/// The flag of a `skald recv` that waits.
const WAIT_FLAG: &str = "--wait";

/// What the tests enqueue; no text is anywhere else.
const WATCHED: &str = "heartbeat test: claude asks for a scan ada cancels while codex waits";
const ONE_SHOT: &str = "heartbeat test: claude asks for an export a one-shot wait keeps";

/// How long a test waits for the waiting command to say the claim is lost.
const SAID_WITHIN: Duration = Duration::from_secs(20);

/// Codex's `skald recv --wait` runs through the helper and codex claims claude's task: it stays
/// claimed for more than a lease. Ada's first device cancels it through the API: the waiting
/// command's output says the claim of the task is lost.
#[test]
fn helped_wait_keeps_its_agent_s_claim_and_says_when_it_is_lost() {
    let claims = scene(
        [4_269_051_001, 4_269_051_002],
        ["ada-cancels-a-watched-scan", "bob-gets-no-scan-result"],
    );
    let helped = Helped::new(&claims.laptop().machine);
    let args = [AGENT, LAPTOP_AGENT, RECV, WAIT_FLAG];
    let waiting = helped.start_in(laptop_folder(&claims), &[], &args);
    let task = claude_enqueues(&claims, WATCHED);
    helped_claims(&claims, &helped, Some(LAPTOP_AGENT), task);

    stays_claimed(&claims, task, STARTED, LEASE + THIRD);

    claims.ada_cancels(task);
    let deadline = Instant::now() + SAID_WITHIN;
    while !says(&waiting.shown(), LOST) {
        assert!(
            Instant::now() < deadline,
            "the wait did not say the claim is lost: {}",
            waiting.shown()
        );
        thread::sleep(Duration::from_millis(200));
    }
    let shown = waiting.kill().shown();
    assert!(names_task(&shown, task), "{shown}");
}

/// Codex's `skald recv --wait` runs one-shot on the second machine, where no helper runs, and
/// codex's one-shot `skald claim` claims claude's task: it stays claimed for two leases. Once the
/// wait is killed, the server's reads show the lease end.
#[test]
fn one_shot_wait_keeps_its_agent_s_claim_and_once_killed_lets_it_end() {
    let claims = scene(
        [4_269_052_001, 4_269_052_002],
        ["ada-waits-one-shot", "bob-watches-the-export"],
    );
    let args = [AGENT, LAPTOP_AGENT, RECV, WAIT_FLAG];
    let waiting = (claims.laptop().machine).start_in(laptop_folder(&claims), &args);
    let task = claude_enqueues(&claims, ONE_SHOT);
    claims.codex_claimed(&[]);

    stays_claimed(&claims, task, STARTED, LEASE * 2);

    waiting.kill();
    let read = lease_ends(&claims, task);
    assert_eq!(read.state, QUEUED, "{read:?}");
}
