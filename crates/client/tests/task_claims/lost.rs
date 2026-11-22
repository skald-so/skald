//! A lost claim: after a cancel, after the lease ended on the server, and once the lease passed on
//! the device's clock without an answered extend, `skald complete` says the claim is lost and no
//! result goes out.

use std::thread;
use std::time::Duration;

use serde_json::json;

use crate::claiming::{CANCELLED, COMPLETE, Claims, Craft, DONE, LOST, QUEUE_FLAG, says, tampers};
use crate::queuing::creates_queue;

/// What the tests enqueue and answer; no text is anywhere else.
const CANCELLED_TASK: &str = "lost test: claude asks for a rollback ada cancels";
const CANCELLED_DONE: &str = "lost test: codex finished the cancelled rollback";
const EXPIRED: &str = "lost test: claude asks for a cache warm-up whose lease ends";
const EXPIRED_DONE: &str = "lost test: codex warmed the cache after its lease";
const SLOW: &str = "lost test: ada asks for a backup whose lease passes on the laptop";
const SLOW_DONE: &str = "lost test: codex finished the backup too late";

/// The shortest lease a queue has, in seconds, and what the device's clock waits past it.
const SHORT_LEASE: u64 = 30;
const PAST_THE_LEASE: Duration = Duration::from_secs(SHORT_LEASE + 3);

/// Codex claims claude's task; Ada's first device cancels it through the API. Codex's `skald
/// complete <task> <text>` fails and says `lost`; the server gives the task `cancelled`, and no
/// result is sent.
#[test]
fn complete_after_a_cancel_says_the_claim_is_lost_and_sends_no_result() {
    let mut claims = Claims::new(
        [4_267_031_001, 4_267_031_002],
        ["ada-cancels-the-rollback", "bob-gets-no-rollback-result"],
    );
    let task = claims.claude_enqueues(CANCELLED_TASK).task;
    claims.codex_claimed(&[]);
    claims.ada_cancels(task);

    let ran = claims.codex_runs(COMPLETE, &[&task.to_string(), CANCELLED_DONE]);

    assert!(!ran.success, "a complete of a cancelled claim: {ran:?}");
    assert!(says(&ran.shown(), LOST), "{ran:?}");
    let read = claims.read(task);
    assert_eq!(read.state, CANCELLED, "{read:?}");
    claims.assert_no_result(task, "a cancelled claim");
}

/// Codex claims claude's task, and its lease ends on the server: the server's end of the lease
/// moves back past now. Codex's `skald complete <task> <text>` fails and says `lost`; the server
/// does not give the task `done`, and no result is sent.
#[test]
fn complete_after_the_lease_ended_says_the_claim_is_lost_and_sends_no_result() {
    let mut claims = Claims::new(
        [4_267_032_001, 4_267_032_002],
        ["ada-lets-the-lease-end", "bob-gets-no-cache-result"],
    );
    let task = claims.claude_enqueues(EXPIRED).task;
    claims.codex_claimed(&[]);
    tampers(
        claims.cloud(),
        task,
        "lease_until = now() - interval '1 second'",
    );

    let ran = claims.codex_runs(COMPLETE, &[&task.to_string(), EXPIRED_DONE]);

    assert!(!ran.success, "a complete after the lease: {ran:?}");
    assert!(says(&ran.shown(), LOST), "{ran:?}");
    let read = claims.read(task);
    assert_ne!(read.state, DONE, "{read:?}");
    claims.assert_no_result(task, "a claim whose lease ended");
}

/// Ada's first device creates `backups` with a lease of 30 seconds and writes a task there. Codex
/// claims it with `--queue backups`; the server's lease is moved an hour ahead, so only the
/// device's clock ends it. Past 30 seconds with no extend, codex's `skald complete <task> <text>`
/// fails and says `lost`; the server does not give the task `done`, and no result is sent.
#[test]
fn complete_once_the_lease_passed_on_the_device_s_clock_says_the_claim_is_lost() {
    let mut claims = Claims::new(
        [4_267_033_001, 4_267_033_002],
        ["ada-waits-past-the-lease", "bob-gets-no-backup-result"],
    );
    let settings = json!({ "name": "backups", "lease": SHORT_LEASE });
    let backups = creates_queue(claims.cloud(), claims.ada(), claims.at, &settings);
    let person = claims.person();
    let task = claims
        .ada_writes(&Craft::new(&person, backups.id, SLOW))
        .task;
    claims.codex_claimed(&[QUEUE_FLAG, "backups"]);
    tampers(
        claims.cloud(),
        task,
        "lease_until = now() + interval '1 hour'",
    );
    thread::sleep(PAST_THE_LEASE);

    let ran = claims.codex_runs(COMPLETE, &[&task.to_string(), SLOW_DONE]);

    assert!(!ran.success, "a complete past the lease: {ran:?}");
    assert!(says(&ran.shown(), LOST), "{ran:?}");
    let read = claims.read_in(backups.id, task);
    assert_ne!(read.state, DONE, "{read:?}");
    claims.assert_no_result(task, "a claim whose lease passed on the device's clock");
}
