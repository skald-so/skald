//! The helper's heartbeat beside codex's commands on the same claim: a command waits for an extend
//! of the heartbeat the server has not answered, and works on the claim as the heartbeat left it.

use std::thread;
use std::time::{Duration, Instant};

use skald_proto::TaskId;

use crate::claiming::{
    AWAITING_APPROVAL, COMPLETE, Claims, DONE, EXTEND, LOST, PROGRESS_FLAG, says,
};
use crate::helped::Helped;
use crate::machine::Finished;
use crate::queuing::tasks_path;
use crate::scene::{
    LEASE, THIRD, claude_enqueues, extends_since, helped_claims, helped_codex_session, helped_runs,
    scene, stays_claimed,
};
use crate::senders::LAPTOP_AGENT;

/// What the tests enqueue and answer; no text is anywhere else.
const APPROVAL: &str = "heartbeat test: claude asks for a deploy codex waits to approve";
const ENDED: &str = "heartbeat test: claude asks for a report codex completes mid-extend";
const ENDED_DONE: &str = "heartbeat test: codex finished the report mid-extend";
const SLOW_EXTEND: &str = "heartbeat test: claude asks for a scan whose extend syncs slowly";
const SLOW_COMPLETE: &str = "heartbeat test: claude asks for a scan whose complete syncs slowly";
const SLOW_DONE: &str = "heartbeat test: codex finished the scan after a slow sync";

/// The lease of the tests that hold the heartbeat's extend, in seconds, and a third of it: room
/// for the hold within the lease on a loaded machine.
const LONG_LEASE: u64 = 15;
const LONG_THIRD: Duration = Duration::from_secs(LONG_LEASE / 3);

/// How long the tests wait for a command to send a request beside a held one.
const SENT_WITHIN: Duration = Duration::from_secs(5);

/// The line that starts an extend of `task` of the claims scene's queue.
fn extend_request(claims: &Claims, task: TaskId) -> String {
    format!(
        "POST {}/{task}/extend ",
        tasks_path(claims.at, claims.queue)
    )
}

/// Waits up to `span` until `done` gives true; gives whether it did.
fn within(span: Duration, done: impl Fn() -> bool) -> bool {
    let until = Instant::now() + span;
    while !done() {
        if Instant::now() >= until {
            return false;
        }
        thread::sleep(Duration::from_millis(100));
    }
    true
}

/// Codex's MCP session runs through `helped`'s helper; codex claims claude's task `text` from the
/// queue, its lease set to `lease` seconds. Gives the task.
fn claimed_with_lease(claims: &Claims, helped: &Helped, text: &str, lease: u64) -> TaskId {
    claims.cloud().execute(&format!(
        "UPDATE queues SET lease = {lease} WHERE id = '{}'::uuid",
        claims.queue
    ));
    let task = claude_enqueues(claims, text);
    helped_claims(claims, helped, Some(LAPTOP_AGENT), task);
    task
}

/// The relay holds the heartbeat's next extend of `task`. Meanwhile codex runs `skald <command>
/// <args>` through the helper; once the command's request ending in `/<act>` came, and the server
/// had half a second for it, or once [`SENT_WITHIN`] passed, the relay lets the extend go on.
/// Gives the command's run.
fn beside_held_extend(
    claims: &Claims,
    helped: &Helped,
    task: TaskId,
    (command, args, act): (&str, &[&str], &str),
) -> Finished {
    let relay = &claims.cloud().relay;
    relay.hold_next(&extend_request(claims, task));
    assert!(
        within(LONG_THIRD * 2, || relay.held() == 1),
        "no extend of {task} came to the hold"
    );
    let since = relay.mark();
    let ending = format!("/tasks/{task}/{act} ");
    thread::scope(|scope| {
        let ran = scope.spawn(|| helped_runs(claims, helped, Some(LAPTOP_AGENT), (command, args)));
        let sent = within(SENT_WITHIN, || {
            (relay.sent_since(since).iter())
                .any(|sent| (sent.head.lines().next()).is_some_and(|line| line.contains(&ending)))
        });
        if sent {
            thread::sleep(Duration::from_millis(500));
        }
        relay.let_go();
        ran.join().expect("the command's run")
    })
}

/// Codex's session runs through the helper and codex holds claude's task. The relay holds the
/// heartbeat's next extend, with `started`, while codex's `skald extend --progress
/// awaiting-approval` runs: the command succeeds, and for two thirds of the lease the server gives
/// the task `awaiting-approval`.
#[test]
fn extend_beside_a_held_heartbeat_extend_keeps_its_progress() {
    let claims = scene(
        [4_269_019_001, 4_269_019_002],
        ["ada-approves-a-deploy", "bob-watches-the-deploy"],
    );
    let helped = Helped::new(&claims.laptop().machine);
    let _session = helped_codex_session(&claims, &helped);
    let task = claimed_with_lease(&claims, &helped, APPROVAL, LONG_LEASE);
    let id = task.to_string();
    let args = [PROGRESS_FLAG, AWAITING_APPROVAL, id.as_str()];

    let ran = beside_held_extend(&claims, &helped, task, (EXTEND, &args, "extend"));

    assert!(ran.success, "the extend: {ran:?}");
    stays_claimed(&claims, task, AWAITING_APPROVAL, LONG_THIRD * 2);
}

/// Codex's session runs through the helper and codex holds claude's task. The relay holds the
/// heartbeat's next extend while codex's `skald complete` runs: the complete succeeds and the
/// server gives the task `done`. A third of the lease later the device holds no claim of it:
/// codex's `skald extend` of it fails without saying the claim is lost.
#[test]
fn complete_beside_a_held_heartbeat_extend_leaves_no_claim() {
    let claims = scene(
        [4_269_019_011, 4_269_019_012],
        ["ada-completes-mid-extend", "bob-reads-the-report"],
    );
    let helped = Helped::new(&claims.laptop().machine);
    let _session = helped_codex_session(&claims, &helped);
    let task = claimed_with_lease(&claims, &helped, ENDED, LONG_LEASE);
    let id = task.to_string();
    let args = [id.as_str(), ENDED_DONE];

    let ran = beside_held_extend(&claims, &helped, task, (COMPLETE, &args, "complete"));

    assert!(ran.success, "the complete: {ran:?}");
    assert_eq!(claims.read(task).state, DONE);
    thread::sleep(THIRD);
    let ran = helped_runs(&claims, &helped, Some(LAPTOP_AGENT), (EXTEND, &[&id]));
    assert!(!ran.success, "an extend of a completed task: {ran:?}");
    assert!(!says(&ran.shown(), LOST), "{ran:?}");
}

/// Codex's session runs through the helper and codex holds `task`, of the lease of [`LEASE`]. The
/// relay holds the sync that codex's `skald <command> <args>` starts with for a lease and a third,
/// while the heartbeat extends the claim at least twice; gives the command's run once the relay
/// lets the sync go on.
fn after_a_sync_past_the_lease(
    claims: &Claims,
    helped: &Helped,
    task: TaskId,
    (command, args): (&str, &[&str]),
) -> Finished {
    let relay = &claims.cloud().relay;
    // The session's runs after the claim end first.
    thread::sleep(THIRD);
    relay.hold_next(&format!("GET /spaces/{}/revisions ", claims.at.0));
    let since = relay.mark();
    thread::scope(|scope| {
        let ran = scope.spawn(|| helped_runs(claims, helped, Some(LAPTOP_AGENT), (command, args)));
        assert!(
            within(SENT_WITHIN, || relay.held() == 1),
            "the command's sync never came"
        );
        thread::sleep(LEASE + THIRD);
        assert!(
            extends_since(claims, since, task).len() >= 2,
            "the heartbeat did not extend {task} while the sync was held"
        );
        relay.let_go();
        ran.join().expect("the command's run")
    })
}

/// While the relay holds the sync of codex's `skald extend --progress awaiting-approval` past the
/// lease of the claim as the command first read it, the heartbeat extends the claim: the command
/// succeeds, and the server gives the task `awaiting-approval`.
#[test]
fn extend_after_a_sync_past_the_lease_extends_the_kept_claim() {
    let claims = scene(
        [4_269_019_021, 4_269_019_022],
        ["ada-scans-slowly", "bob-watches-the-scan"],
    );
    let helped = Helped::new(&claims.laptop().machine);
    let _session = helped_codex_session(&claims, &helped);
    let task = claimed_with_lease(&claims, &helped, SLOW_EXTEND, LEASE.as_secs());
    let id = task.to_string();
    let args = [PROGRESS_FLAG, AWAITING_APPROVAL, id.as_str()];

    let ran = after_a_sync_past_the_lease(&claims, &helped, task, (EXTEND, &args));

    assert!(ran.success, "the extend: {ran:?}");
    stays_claimed(&claims, task, AWAITING_APPROVAL, THIRD);
}

/// While the relay holds the sync of codex's `skald complete` past the lease of the claim as the
/// command first read it, the heartbeat extends the claim: the complete succeeds, and the server
/// gives the task `done`.
#[test]
fn complete_after_a_sync_past_the_lease_completes_the_kept_claim() {
    let claims = scene(
        [4_269_019_031, 4_269_019_032],
        ["ada-completes-slowly", "bob-reads-the-scan"],
    );
    let helped = Helped::new(&claims.laptop().machine);
    let _session = helped_codex_session(&claims, &helped);
    let task = claimed_with_lease(&claims, &helped, SLOW_COMPLETE, LEASE.as_secs());
    let id = task.to_string();
    let args = [id.as_str(), SLOW_DONE];

    let ran = after_a_sync_past_the_lease(&claims, &helped, task, (COMPLETE, &args));

    assert!(ran.success, "the complete: {ran:?}");
    assert_eq!(claims.read(task).state, DONE);
}
