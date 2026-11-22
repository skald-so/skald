//! Results the device drops with a warning: from a device other than the claiming one, naming
//! another claim or generation, sent while the server or a proxy gives the task as not `done`,
//! with another `to` or `from` than the task's, of a task the device did not enqueue or of another
//! channel; and a message with a `task` body.

use serde_json::Value;
use skald_proto::{ClaimId, Content, Outcome, Task, Unknown};

use crate::claiming::address;
use crate::fixtures::TO;
use crate::lines::AFTER_MISSING;
use crate::queuing::tasks_path;
use crate::results::{Results, assert_dropped, assert_result_line, named};
use crate::senders::{LAPTOP_AGENT, labelled_of};
use crate::states::{self, CLAIMED, Held};

/// What the tests enqueue and answer; no text is anywhere else.
const BUILD: &str = "drop test: claude asks for build 733";
const BUILT_ELSEWHERE: &str = "drop test: bob claims build 733 the laptop ran";
const OTHER_CLAIM: &str = "drop test: bob names a claim of build 733 no one held";
const OTHER_GENERATION: &str = "drop test: bob names a later generation of build 733";
const BUILT: &str = "drop test: bob's agent built 733";
const EARLY: &str = "drop test: bob says build 733 is done before it is";
const PROXIED: &str = "drop test: bob's result while a proxy says claimed";
const TO_CODEX: &str = "drop test: bob's result to codex, not to claude";
const FOR_CODEX: &str = "drop test: claude asks codex alone for build 734";
const FROM_BOB: &str = "drop test: bob answers the task meant for codex";
const LAPTOP_S: &str = "drop test: codex on the laptop asks for build 735";
const NOT_ENQUEUED: &str = "drop test: bob answers the laptop's task to claude";
const IN_DEFAULT: &str = "drop test: claude asks in the default channel for build 736";
const OTHER_CHANNEL: &str = "drop test: bob answers the default channel's task here";
const TASK_BODY: &str = "drop test: bob writes a task as a message to claude";
const BEFORE_RESULT: &str = "drop test: bob tells claude the result is coming";
const AFTER_DROPPED: &str = "drop test: bob asks claude whether the result came";

/// Codex on Ada's second machine claims claude's task with `skald claim`, and its device completes
/// it through the server's request, sending no result. Bob's device says a line to claude, then
/// writes a `done` result of the task to claude, naming that claim: claude's `skald recv` shows the
/// line and drops the result with a warning. Bob's device says another line to claude; claude's
/// next run shows it, not after missing ones: the dropped result's counter counts.
#[test]
fn result_from_another_device_than_the_claiming_one_is_dropped() {
    let mut results = Results::new(
        [4_268_111_001, 4_268_111_002],
        ["ada-drops-a-stolen-result", "bob-claims-the-laptop-s-work"],
    );
    let task = results.claude_enqueues(&[BUILD]);
    results.claims.codex_claimed(&[]);
    let claim = results.claims.read(task).held().clone();
    let held = Held {
        at: results.claims.at,
        queue: results.claims.queue,
        task,
        claim_id: claim.claim_id.to_string(),
        generation: claim.generation,
    };
    let (cloud, laptop) = (results.claims.cloud(), results.claims.laptop());
    states::taken(cloud, laptop, &held, ("complete", serde_json::json!({})));
    results.bob_says_to_claude(BEFORE_RESULT);
    let msg_id = results.bob_results_to_claude(&held, (Outcome::Done, BUILT_ELSEWHERE));

    let read = results.claude_reads();

    assert_dropped(&read, msg_id, BUILT_ELSEWHERE);
    labelled_of(&read.stdout, BEFORE_RESULT, BEFORE_RESULT);

    results.bob_says_to_claude(AFTER_DROPPED);

    let read = results.claude_reads();

    let line = labelled_of(&read.stdout, AFTER_DROPPED, AFTER_DROPPED);
    assert!(
        !read.stdout.contains(AFTER_MISSING),
        "{line:?} after missing: {read:?}"
    );
}

/// Bob's device claims and completes claude's task, then writes three `done` results of it to
/// claude: one naming another `claim_id`, one the next generation, and one naming the claim as the
/// server gives it. Claude's `skald recv` drops the first two with a warning each and shows the
/// third.
#[test]
fn result_naming_another_claim_or_generation_is_dropped() {
    let mut results = Results::new(
        [4_268_112_001, 4_268_112_002],
        ["ada-checks-the-claim-named", "bob-names-other-claims"],
    );
    let task = results.claude_enqueues(&[BUILD]);
    let held = results.bob_claims();
    results.bob_completes(&held);
    let (claim_id, generation) = named(&held);
    let claude = results.claims.claude();
    let other_claim = results.bob_results(
        (task, (ClaimId::generate(), generation)),
        &claude,
        (Outcome::Done, OTHER_CLAIM),
    );
    let other_generation = results.bob_results(
        (task, (claim_id, generation + 1)),
        &claude,
        (Outcome::Done, OTHER_GENERATION),
    );
    results.bob_results_to_claude(&held, (Outcome::Done, BUILT));

    let read = results.claude_reads();

    assert_dropped(&read, other_claim, OTHER_CLAIM);
    assert_dropped(&read, other_generation, OTHER_GENERATION);
    assert_result_line(&read.stdout, task, (Outcome::Done, BUILT));
}

/// Bob's device claims claude's task and writes a `done` result of it before it completes it:
/// claude's `skald recv` drops it with a warning. Bob's device completes it, and a proxy gives the
/// task as `claimed` once; Bob's next `done` result is dropped with a warning too. His third one,
/// with the server's answer as it is, is shown.
#[test]
fn done_result_while_the_server_or_a_proxy_gives_the_task_not_done_is_dropped() {
    let mut results = Results::new(
        [4_268_113_001, 4_268_113_002],
        ["ada-waits-for-done", "bob-answers-too-early"],
    );
    let task = results.claude_enqueues(&[BUILD]);
    let held = results.bob_claims();
    let early = results.bob_results_to_claude(&held, (Outcome::Done, EARLY));

    let read = results.claude_reads();

    assert_dropped(&read, early, EARLY);

    results.bob_completes(&held);
    let request = format!("GET {}/{task}", tasks_path(held.at, held.queue));
    results
        .claims
        .cloud()
        .relay
        .rewrite_once(&request, |answer| {
            let mut task: Value = serde_json::from_str(answer).expect("a task in JSON");
            task["state"] = Value::from(CLAIMED);
            task.to_string()
        });
    let proxied = results.bob_results_to_claude(&held, (Outcome::Done, PROXIED));

    let read = results.claude_reads();

    assert_dropped(&read, proxied, PROXIED);

    results.bob_results_to_claude(&held, (Outcome::Done, BUILT));

    let read = results.claude_reads();

    assert_result_line(&read.stdout, task, (Outcome::Done, BUILT));
}

/// Bob's device claims and completes claude's task, then writes a `done` result of it to codex,
/// not to claude, the task's `from`. Claude enqueues a second task `--to` codex, and Bob's device
/// writes a `failed` result of it from Bob's agent, not from codex, its `to`. Claude's `skald recv`
/// drops both with a warning each.
#[test]
fn result_whose_to_or_from_is_not_the_task_s_is_dropped() {
    let mut results = Results::new(
        [4_268_114_001, 4_268_114_002],
        ["ada-checks-whose-result", "bob-answers-the-wrong-agent"],
    );
    let task = results.claude_enqueues(&[BUILD]);
    let held = results.bob_claims();
    results.bob_completes(&held);
    let codex = results.claims.codex();
    let to_codex = results.bob_results((task, named(&held)), &codex, (Outcome::Done, TO_CODEX));
    let addressee = format!("{}/{LAPTOP_AGENT}", results.claims.ada().handle);
    let for_codex = results.claude_enqueues(&[TO, &addressee, FOR_CODEX]);
    let claude = results.claims.claude();
    let from_bob = results.bob_results(
        (for_codex, (ClaimId::generate(), 1)),
        &claude,
        (Outcome::Failed, FROM_BOB),
    );

    let read = results.claude_reads();

    assert_dropped(&read, to_codex, TO_CODEX);
    assert_dropped(&read, from_bob, FROM_BOB);
}

/// Codex on Ada's second machine enqueues a task, which Bob's device claims and completes; Bob's
/// device writes a `done` result of it to claude. Claude enqueues a task in the space's default
/// channel, and Bob's device writes a `failed` result of it to claude in the talk's channel.
/// Claude's `skald recv` on Ada's first machine drops both with a warning each.
#[test]
fn result_of_a_task_the_device_did_not_enqueue_or_of_another_channel_is_dropped() {
    let mut results = Results::new(
        [4_268_115_001, 4_268_115_002],
        ["ada-drops-foreign-results", "bob-answers-foreign-tasks"],
    );
    let laptop = results.claims.laptop();
    let codex = (
        &laptop.machine,
        results.claims.senders.laptop_folder.as_path(),
        Some(LAPTOP_AGENT),
    );
    let laptop_s = results.enqueues(codex, results.claims.at, &[LAPTOP_S]);
    let held = results.bob_claims();
    assert_eq!(held.task, laptop_s, "Bob claimed another task");
    results.bob_completes(&held);
    let not_enqueued = results.bob_results_to_claude(&held, (Outcome::Done, NOT_ENQUEUED));
    let (space, default) = (results.claims.at.0, results.claims.talk().space.default);
    let agents = &results.claims.senders.agents;
    let claude = (
        &agents.talk.ada.machine,
        agents.folder.as_path(),
        Some(crate::agents::CLAUDE),
    );
    let in_default = results.enqueues(claude, (space, default), &[IN_DEFAULT]);
    let to = results.claims.claude();
    let other_channel = results.bob_results(
        (in_default, (ClaimId::generate(), 1)),
        &to,
        (Outcome::Failed, OTHER_CHANNEL),
    );

    let read = results.claude_reads();

    assert_dropped(&read, not_enqueued, NOT_ENQUEUED);
    assert_dropped(&read, other_channel, OTHER_CHANNEL);
}

/// Bob's device writes, in the channel's group, a message to claude whose content holds a `task`
/// body. Claude's `skald recv` drops it with a warning.
#[test]
fn message_with_a_task_body_is_dropped() {
    let mut results = Results::new(
        [4_268_116_001, 4_268_116_002],
        ["ada-drops-a-task-message", "bob-writes-a-task-message"],
    );
    let (from, to) = (
        address(&results.bob_agent()),
        address(&results.claims.claude()),
    );
    let msg_id = results.bob_writes(|msg_id, counter| {
        let request = serde_json::json!({
            "messageId": msg_id.to_string(),
            "role": "ROLE_USER",
            "parts": [{ "text": TASK_BODY }],
        });
        let task = Task {
            request: request.to_string(),
            unknown: Unknown::default(),
        };
        Content {
            counter: Some(counter),
            ..task.content(from, Some(to), 0)
        }
    });

    let read = results.claude_reads();

    assert_dropped(&read, msg_id, TASK_BODY);
}
