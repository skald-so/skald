//! Failing a claim in the CLI: with retry, the device sends a `failed` result and the task waits
//! its backoff; without retry, the task goes `dead` with the reason `failed`.

use skald_proto::Outcome;

use crate::claiming::{Claims, DEAD, FAIL, FAILED, NO_TASK, QUEUED, RETRY_FLAG};

/// What the tests enqueue and answer; no text is anywhere else.
const FLAKY: &str = "fail test: claude asks for a run of the flaky integration suite";
const FLAKY_FAILED: &str = "fail test: codex found the database down, try again later";
const BROKEN: &str = "fail test: claude asks for a build of the deleted branch";
const BROKEN_FAILED: &str = "fail test: codex found no such branch";

/// Codex claims claude's task, then `skald fail --retry <task> <text>`: the server gives it
/// `queued` with attempts 1, and Bob's device reads one result of it from the second machine's
/// device: from codex, to claude, `hops` 1, `failed`, naming the claim, with codex's text. Codex's
/// next claim at once gets no task: the task waits its backoff.
#[test]
fn fail_with_retry_sends_a_failed_result_and_the_task_waits_its_backoff() {
    let mut claims = Claims::new(
        [4_267_021_001, 4_267_021_002],
        ["ada-retries-the-suite", "bob-reads-the-failure"],
    );
    let task = claims.claude_enqueues(FLAKY).task;
    claims.codex_claimed(&[]);
    let held = claims.read(task).held().clone();

    claims.codex_ran(FAIL, &[RETRY_FLAG, &task.to_string(), FLAKY_FAILED]);

    let read = claims.read(task);
    assert_eq!(read.summary(), (QUEUED, None, 1), "{read:?}");
    let claude = claims.claude();
    claims.assert_codex_result((task, &held), (&claude, 1), (Outcome::Failed, FLAKY_FAILED));

    let again = claims.codex_claimed(&[]);

    assert!(
        again.shown().to_lowercase().contains(NO_TASK),
        "the task handed out in its backoff: {again:?}"
    );
    assert!(!again.shown().contains(FLAKY), "{again:?}");
}

/// Codex claims claude's task, then `skald fail <task> <text>`: the server gives it `dead` with the
/// reason `failed`.
#[test]
fn fail_without_retry_makes_the_task_dead() {
    let claims = Claims::new(
        [4_267_022_001, 4_267_022_002],
        ["ada-fails-the-branch", "bob-sees-it-dead"],
    );
    let task = claims.claude_enqueues(BROKEN).task;
    claims.codex_claimed(&[]);

    claims.codex_ran(FAIL, &[&task.to_string(), BROKEN_FAILED]);

    let read = claims.read(task);
    assert_eq!(read.summary(), (DEAD, Some(FAILED), 1), "{read:?}");
}
