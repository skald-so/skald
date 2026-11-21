//! Claim checks that a test device or a compromised server breaks otherwise than the tests of
//! refusals and releases do: a `sender` that is not the claim's `uploader`, a `to` without
//! `sched.device` or the reverse, a grant in a `client` sender's content, and a `to` that names
//! another agent than the one that claims ("Claim checks").

use skald_proto::{Cbor as _, Grant, GrantBody, NonEmpty, TaskId, Unknown};

use crate::claiming::{CLAIM, Claims, Craft, DEAD, FAILED, QUEUED, sealed_by, uploads, warns_of};
use crate::fixtures::CHANNEL;
use crate::queuing::run_in;
use crate::senders::LAPTOP_AGENT;
use crate::world::now;

/// What the tests write; no text is anywhere else.
const UPLOADED: &str = "check test: bob's task that ada's device uploads";
const TO_ONLY: &str = "check test: a task for codex whose sched names no device";
const DEVICE_ONLY: &str = "check test: a task for the laptop whose content names no agent";
const GRANTED: &str = "check test: ada's task carrying a grant";
const FOR_CODEX: &str = "check test: ada asks codex alone for the release notes";

/// An agent of Ada's other than codex, which claims on her second machine.
const GEMINI: &str = "gemini-p4r8vd";

/// How long the grant in a task holds, in seconds.
const GRANT_HOLDS: u64 = 3600;

/// Fails unless `task`, of `text`, is `dead` with the reason `failed`, `shown` holds not its text
/// and has a warning line naming it.
#[track_caller]
fn assert_refused(claims: &Claims, (task, text): (TaskId, &str), shown: &str) {
    assert!(!shown.contains(text), "{text:?} handed over: {shown}");
    let read = claims.read(task);
    assert_eq!(
        read.summary(),
        (DEAD, Some(FAILED), 1),
        "{text:?}: {read:?}"
    );
    assert!(
        warns_of(shown, task),
        "{text:?}: no warning naming it: {shown}"
    );
}

/// Bob's device writes and signs a task, from Bob's person address, and Ada's first device
/// uploads it. Codex's claim shows none of its text and a warning naming it; the server gives it
/// `dead` with the reason `failed`.
#[test]
fn task_whose_sender_is_not_its_uploader_is_failed_without_retry() {
    let mut claims = Claims::new(
        [4_267_101_001, 4_267_101_002],
        ["ada-uploads-bob-s-task", "bob-signs-for-ada"],
    );
    let (at, queue, bob_person) = (claims.at, claims.queue, claims.bob_person());
    let talk = &mut claims.senders.agents.talk;
    talk.played[0].take_in(&talk.world.cloud);
    let bob = &talk.played[0];
    let craft = Craft::new(&bob_person, queue, UPLOADED);
    let record = sealed_by(bob, at, (&bob.person.key, bob.device()), &craft);
    uploads(&talk.world.cloud, &talk.ada, at, &record);

    let shown = claims.codex_claims(&[]).shown();

    assert_refused(&claims, (record.task, UPLOADED), &shown);
}

/// Ada's first device writes a task whose content's `to` is codex while its `sched` names no
/// device, and one whose `sched` names the second machine's device while its content has no `to`.
/// Codex's claims show neither text and a warning naming each; the server gives both `dead` with
/// the reason `failed`.
#[test]
fn to_without_a_device_in_sched_and_the_reverse_are_failed_without_retry() {
    let mut claims = Claims::new(
        [4_267_102_001, 4_267_102_002],
        ["ada-mismatches-the-addressee", "bob-sees-the-mismatch"],
    );
    let (person, queue, codex) = (claims.person(), claims.queue, claims.codex());
    let laptop = claims.laptop().device;
    let mut to_only = Craft::new(&person, queue, TO_ONLY);
    to_only.to = Some(codex);
    let to_only = claims.ada_writes(&to_only).task;
    let mut device_only = Craft::new(&person, queue, DEVICE_ONLY);
    device_only.sched.device = Some(laptop);
    let device_only = claims.ada_writes(&device_only).task;

    let shown = claims.codex_claims_times(2, &[]);

    assert_refused(&claims, (to_only, TO_ONLY), &shown);
    assert_refused(&claims, (device_only, DEVICE_ONLY), &shown);
}

/// Ada's first device writes a task from claude whose content carries a grant it signed for the second
/// machine's device in the channel, though a `client` device's content carries none. Codex's claim
/// shows none of its text and a warning naming it; the server gives it `dead` with the reason
/// `failed`.
#[test]
fn client_sender_s_task_with_a_grant_is_failed_without_retry() {
    let mut claims = Claims::new(
        [4_267_103_001, 4_267_103_002],
        ["ada-grants-in-a-task", "bob-sees-the-grant-refused"],
    );
    let (claude, queue, channel) = (claims.claude(), claims.queue, claims.at.1);
    let ada = claims.ada();
    let body = GrantBody {
        service: claims.laptop().device,
        channels: NonEmpty::from_vec(vec![channel]).expect("a channel"),
        signer: ada.device,
        expires: now() + GRANT_HOLDS,
        unknown: Unknown::default(),
    };
    let signature = ada.key.sign("skald grant", &[&body.to_cbor()]);
    let mut granted = Craft::new(&claude, queue, GRANTED);
    granted.grant = Some(Grant { body, signature });
    let granted = claims.ada_writes(&granted).task;

    let shown = claims.codex_claims(&[]).shown();

    assert_refused(&claims, (granted, GRANTED), &shown);
}

/// Ada's first device writes a task for codex on the second machine. The person's claim there, and
/// then `gemini-p4r8vd`'s in the same folder, each get it and show none of its text, and the server
/// gives it `queued` with attempts 0 after each; the test ends the release's wait each time.
/// Codex's claim then gets it.
#[test]
fn task_for_one_agent_is_handed_back_when_the_person_or_another_agent_claims_it() {
    let mut claims = Claims::new(
        [4_267_104_001, 4_267_104_002],
        ["ada-asks-codex-alone", "bob-sees-codex-run-it"],
    );
    let (person, queue, codex) = (claims.person(), claims.queue, claims.codex());
    let mut craft = Craft::new(&person, queue, FOR_CODEX);
    craft.sched.device = Some(claims.laptop().device);
    craft.to = Some(codex);
    let task = claims.ada_writes(&craft).task;
    let channel = claims.channel.clone();
    let folder = claims.senders.laptop_folder.clone();

    for (generation, agent) in [(1, None), (2, Some(GEMINI))] {
        let ran = run_in(
            &claims.laptop().machine,
            (&folder, agent),
            CLAIM,
            &[CHANNEL, &channel],
        );
        assert!(ran.success, "{agent:?}'s claim: {ran:?}");
        assert!(
            !ran.shown().contains(FOR_CODEX),
            "handed to {agent:?}: {ran:?}"
        );
        let read = claims.read(task);
        assert_eq!(read.summary(), (QUEUED, None, 0), "{agent:?}: {read:?}");
        assert_eq!(read.held().generation, generation, "not claimed: {read:?}");
        (claims.cloud()).execute(&format!(
            "DELETE FROM task_waits WHERE task_id = '{task}'::uuid"
        ));
    }
    let claimed = claims.codex_claimed(&[]);

    assert!(
        claimed.shown().contains(FOR_CODEX),
        "not handed to {LAPTOP_AGENT}: {claimed:?}"
    );
}
