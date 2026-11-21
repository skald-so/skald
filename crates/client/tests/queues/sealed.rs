//! A task an agent or the person enqueues: uploaded once, sealed to the channel's current epoch
//! and signed by the device, with every field as "Tasks" says; the server gets only `sched` and
//! ciphertext.

use serde_json::json;
use skald_proto::QueueMode;

use crate::agents::{Agents, CLAUDE};
use crate::fixtures::CHANNEL;
use crate::queuing::{
    AT_MOST_ONCE, QUEUE_FLAG, Sending, assert_never_in_the_clear, assert_none_uploaded,
    assert_plain_record, assert_refused_naming, assert_says_reference, assert_task, creates_queue,
    default_queue, enqueue, enqueued, occurrences, opened, sending, task_uploads, tasks,
    the_one_task,
};
use crate::world::now;

/// What the tests enqueue; no text is anywhere else.
const AGENT_S: &str = "sealed task test: claude asks for the release notes of the last build";
const IN_DEFAULT: &str = "sealed task test: ada asks for a summary of the open incidents";
const IN_DEPLOYS: &str = "sealed task test: ada asks for a deploy of the billing service";
const NOWHERE: &str = "sealed task test: ada names a queue the channel lacks";

/// The state of a task waiting for a claim.
const QUEUED: &str = "queued";

/// A queue the channel lacks.
const NO_SUCH_QUEUE: &str = "nosuch";

/// As claude in `~/work/acme`, `skald enqueue --channel <channel> <text>` says the task's
/// `#<ref>`. The machine uploaded one task, once; the server holds it in the channel's `default`,
/// `queued`, byte for byte as uploaded, a record with no other field. It is sealed under the
/// channel's current epoch: the queue key of Bob's history key of that epoch opens it, and it is
/// signed by Ada's device, which it names as its sender, with the channel, the queue, its own ID,
/// the record's `sched`, the mode `at-least-once` and the time of enqueue. Its content is from
/// claude, with `hops` 0, no `to`, no `counter` and a `task` body, whose request is a `ROLE_USER`
/// message with the task's ID as `messageId` and the text as its one part. The text is in no byte
/// the machines sent and in no row of the server's database.
#[test]
fn agent_s_task_is_stored_once_sealed_to_the_current_epoch_with_every_field() {
    let mut agents = Agents::new(
        [4_265_011_001, 4_265_011_002],
        ["ada-enqueues-as-claude", "bob-opens-the-task"],
    );
    agents.run_as(CLAUDE);
    let (cloud, ada) = (agents.talk.cloud(), &agents.talk.ada);
    let Sending {
        at,
        channel,
        queue,
        device: ada_device,
        signing,
    } = sending(&agents.talk);
    let claude = agents.address(CLAUDE);
    let (since, before) = (cloud.relay.mark(), now());

    let ran = enqueued(
        &ada.machine,
        (&agents.folder, Some(CLAUDE)),
        &[CHANNEL, &channel, AGENT_S],
    );

    let after = now();
    let stored = the_one_task(cloud, ada, at, queue);
    assert_says_reference(&ran, stored.task.task);
    let uploads = task_uploads(cloud, since);
    assert_eq!(uploads.len(), 1, "not one upload: {uploads:?}");
    let sent = cloud.relay.sent_bytes();
    assert_eq!(
        occurrences(&sent, &stored.bytes),
        1,
        "the stored bytes not uploaded once"
    );
    assert_eq!(stored.state, QUEUED, "{stored:?}");
    assert_plain_record(&stored);
    let sched = &stored.task.sched;
    assert_eq!(
        (&sched.idempotency_key, &sched.template, sched.device),
        (&None, &None, None),
        "{sched:?}"
    );
    assert_never_in_the_clear(cloud, AGENT_S);
    let opened = opened(&mut agents.talk, at, (queue, &stored), &signing);
    let epoch = (agents.talk.played[0].mls.epoch(at.1)).expect("Bob's epoch");
    assert_eq!(
        stored.task.epoch, epoch,
        "not sealed under the current epoch"
    );
    assert_task(
        &opened,
        (at, queue, &stored),
        (ada_device, &claude, AGENT_S),
        (before, after),
    );
    assert_eq!(
        opened.body.mode,
        QueueMode::AtLeastOnce,
        "{:?}",
        opened.body
    );
    let content = opened.content();
    assert_eq!((content.hops, &content.to), (0, &None), "{content:?}");
}

/// Ada's device creates `deploys`, `at-most-once`, in the channel through the API. In
/// `~/work/acme`, the person's `skald enqueue <text>` puts a task in the `default` of `acme`'s
/// default channel, from Ada's person address, `at-least-once`, opening and verifying as claude's
/// does. `skald enqueue --channel <channel> --queue deploys <text>` puts one in `deploys`, with the
/// mode `at-most-once`. `skald enqueue --channel <channel> --queue nosuch <text>` fails naming
/// `nosuch`, uploads nothing, and the channel's queues hold no more tasks.
#[test]
fn person_enqueues_in_the_default_channel_or_a_named_queue() {
    let mut agents = Agents::new(
        [4_265_012_001, 4_265_012_002],
        ["ada-enqueues-herself", "bob-opens-ada-s-tasks"],
    );
    let talk = &agents.talk;
    let (cloud, ada, at) = (talk.cloud(), &talk.ada, talk.at());
    let in_default = (talk.space.id, talk.space.default);
    let from = talk.address(ada);
    let Sending {
        channel,
        device: ada_device,
        signing,
        ..
    } = sending(talk);
    let settings = json!({ "name": "deploys", "mode": AT_MOST_ONCE });
    let deploys = creates_queue(cloud, ada, at, &settings).id;
    let default = default_queue(cloud, ada, in_default);
    let person = (agents.folder.as_path(), None);
    let before = now();

    let ran = enqueued(&ada.machine, person, &[IN_DEFAULT]);

    let after = now();
    let stored = the_one_task(cloud, ada, in_default, default);
    assert_says_reference(&ran, stored.task.task);
    let opened_default = opened(&mut agents.talk, in_default, (default, &stored), &signing);
    assert_task(
        &opened_default,
        (in_default, default, &stored),
        (ada_device, &from, IN_DEFAULT),
        (before, after),
    );
    assert_eq!(opened_default.body.mode, QueueMode::AtLeastOnce);

    let talk = &agents.talk;
    let (cloud, ada) = (talk.cloud(), &talk.ada);
    let before = now();

    let ran = enqueued(
        &ada.machine,
        person,
        &[CHANNEL, &channel, QUEUE_FLAG, "deploys", IN_DEPLOYS],
    );

    let after = now();
    let stored = the_one_task(cloud, ada, at, deploys);
    assert_says_reference(&ran, stored.task.task);
    let opened_deploys = opened(&mut agents.talk, at, (deploys, &stored), &signing);
    assert_task(
        &opened_deploys,
        (at, deploys, &stored),
        (ada_device, &from, IN_DEPLOYS),
        (before, after),
    );
    assert_eq!(opened_deploys.body.mode, QueueMode::AtMostOnce);

    let talk = &agents.talk;
    let (cloud, ada) = (talk.cloud(), &talk.ada);
    let since = cloud.relay.mark();

    let refused = enqueue(
        &ada.machine,
        person,
        &[CHANNEL, &channel, QUEUE_FLAG, NO_SUCH_QUEUE, NOWHERE],
    );

    assert_refused_naming(&refused, &[NO_SUCH_QUEUE], "an unknown queue");
    assert_none_uploaded(cloud, since, "an unknown queue");
    let in_channel = default_queue(cloud, ada, at);
    assert!(
        tasks(cloud, ada, at, in_channel).is_empty(),
        "a task in default"
    );
    assert_eq!(
        tasks(cloud, ada, at, deploys).len(),
        1,
        "another task in deploys"
    );
}
