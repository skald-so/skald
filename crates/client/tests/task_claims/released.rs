//! Tasks a `client` device releases, their attempt not counted, and never hands to the agent: one
//! of a trust level its claim did not state, one naming a template, one whose `to` names no agent
//! of the device, and one under an epoch whose key the device lacks ("Claim checks").

use crate::claiming::{
    Claims, Craft, QUEUED, gives_bytes, hands_out, requests_naming, sent_act, tampers,
};

/// What the tests write; no text is anywhere else.
const BOB_S: &str = "release test: bob asks ada's agents to rotate the api keys";
const TEMPLATED: &str = "release test: ada asks for a review through the review template";
const FOR_GEMINI: &str = "release test: ada asks her gemini, which the laptop lacks";
const NEXT_EPOCH: &str = "release test: ada's task under an epoch the laptop never had";

/// An agent of Ada's that never ran on her second machine.
const GEMINI: &str = "gemini-p4r8vd";

/// The template the task names, and an epoch the channel never had.
const REVIEW: &str = "review";
const NO_SUCH_EPOCH: u64 = 99;

/// Bob's device writes a task, `peer` for Ada's devices, which the server stores. The relay
/// answers codex's next claim itself with a claim of that task from Bob's device, as a compromised
/// server would hand it to a claim stating only `own`. Codex's `skald claim` shows none of its
/// text; the second machine releases it and sends no extend, complete or fail of it.
#[test]
fn peer_task_handed_to_an_own_claim_is_released() {
    let mut claims = Claims::new(
        [4_267_061_001, 4_267_061_002],
        ["ada-gets-bob-s-task", "bob-writes-a-peer-task"],
    );
    let bob_person = claims.bob_person();
    let record = claims.bob_writes(&Craft::new(&bob_person, claims.queue, BOB_S));
    let cloud = claims.cloud();
    let since = hands_out(cloud, claims.at, claims.bob().device(), &record);

    let ran = claims.codex_claims(&[]);

    claims.cloud().relay.stop_refusing();
    assert!(!ran.shown().contains(BOB_S), "handed to codex: {ran:?}");
    let requests = requests_naming(claims.cloud(), since, record.task);
    assert!(sent_act(&requests, "release"), "not released: {requests:?}");
    for act in ["extend", "complete", "fail"] {
        assert!(!sent_act(&requests, act), "{act} sent: {requests:?}");
    }
}

/// Ada's first device writes three tasks to the queue `default`: one naming the template
/// `review`, which a compromised server gives as naming none; one for the device of Ada's second
/// machine whose `to` is `gemini-p4r8vd`, an agent that never ran there; and one the server gives
/// under epoch 99, whose key no device holds. Codex's claims show none of their texts; the server
/// gives each `queued` with attempts 0, its last claim the second machine's device's.
#[test]
fn tasks_failing_the_template_addressee_or_key_checks_are_released_their_attempt_not_counted() {
    let mut claims = Claims::new(
        [4_267_062_001, 4_267_062_002],
        ["ada-writes-unclaimable-tasks", "bob-sees-them-released"],
    );
    let (person, queue) = (claims.person(), claims.queue);
    let laptop = claims.laptop().device;
    let mut templated = Craft::new(&person, queue, TEMPLATED);
    templated.sched.template = Some(REVIEW.parse().expect("a name"));
    let templated = claims.ada_writes(&templated).task;
    tampers(claims.cloud(), templated, "template = NULL");
    let mut for_gemini = Craft::new(&person, queue, FOR_GEMINI);
    for_gemini.sched.device = Some(laptop);
    for_gemini.to = Some(claims.ada_agent(GEMINI));
    let for_gemini = claims.ada_writes(&for_gemini).task;
    let mut next_epoch = claims.ada_writes(&Craft::new(&person, queue, NEXT_EPOCH));
    next_epoch.epoch = NO_SUCH_EPOCH;
    gives_bytes(claims.cloud(), &next_epoch);

    let shown = claims.codex_claims_times(3, &[]);

    for (task, text) in [
        (templated, TEMPLATED),
        (for_gemini, FOR_GEMINI),
        (next_epoch.task, NEXT_EPOCH),
    ] {
        assert!(!shown.contains(text), "{text:?} handed to codex: {shown}");
        let read = claims.read(task);
        assert_eq!(read.summary(), (QUEUED, None, 0), "{text:?}: {read:?}");
        assert_eq!(read.held().device, laptop, "{text:?} not claimed: {read:?}");
    }
}
