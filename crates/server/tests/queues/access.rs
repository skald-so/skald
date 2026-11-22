//! Who reads a queue's tasks: devices that may send in the channel, member devices and a service
//! token's device with `send` for the channel, each proving its key; nobody else.

use skald_proto::TaskId;

use crate::bot::ops;
use crate::kit::{DEFAULT, Given, UNKNOWN_TASK, World, assert_problem, queued};
use crate::presence::{assert_refused_with, assert_unproved};
use crate::tokens::{QUEUES, RECEIVE, SEND};

/// alice's laptop enqueues a task; bob's and carol's laptops get it, and so does the bot on a token
/// with `send` for `general`, in the list and on its own.
#[test]
fn member_devices_and_a_token_that_sends_in_the_channel_read_its_tasks() {
    let w = World::new(3_907_400_001);
    let [alice, bob, carol, ..] = w.laptops();
    let default = alice.queue(DEFAULT).id;
    let task = w.task(default);
    alice.stores(&task);

    let bot = w.bot(&[SEND]);
    for client in [&bob, &carol, &bot] {
        assert_eq!(client.tasks(default), queued(&[&task]), "{}", client.who);
        assert_eq!(
            client.task(default, task.task),
            Given::queued(&task),
            "{}",
            client.who
        );
    }
}

/// alice's laptop enqueues a task. Refused, in the list and on its own: dave's laptop, outside the
/// channel; eve's, outside the space; the bot on a token without `send`, and on one with `send`
/// for another channel alone; and, with `401` and `auth-required`, carol's session without her
/// laptop's proof and the bot's sending token alone.
#[test]
fn others_and_a_session_or_token_without_the_proof_read_no_task() {
    let mut w = World::new(3_907_400_011);
    let other = ops(&mut w.stage);
    let [alice, _, carol, dave, eve] = w.laptops();
    let default = alice.queue(DEFAULT).id;
    let task = w.task(default);
    alice.stores(&task);

    let no_send = w.bot(&[RECEIVE, QUEUES]);
    let other_only = w.bot_in(
        other,
        &[SEND, QUEUES],
        "the bot with send in another channel",
    );
    for client in [&dave, &eve, &no_send, &other_only] {
        assert_refused_with(&client.list_tasks(default, true), None, &client.who);
        assert_refused_with(&client.read(default, task.task, true), None, &client.who);
    }

    let sending = w.bot(&[SEND]);
    for client in [&carol, &sending] {
        assert_unproved(&client.list_tasks(default, false), &client.who);
        assert_unproved(&client.read(default, task.task, false), &client.who);
    }
}

/// A read of a task the queue does not have gets `unknown-task`.
#[test]
fn a_task_the_queue_does_not_have_is_unknown() {
    let w = World::new(3_907_400_021);
    let [alice, ..] = w.laptops();
    let default = alice.queue(DEFAULT).id;
    let response = alice.read(default, TaskId::generate(), true);
    assert_problem(&response, UNKNOWN_TASK, "a task of no queue");
}
