//! Enqueue: the server stores a task only from a device that may send in the channel and, for a
//! `service` device, whose token allows the queue; once per `task` and per `idempotency_key` in its
//! queue; under the channel's current or previous epoch; with a `ct` within the message text limit;
//! byte for byte, `queued` with attempts 0. A repeated upload of a `task` from its device gets the
//! first answer, whatever its bytes.

use serde_json::json;
use skald_proto::{Cbor, QueueId};

use crate::kit::{
    Client, DEFAULT, EPOCH_MISMATCH, Given, NOT_A_MEMBER, SIZE_LIMIT, TOO_LARGE, UNKNOWN_QUEUE,
    World, assert_duplicate, assert_not_held, assert_problem, assert_stored, keyed, named, queued,
    task,
};
use crate::presence::{assert_refused_with, assert_unproved, held, random_bytes};
use crate::rosters::with_later_field;
use crate::tokens::{QUEUES, RECEIVE, SEND};

/// The world, with carol's queue `builds`: the laptops, as [`World::laptops`] gives them, and the
/// IDs of `default` and `builds`.
fn with_builds(github: u64) -> (World, [Client; 5], QueueId, QueueId) {
    let w = World::new(github);
    let laptops = w.laptops();
    let default = laptops[0].queue(DEFAULT).id;
    let builds = laptops[2].creates(&named("builds", json!({}))).id;
    (w, laptops, default, builds)
}

/// alice's laptop enqueues a task with an `idempotency_key` and a field of a later version, and
/// carol's one without: carol's laptop gets both in `default`, byte for byte, `queued` with
/// attempts 0, in the list and each on its own.
#[test]
fn a_member_devices_task_is_stored_byte_for_byte_and_given_queued() {
    let w = World::new(3_907_300_001);
    let [alice, _, carol, ..] = w.laptops();
    let default = alice.queue(DEFAULT).id;

    let alices = keyed(w.task(default), b"build 1");
    let later = with_later_field(&alices.to_cbor());
    assert_stored(
        &alice.upload(default, &later, true),
        "a task with a later field",
    );
    let carols = w.task(default);
    carol.stores(&carols);

    let mut expected = vec![Given::queued_bytes(&later), Given::queued(&carols)];
    expected.sort_by(|a, b| a.bytes.cmp(&b.bytes));
    assert_eq!(carol.tasks(default), expected, "the tasks of default");
    assert_eq!(
        carol.task(default, alices.task),
        Given::queued_bytes(&later)
    );
    assert_eq!(alice.task(default, carols.task), Given::queued(&carols));
}

/// A retry of alice's upload, with the same `task` and other bytes, gets the first answer, and so
/// does one with the same bytes; the server keeps the first bytes alone.
#[test]
fn a_repeated_upload_from_its_device_gets_the_first_answer_and_stores_nothing() {
    let w = World::new(3_907_300_011);
    let [alice, _, carol, ..] = w.laptops();
    let default = alice.queue(DEFAULT).id;
    let first = keyed(w.task(default), b"deploy");
    let answer = alice.stores(&first);

    let mut other = keyed(w.task(default), b"another key");
    other.task = first.task;
    for (retry, what) in [(&other, "other bytes"), (&first, "the same bytes")] {
        let again = alice.enqueue(retry);
        let found = (again.status, again.content_type.clone(), again.body.clone());
        let expected = (
            answer.status,
            answer.content_type.clone(),
            answer.body.clone(),
        );
        assert_eq!(found, expected, "a retry with {what}: the first answer");
    }
    assert_not_held(w.db(), &other, "the retry's bytes");
    assert_eq!(carol.tasks(default), queued(&[&first]), "the first alone");
}

/// Refused, storing nothing: dave's laptop, outside the channel, with `not-a-member`; eve's,
/// outside the space; the bot on a token without `queues` or without `send`, with `not-a-member`.
/// On a token with both, the bot's task is stored, and carol's laptop gets it alone.
#[test]
fn a_device_outside_the_channel_or_a_token_without_queues_enqueues_nothing() {
    let w = World::new(3_907_300_021);
    let [alice, _, carol, dave, eve] = w.laptops();
    let default = alice.queue(DEFAULT).id;

    let [no_queues, no_send] = [[SEND, RECEIVE], [RECEIVE, QUEUES]].map(|perms| w.bot(&perms));
    let refused = [
        (&dave, Some(NOT_A_MEMBER)),
        (&eve, None),
        (&no_queues, Some(NOT_A_MEMBER)),
        (&no_send, Some(NOT_A_MEMBER)),
    ];
    for (client, problem) in refused {
        let attempt = w.task(default);
        assert_refused_with(&client.enqueue(&attempt), problem, &client.who);
        assert_not_held(w.db(), &attempt, &client.who);
    }

    let bot = w.bot(&[SEND, QUEUES]);
    let bots = w.task(default);
    bot.stores(&bots);
    assert_eq!(carol.tasks(default), queued(&[&bots]), "the bot's alone");
}

/// carol's laptop uploads a task with the ID of alice's, and one with the `idempotency_key` of
/// alice's, each refused with `duplicate` naming alice's task, and so is alice's own next task with
/// that key. The same key is taken in another queue.
#[test]
fn a_task_id_another_device_used_or_an_idempotency_key_the_queue_has_is_a_duplicate() {
    let (w, [alice, _, carol, ..], default, builds) = with_builds(3_907_300_031);
    let first = keyed(w.task(default), b"release 1.0");
    alice.stores(&first);

    let mut same_id = w.task(default);
    same_id.task = first.task;
    let same_key = keyed(w.task(default), b"release 1.0");
    let alices_next = keyed(w.task(default), b"release 1.0");
    for (client, attempt, what) in [
        (&carol, &same_id, "carol's with alice's task ID"),
        (&carol, &same_key, "carol's with alice's key"),
        (&alice, &alices_next, "alice's next with her key"),
    ] {
        assert_duplicate(&client.enqueue(attempt), first.task, what);
        assert_not_held(w.db(), attempt, what);
    }
    assert_eq!(carol.tasks(default), queued(&[&first]), "the first alone");

    let elsewhere = keyed(w.task(builds), b"release 1.0");
    carol.stores(&elsewhere);
    assert_eq!(
        alice.tasks(builds),
        queued(&[&elsewhere]),
        "the key in builds"
    );
}

/// Once the channel is at epoch 3, tasks under epochs 2 and 3 are stored, and those under epoch 1,
/// before the previous one, and epoch 4, after the current one, refused with `epoch-mismatch`.
#[test]
fn a_task_under_an_epoch_other_than_the_current_or_previous_is_refused() {
    let mut w = World::new(3_907_300_041);
    w.rotate();
    assert_eq!(w.rotate(), 3, "the epoch after two Commits");
    let [alice, _, carol, ..] = w.laptops();
    let default = alice.queue(DEFAULT).id;

    for epoch in [1, 4] {
        let attempt = task(default, epoch);
        let what = format!("epoch {epoch}");
        assert_problem(&alice.enqueue(&attempt), EPOCH_MISMATCH, &what);
        assert_not_held(w.db(), &attempt, &what);
    }
    let [previous, current] = [2, 3].map(|epoch| task(default, epoch));
    alice.stores(&previous);
    alice.stores(&current);
    let expected = queued(&[&previous, &current]);
    assert_eq!(carol.tasks(default), expected, "epochs 2 and 3");
}

/// With the message text limit at 20000 bytes, a task whose `ct` is 20001 bytes is refused with
/// `too-large`; one of 20000 bytes is stored.
#[test]
fn a_ct_over_the_message_text_limit_is_too_large() {
    let w = World::with(3_907_300_051, &[SIZE_LIMIT, "20000"]);
    let [alice, _, carol, ..] = w.laptops();
    let default = alice.queue(DEFAULT).id;

    let mut over = w.task(default);
    over.ct = random_bytes(20_001);
    assert_problem(&alice.enqueue(&over), TOO_LARGE, "a ct of 20001 bytes");
    assert_not_held(w.db(), &over, "a ct of 20001 bytes");
    let mut longest = w.task(default);
    longest.ct = random_bytes(20_000);
    alice.stores(&longest);
    assert_eq!(carol.tasks(default), queued(&[&longest]), "the longest ct");
}

/// Refused, storing nothing: a task to a queue the channel does not have, with `unknown-queue`;
/// bytes that are no CBOR, a task cut short, and a task that names another queue than the
/// request's.
#[test]
fn an_unknown_queue_and_a_record_that_does_not_fit_are_refused() {
    let (w, [alice, _, carol, ..], default, builds) = with_builds(3_907_300_061);

    let nowhere = w.task(QueueId::generate());
    assert_problem(&alice.enqueue(&nowhere), UNKNOWN_QUEUE, "an unknown queue");
    assert_not_held(w.db(), &nowhere, "an unknown queue");

    let cut = w.task(default);
    let bytes = cut.to_cbor();
    let in_builds = w.task(builds);
    let cases = [
        (random_bytes(64), "bytes that are no CBOR"),
        (bytes[..bytes.len() - 9].to_vec(), "a task cut short"),
        (in_builds.to_cbor(), "a task of builds"),
    ];
    for (body, what) in &cases {
        assert_refused_with(&alice.upload(default, body, true), None, what);
        assert!(!held(w.db(), body), "{what}: kept");
    }
    assert_not_held(w.db(), &in_builds, "a task of builds");
    assert_eq!(carol.tasks(default), [], "default after the refusals");
    assert_eq!(carol.tasks(builds), [], "builds after the refusals");
}

/// carol's session without her laptop's proof enqueues nothing, and neither does the bot's token
/// with `send` and `queues` alone: `401` with `auth-required`.
#[test]
fn a_session_or_token_without_the_devices_proof_enqueues_nothing() {
    let w = World::new(3_907_300_071);
    let [alice, _, carol, ..] = w.laptops();
    let default = alice.queue(DEFAULT).id;
    let bot = w.bot(&[SEND, QUEUES]);

    for client in [&carol, &bot] {
        let attempt = w.task(default);
        let response = client.upload(default, &attempt.to_cbor(), false);
        assert_unproved(&response, &client.who);
        assert_not_held(w.db(), &attempt, &client.who);
    }
    assert_eq!(alice.tasks(default), [], "default after the refusals");
}
