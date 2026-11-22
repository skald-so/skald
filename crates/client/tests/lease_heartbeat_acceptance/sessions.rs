//! Claims kept while a session of their agent runs through the helper: its `skald mcp` or its
//! `skald recv --wait`, each third of the lease, with the claim's last progress, until the claim
//! ends.

use skald_proto::Outcome;

use crate::claiming::{AWAITING_APPROVAL, COMPLETE, DONE, EXTEND, PROGRESS_FLAG, STARTED};
use crate::scene::{LEASE, Scene, THIRD, THREE_LEASES, assert_held, waits};

/// What the tests enqueue and answer; no text is anywhere else.
const MIGRATION: &str = "heartbeat test: claude asks for a slow migration of the invoices";
const MIGRATED: &str = "heartbeat test: codex migrated the invoices after three leases";
const REINDEX: &str = "heartbeat test: claude asks for a reindex while codex waits";
const REINDEXED: &str = "heartbeat test: codex reindexed the search tables";

/// The second machine runs through a helper, and codex's `skald mcp` runs through it in
/// `~/work/acme`. Claude enqueues a task; codex claims it with `skald claim` and runs no command
/// for three leases. Throughout, the server's reads show the task `claimed` by the same claim with
/// `started`, and the machine sent an extend of it each third of the lease. Codex's
/// `skald extend --progress awaiting-approval <task>` shows in the server's reads at once, and
/// for one more lease the task stays held with `awaiting-approval`, still extended each third.
/// Codex's `skald complete <task> <text>` then succeeds: the task is `done`, and Bob's device reads
/// its one result from codex.
#[test]
fn agent_whose_mcp_session_runs_through_the_helper_keeps_its_task_for_three_leases() {
    let mut scene = Scene::new(
        [4_812_011_001, 4_812_011_002],
        [
            "ada-runs-a-slow-migration",
            "bob-reads-the-migration-result",
        ],
    );
    let helped = scene.helped();
    let _session = scene.helped_session(&helped);
    let task = scene.claude_enqueues(MIGRATION);
    let (held, since) = scene.codex_holds(task);

    scene.assert_held_for(task, &held, STARTED, THREE_LEASES);

    scene.assert_heartbeat(since, task);
    let id = task.to_string();
    let waiting = scene.claims.cloud().relay.mark();
    scene
        .claims
        .codex_ran(EXTEND, &[PROGRESS_FLAG, AWAITING_APPROVAL, &id]);
    assert_held(&scene.claims.read(task), &held, AWAITING_APPROVAL);
    let lease = std::time::Duration::from_secs(LEASE);
    scene.assert_held_for(task, &held, AWAITING_APPROVAL, lease);
    scene.assert_heartbeat(waiting, task);

    scene.claims.codex_ran(COMPLETE, &[&id, MIGRATED]);

    assert_eq!(scene.claims.read(task).state, DONE);
    let claude = scene.claims.claude();
    scene
        .claims
        .assert_codex_result((task, &held), (&claude, 1), (Outcome::Done, MIGRATED));
}

/// The second machine runs through a helper, and codex's `skald recv --wait` waits through it in
/// `~/work/acme`. Claude enqueues a task and codex claims it. For two leases the server's reads
/// show it `claimed` by the same claim with `started`, extended each third of the lease, while the
/// wait goes on. Codex's `skald complete <task> <text>` succeeds and the task is `done`; from then
/// on, for two thirds of the lease, the machine sends no extend of it.
#[test]
fn agent_whose_recv_wait_runs_through_the_helper_keeps_its_task_until_it_completes_it() {
    let scene = Scene::new(
        [4_812_012_001, 4_812_012_002],
        ["ada-waits-for-a-reindex", "bob-reads-the-reindex-result"],
    );
    let helped = scene.helped();
    let task = scene.claude_enqueues(REINDEX);
    let mut wait = scene.helped_wait(&helped);
    let (held, since) = scene.codex_holds(task);

    let two_leases = std::time::Duration::from_secs(2 * LEASE);
    scene.assert_held_for(task, &held, STARTED, two_leases);

    scene.assert_heartbeat(since, task);
    wait.assert_running();
    let id = task.to_string();
    scene.claims.codex_ran(COMPLETE, &[&id, REINDEXED]);
    let completed = scene.claims.cloud().relay.mark();
    assert_eq!(scene.claims.read(task).state, DONE);
    waits(THIRD * 2);
    scene.assert_no_extend(completed, task, "after the complete");
    drop(wait);
}
