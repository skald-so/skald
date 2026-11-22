//! Results the task's sending device drops with a warning, as checks 2 and 3 of "Results" say,
//! each followed by one that passes, which it takes.

use serde_json::{Value, json};
use skald_proto::{AgentState, ClaimId, Outcome};

use crate::agents::{CODEX, run_as_in};
use crate::claiming::{COMPLETE, Craft};
use crate::fixtures::TO;
use crate::queuing::queues_path;
use crate::returns::{
    Answer, COMPLETE_ACT, DONE, OWN, Returns, assert_no_result, assert_one_result,
};
use crate::senders::{BOB_AGENT, LAPTOP_AGENT};
use crate::states::{self, Held};
use crate::values::{status, uploads};
use crate::world::{ACME, HOST, now};

/// What the tests enqueue and answer; no text is anywhere else.
const MIGRATION: &str = "drop test: claude asks for a review of the billing migration";
const FORGED: &str = "drop test: a result the claimer never sent";
const REVIEWED: &str = "drop test: the billing migration looks right";
const ROTATION: &str = "drop test: claude asks for the key rotation plan";
const ROTATION_DONE: &str = "drop test: the key rotation plan is ready";
const WRITTEN: &str = "drop test: a task ada's device wrote without the cli";

/// Bob's agent that no task names.
const BOB_OTHER: &str = "gemini-w8c3nb";

/// The `held` of the claim the server gives of `task` in the talk's channel.
#[track_caller]
fn held_of(returns: &Returns, task: skald_proto::TaskId) -> Held {
    let read = returns.claims.read(task);
    let claim = read.held();
    Held {
        at: returns.claims.at,
        queue: returns.claims.queue,
        task,
        claim_id: claim.claim_id.to_string(),
        generation: claim.generation,
    }
}

/// Codex claims claude's task with `skald claim`; the test completes that claim with the second
/// machine's own request, so the CLI sends no result, and the server gives the task `done`,
/// claimed by the second machine. Bob's device sends a `done` result naming that claim and its
/// generation, from Bob's agent to claude: claude's next run does not hand it over, and the
/// person's warns of it. Codex's `skald complete` then gets the server's first answer and sends
/// its result, which claude's next run hands over from codex.
#[test]
fn done_result_from_a_device_other_than_the_claimer_is_dropped() {
    let mut returns = Returns::new(
        [5_873_011_001, 5_873_011_002],
        ["ada-claims-on-her-laptop", "bob-forges-the-laptop-result"],
    );
    let task = returns.claude_enqueues(MIGRATION);
    returns.claims.codex_claimed(&[]);
    let held = held_of(&returns, task);
    let laptop = returns.claims.laptop();
    states::taken(
        returns.claims.cloud(),
        laptop,
        &held,
        (COMPLETE_ACT, json!({})),
    );
    assert_eq!(returns.claims.read(task).state, DONE);
    let (bob_agent, claude) = (returns.bob_agent(), returns.claims.claude());
    let forged = Answer::of(&held, Outcome::Done, (&bob_agent, &claude), FORGED);

    returns.bob_answers(&forged);

    returns.assert_dropped(task, "a result from Bob's device");
    returns
        .claims
        .codex_ran(COMPLETE, &[&task.to_string(), REVIEWED]);
    returns.claims.results_once_read(task);
    let shown = returns.claude_reads().shown();
    assert_one_result(&shown, task, DONE, (LAPTOP_AGENT, OWN));
    assert!(!shown.contains(FORGED), "{shown}");
}

/// Bob's device claims and completes claude's task, then sends a `done` result naming another
/// `claim_id`: claude's next run does not hand it over, and the person's warns of it. Its result
/// naming the claim the server gives is handed over to claude from Bob's agent at `peer`.
#[test]
fn done_result_with_another_claim_id_is_dropped() {
    let mut returns = Returns::new(
        [5_873_012_001, 5_873_012_002],
        ["ada-checks-the-claim-id", "bob-names-another-claim"],
    );
    let task = returns.claude_enqueues(MIGRATION);
    let right = returns.bob_runs(REVIEWED);
    let forged = Answer {
        claim_id: ClaimId::generate(),
        text: FORGED.to_owned(),
        ..right.clone()
    };

    returns.assert_drops_then_takes(task, (&forged, &right), "another claim_id");
}

/// As [`done_result_with_another_claim_id_is_dropped`], the dropped result naming the claim's
/// generation plus one.
#[test]
fn done_result_with_another_generation_is_dropped() {
    let mut returns = Returns::new(
        [5_873_013_001, 5_873_013_002],
        ["ada-checks-the-generation", "bob-names-another-generation"],
    );
    let task = returns.claude_enqueues(MIGRATION);
    let right = returns.bob_runs(REVIEWED);
    let forged = Answer {
        generation: right.generation + 1,
        text: FORGED.to_owned(),
        ..right.clone()
    };

    returns.assert_drops_then_takes(task, (&forged, &right), "another generation");
}

/// `value` with each `state` of `done` made `claimed`, as a proxy gives the server's reads.
fn not_done(value: &mut Value) {
    match value {
        Value::Object(fields) => {
            if fields.get("state").and_then(Value::as_str) == Some(DONE) {
                fields.insert("state".to_owned(), json!("claimed"));
            }
            fields.values_mut().for_each(not_done);
        }
        Value::Array(items) => items.iter_mut().for_each(not_done),
        _ => {}
    }
}

/// Bob's device claims and completes claude's task. While a proxy gives every read of the
/// channel's queues with `claimed` in place of `done`, Bob's device sends a `done` result naming
/// the claim: claude's next run does not hand it over, and the person's warns of it. Once the
/// proxy gives the reads as they are, Bob's device sends that result again, and claude's next run
/// hands it over.
#[test]
fn done_result_while_a_proxy_shows_the_task_not_done_is_dropped() {
    let mut returns = Returns::new(
        [5_873_014_001, 5_873_014_002],
        ["ada-asks-the-server-first", "bob-completes-behind-a-proxy"],
    );
    let task = returns.claude_enqueues(MIGRATION);
    let answer = returns.bob_runs(REVIEWED);
    let reads = format!("GET {}", queues_path(returns.claims.at));
    let relay = &returns.claims.cloud().relay;
    relay.rewrite(&reads, |given| match serde_json::from_str::<Value>(given) {
        Ok(mut value) => {
            not_done(&mut value);
            value.to_string()
        }
        Err(_) => given.to_owned(),
    });

    returns.bob_answers(&answer);

    returns.assert_dropped(task, "a task the proxy shows claimed");
    let relay = &returns.claims.cloud().relay;
    relay.rewrite(&reads, str::to_owned);
    returns.bob_answers(&answer);
    returns.assert_claude_gets_bob_s(task, DONE);
}

/// Bob's device claims and completes claude's task, then sends a `done` result to Ada's codex,
/// another agent of her first machine: codex's next run does not hand it over, claude's neither,
/// and the person's warns of it. Its result to claude is handed over.
#[test]
fn result_whose_to_is_not_the_task_s_from_is_dropped() {
    let mut returns = Returns::new(
        [5_873_015_001, 5_873_015_002],
        ["ada-has-two-agents-in-acme", "bob-answers-the-wrong-agent"],
    );
    let agents = &returns.claims.senders.agents;
    run_as_in(&agents.talk.ada.machine, &agents.folder, CODEX);
    let task = returns.claude_enqueues(MIGRATION);
    let right = returns.bob_runs(REVIEWED);
    let to_codex = Answer {
        to: returns.claims.ada_agent(CODEX),
        text: FORGED.to_owned(),
        ..right.clone()
    };

    returns.bob_answers(&to_codex);

    let agents = &returns.claims.senders.agents;
    let codex_got = run_as_in(&agents.talk.ada.machine, &agents.folder, CODEX).shown();
    assert_no_result(&codex_got, task, "codex");
    assert!(!codex_got.contains(FORGED), "{codex_got}");
    returns.assert_dropped(task, "a result to codex");
    returns.bob_answers(&right);
    returns.assert_claude_gets_bob_s(task, DONE);
}

/// Bob's device uploads a presence value listing his agent `claude-q3x6np`. As claude, `skald
/// enqueue --to <bob>/claude-q3x6np` puts a task for it; Bob's device claims and completes it,
/// then sends a `done` result from his agent `gemini-w8c3nb`: claude's next run does not hand it
/// over, and the person's warns of it. Its result from `claude-q3x6np` is handed over.
#[test]
fn result_whose_from_is_not_the_task_s_to_is_dropped() {
    let mut returns = Returns::new(
        [5_873_016_001, 5_873_016_002],
        [
            "ada-asks-bob-s-agent-by-name",
            "bob-answers-as-another-agent",
        ],
    );
    let (at, bob_agent) = (returns.claims.at, returns.bob_agent());
    let listed = status(&bob_agent, AgentState::Available, now(), (None, None));
    let talk = &mut returns.claims.senders.agents.talk;
    uploads(
        &talk.world.cloud,
        &mut talk.played[0],
        (at, 1),
        vec![listed],
    );
    let bob = returns.claims.senders.bob();
    let to = format!("{bob}/{BOB_AGENT}");
    let task = returns.claude_enqueues_in(returns.channel(), &[TO, &to], ROTATION);
    let right = returns.bob_runs(ROTATION_DONE);
    let from_other = Answer {
        from: format!("{ACME}/{bob}/{BOB_OTHER}@{HOST}"),
        text: FORGED.to_owned(),
        ..right.clone()
    };

    returns.assert_drops_then_takes(task, (&from_other, &right), "another agent of Bob's");
}

/// Ada's first device writes a task from claude to the queue `default` with the tests' own
/// request, not with `skald enqueue`. Bob's device claims and completes it, then sends a `done`
/// result to claude: claude's next run does not hand it over, and the person's warns of it. A
/// task claude enqueues with `skald enqueue`, which Bob's device claims and completes next, gets
/// its result handed over.
#[test]
fn result_for_a_task_the_device_did_not_enqueue_is_dropped() {
    let mut returns = Returns::new(
        [5_873_017_001, 5_873_017_002],
        ["ada-knows-her-own-tasks", "bob-answers-an-unknown-task"],
    );
    let claude = returns.claims.claude();
    let craft = Craft::new(&claude, returns.claims.queue, WRITTEN);
    let written = returns.claims.ada_writes(&craft).task;
    let unknown = returns.bob_runs(FORGED);
    assert_eq!(
        unknown.task, written,
        "Bob's claim is not of the written task"
    );

    returns.bob_answers(&unknown);

    returns.assert_dropped(written, "a task the CLI did not enqueue");
    let task = returns.claude_enqueues(ROTATION);
    let right = returns.bob_runs(ROTATION_DONE);
    assert_eq!(right.task, task, "Bob's claim is not of claude's task");
    returns.bob_answers(&right);
    returns.assert_claude_gets_bob_s(task, DONE);
}
