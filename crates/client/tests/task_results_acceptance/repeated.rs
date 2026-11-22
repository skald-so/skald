//! The first `done` result that passes is the task's; later ones are not taken. A `failed` one is
//! handed over as a failed attempt.

use skald_proto::Outcome;

use crate::returns::{
    Answer, DONE, FAILED, PEER, Returns, assert_no_result, holds_part, result_objects,
};
use crate::senders::BOB_AGENT;
use crate::states::passes;

/// What the tests enqueue and answer; no text is anywhere else.
const SCHEMA: &str = "repeat test: claude asks for the schema of the audit log";
const FIRST: &str = "repeat test: the audit log has five columns";
const SECOND: &str = "repeat test: the audit log has six columns after all";
const BACKUP: &str = "repeat test: claude asks for a restore drill of last night's backup";
const NOT_YET: &str = "repeat test: the backup bucket was unreachable";
const RESTORED: &str = "repeat test: the backup restored in four minutes";

/// How far the tests move a task's backoff: past its longest.
const PAST_BACKOFF: u64 = 3_600;

/// Bob's device claims and completes claude's task and sends two `done` results naming the claim,
/// one after the other. claude's next `skald recv --json` gives one result of the task: a JSON
/// object that names the task, `done`, Bob's agent, `peer` and the first result's text; nothing it
/// gives holds the second's text. The run after it gives no result of the task.
#[test]
fn only_the_first_done_result_is_taken_and_json_has_every_field() {
    let mut returns = Returns::new(
        [5_873_031_001, 5_873_031_002],
        ["ada-takes-the-first-result", "bob-answers-twice"],
    );
    let task = returns.claude_enqueues(SCHEMA);
    let first = returns.bob_runs(FIRST);
    let second = Answer {
        text: SECOND.to_owned(),
        ..first.clone()
    };
    returns.bob_answers(&first);
    returns.bob_answers(&second);

    let got = returns.claude_reads_json();

    let objects = result_objects(&got.stdout, task, DONE);
    let [object] = objects.as_slice() else {
        panic!("not one done result of {task}: {got:?}");
    };
    assert!(holds_part(object, BOB_AGENT), "no claiming agent: {object}");
    assert!(crate::lines::holds_text(object, PEER), "no level: {object}");
    assert!(holds_part(object, FIRST), "not the first text: {object}");
    assert!(!got.shown().contains(SECOND), "the second result: {got:?}");
    let again = returns.claude_reads();
    assert_no_result(&again.shown(), task, "claude's second run");
    assert!(
        !again.shown().contains(SECOND),
        "the second result: {again:?}"
    );
}

/// Bob's device claims claude's task, fails it with retry and sends a `failed` result naming the
/// claim: claude's next run hands it over as `task failed`. Past the backoff, Bob's device claims
/// the task again, completes it and sends a `done` result naming the new claim: claude's next run
/// hands it over as `task done`.
#[test]
fn a_failed_result_is_handed_over_and_a_later_done_one_taken() {
    let mut returns = Returns::new(
        [5_873_032_001, 5_873_032_002],
        [
            "ada-waits-out-a-failed-attempt",
            "bob-retries-the-restore-drill",
        ],
    );
    let task = returns.claude_enqueues(BACKUP);
    let held = returns.bob_claims();
    returns.bob_fails(&held, true);
    let (bob_agent, claude) = (returns.bob_agent(), returns.claims.claude());
    let failed = Answer::of(&held, Outcome::Failed, (&bob_agent, &claude), NOT_YET);

    returns.bob_answers(&failed);

    returns.assert_claude_gets_bob_s(task, FAILED);
    passes(returns.claims.cloud(), task, PAST_BACKOFF);
    let again = returns.bob_claims();
    assert_eq!(again.task, task, "not claude's task claimed again");
    assert!(
        again.generation > held.generation,
        "{again:?} after {held:?}"
    );
    returns.bob_completes(&again);
    let done = Answer::of(&again, Outcome::Done, (&bob_agent, &claude), RESTORED);
    returns.bob_answers(&done);
    let shown = returns.assert_claude_gets_bob_s(task, DONE);
    assert!(
        crate::returns::result_lines(&shown, task, FAILED).is_empty(),
        "the failed result again: {shown}"
    );
}
