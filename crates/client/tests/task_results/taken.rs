//! Results the device takes: the first `done` one of each task, once, handed to the task's `from`
//! with its `read`; each `failed` one as a failed attempt.

use skald_proto::Outcome;

use crate::agents::CODEX;
use crate::claiming::{CLAIM, COMPLETE, FAIL, RETRY_FLAG};
use crate::fixtures::CHANNEL;
use crate::queuing::run_in;
use crate::results::{Results, assert_dropped, assert_result_line, outcome_mark, result_objects};
use crate::states::passes;

/// What the tests enqueue and answer; no text is anywhere else.
const NOTES: &str = "result test: claude asks for the release notes of build 518";
const NOTES_DONE: &str = "result test: codex wrote the release notes of build 518";
const LINT: &str = "result test: claude asks codex beside it to lint the parser";
const LINT_DONE: &str = "result test: codex beside claude linted the parser";
const SUITE: &str = "result test: claude asks for a run of the flaky suite";
const SUITE_FAILED: &str = "result test: codex found the database down";
const SUITE_DONE: &str = "result test: codex ran the suite, all green";
const DIGEST: &str = "result test: claude asks bob for a digest of the incidents";
const DIGEST_DONE: &str = "result test: bob's agent wrote the digest";
const DIGEST_AGAIN: &str = "result test: bob's agent wrote another digest";

/// The longest a task's backoff after a failed attempt can be, in seconds.
const BACKOFF: u64 = 3_600;

/// Claude enqueues a task; codex on Ada's second machine claims and completes it with `skald
/// complete`. Claude's `skald recv` shows its result once, as `#<ref> (task done): <text>` with the
/// task's reference; claude's next run does not show it again. Bob's device reads, in the channel,
/// a `read` receipt of the result from Ada's first device, from claude to codex: the claiming
/// device gets it under `mls`, as every member device of the group does.
#[test]
fn done_result_reaches_the_enqueuing_agent_once_and_the_claiming_device_gets_its_read() {
    let mut results = Results::new(
        [4_268_101_001, 4_268_101_002],
        ["ada-gets-the-notes-result", "bob-sees-the-notes-read"],
    );
    let task = results.claude_enqueues(&[NOTES]);
    results.claims.codex_claimed(&[]);
    results
        .claims
        .codex_ran(COMPLETE, &[&task.to_string(), NOTES_DONE]);
    let sent = results.claims.results_once_read(task);
    let [result] = sent.as_slice() else {
        panic!("not one result sent: {sent:?}");
    };

    let read = results.claude_reads();

    assert_result_line(&read.stdout, task, (Outcome::Done, NOTES_DONE));
    let again = results.claude_reads();
    assert!(
        !again.shown().contains(NOTES_DONE),
        "shown again: {again:?}"
    );
    let receipts = results.read_receipts_of(result.msg_id);
    let [(sender, receipt)] = receipts.as_slice() else {
        panic!("not one read: {receipts:?}");
    };
    assert_eq!(*sender, results.claims.ada().device, "{receipt:?}");
    assert_eq!(
        receipt.from.to_string(),
        results.claims.claude(),
        "{receipt:?}"
    );
    assert_eq!(
        receipt.to.as_ref().map(ToString::to_string),
        Some(results.claims.codex()),
        "{receipt:?}"
    );
}

/// Claude enqueues a task; codex on the same machine claims and completes it. Claude's `skald recv
/// --json` gives its result once, as an object of the type `result` with the task's ID, `done`,
/// codex's text and the reference of the task; claude's next run gives it no more.
#[test]
fn done_result_of_an_agent_on_the_same_device_reaches_the_enqueuing_agent_once() {
    let results = Results::new(
        [4_268_102_001, 4_268_102_002],
        ["ada-lints-on-one-machine", "bob-watches-one-machine"],
    );
    let task = results.claude_enqueues(&[LINT]);
    let agents = &results.claims.senders.agents;
    let codex = (agents.folder.as_path(), Some(CODEX));
    let machine = &agents.talk.ada.machine;
    let channel = results.claims.channel.as_str();
    let claimed = run_in(machine, codex, CLAIM, &[CHANNEL, channel]);
    assert!(claimed.stdout.contains(LINT), "not claimed: {claimed:?}");
    let completed = run_in(machine, codex, COMPLETE, &[&task.to_string(), LINT_DONE]);
    assert!(completed.success, "{completed:?}");

    let read = results.claude_reads_json();

    let objects = result_objects(&read.stdout);
    let [object] = objects.as_slice() else {
        panic!("not one result: {read:?}");
    };
    assert_eq!(object["task"], task.to_string(), "{object}");
    assert_eq!(object["outcome"], "done", "{object}");
    assert_eq!(object["text"], LINT_DONE, "{object}");
    let reference = object["ref"].as_str().unwrap_or_default();
    assert!(
        crate::queuing::refers_to_task(&format!("#{reference}"), task),
        "not the task's reference: {object}"
    );
    let again = results.claude_reads();
    assert!(!again.shown().contains(LINT_DONE), "shown again: {again:?}");
}

/// Codex claims claude's task and fails it with retry; claude's `skald recv` shows the result as
/// `(task failed)`. Once the backoff passed, codex claims it again and completes it; claude's next
/// run shows that result as `(task done)`.
#[test]
fn failed_result_is_shown_and_a_later_done_one_is_taken() {
    let mut results = Results::new(
        [4_268_103_001, 4_268_103_002],
        ["ada-retries-the-flaky-suite", "bob-sees-the-retry"],
    );
    let task = results.claude_enqueues(&[SUITE]);
    results.claims.codex_claimed(&[]);
    results
        .claims
        .codex_ran(FAIL, &[RETRY_FLAG, &task.to_string(), SUITE_FAILED]);
    results.bob_reads_results(task, 1);

    let failed = results.claude_reads();

    assert_result_line(&failed.stdout, task, (Outcome::Failed, SUITE_FAILED));

    passes(results.claims.cloud(), task, BACKOFF);
    results.claims.codex_claimed(&[]);
    results
        .claims
        .codex_ran(COMPLETE, &[&task.to_string(), SUITE_DONE]);
    results.bob_reads_results(task, 2);

    let done = results.claude_reads();

    assert_result_line(&done.stdout, task, (Outcome::Done, SUITE_DONE));
    assert!(
        !done.stdout.contains(outcome_mark(Outcome::Failed)),
        "{done:?}"
    );
}

/// Bob's device claims and completes claude's task, then writes two `done` results of it from his
/// agent to claude, naming the claim. Claude's `skald recv` shows the first, and drops the second
/// with a warning.
#[test]
fn only_the_first_done_result_is_taken() {
    let mut results = Results::new(
        [4_268_104_001, 4_268_104_002],
        ["ada-takes-one-digest", "bob-sends-two-digests"],
    );
    let task = results.claude_enqueues(&[DIGEST]);
    let held = results.bob_claims();
    results.bob_completes(&held);
    results.bob_results_to_claude(&held, (Outcome::Done, DIGEST_DONE));
    let second = results.bob_results_to_claude(&held, (Outcome::Done, DIGEST_AGAIN));

    let read = results.claude_reads();

    assert_result_line(&read.stdout, task, (Outcome::Done, DIGEST_DONE));
    assert_dropped(&read, second, DIGEST_AGAIN);
}
