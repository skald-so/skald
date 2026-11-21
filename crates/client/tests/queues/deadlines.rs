//! The `deadline` a sending device sets: 7 days after enqueue when its user sets none and a device
//! of another account may claim the task, none when only the account's devices may; one the user
//! sets is kept.

use crate::agents::{Agents, CLAUDE};
use crate::fixtures::CHANNEL;
use crate::kit::{received, with_channel};
use crate::queuing::{
    DEADLINE_FLAG, Sending, assert_says_reference, assert_week_after, default_queue, enqueued,
    opened, referred, rfc3339, sending, tasks, the_one_task,
};
use crate::world::now;

/// What claude enqueues; no text is anywhere else.
const FOR_BOB_TOO: &str = "deadline test: claude asks anyone in the channel for a code review";
const SET_BY_USER: &str = "deadline test: claude asks for the review by a set time";
const ADA_ALONE: &str = "deadline test: claude asks only ada's devices to rotate the logs";

/// Three days, in seconds: how far ahead the deadline the user sets is.
const THREE_DAYS: u64 = 3 * 24 * 60 * 60;

/// As claude in `~/work/acme`, `skald enqueue --channel <channel> <text>` in the channel of Ada's
/// and Bob's accounts puts a task whose `sched` has a `deadline` a week after its time of enqueue,
/// as its sealed body states it. `skald enqueue --channel <channel> --deadline <time> <text>`,
/// `<time>` three days ahead, puts one whose `deadline` is that time.
#[test]
fn task_another_account_may_claim_gets_a_deadline_seven_days_after_enqueue() {
    let mut agents = Agents::new(
        [4_265_041_001, 4_265_041_002],
        ["ada-enqueues-for-bob-too", "bob-may-claim-it"],
    );
    agents.run_as(CLAUDE);
    let claude = (agents.folder.as_path(), Some(CLAUDE));
    let Sending {
        at,
        channel,
        queue,
        signing,
        ..
    } = sending(&agents.talk);
    let (cloud, ada) = (agents.talk.cloud(), &agents.talk.ada);
    let before = now();

    let ran = enqueued(&ada.machine, claude, &[CHANNEL, &channel, FOR_BOB_TOO]);

    let after = now();
    let stored = the_one_task(cloud, ada, at, queue);
    assert_says_reference(&ran, stored.task.task);
    let opened = opened(&mut agents.talk, at, (queue, &stored), &signing);
    assert_eq!(opened.body.sched, stored.task.sched, "another sched sealed");
    assert_week_after(
        stored.task.sched.deadline,
        opened.body.enqueued,
        (before, after),
    );

    let talk = &agents.talk;
    let (cloud, ada) = (talk.cloud(), &talk.ada);
    let deadline = now() + THREE_DAYS;
    let time = rfc3339(deadline);

    let ran = enqueued(
        &ada.machine,
        claude,
        &[CHANNEL, &channel, DEADLINE_FLAG, &time, SET_BY_USER],
    );

    let set = referred(&tasks(cloud, ada, at, queue), &ran);
    assert_eq!(
        set.task.sched.deadline,
        Some(deadline),
        "not the deadline {time}: {:?}",
        set.task.sched
    );
}

/// A revision Ada's device signs adds a channel whose only member is Ada; her machine forms its
/// group. As claude in `~/work/acme`, `skald enqueue --channel <that channel> <text>` puts a task
/// there whose `sched` has no `deadline`.
#[test]
fn task_only_the_account_s_devices_may_claim_gets_no_deadline() {
    let agents = Agents::new(
        [4_265_042_001, 4_265_042_002],
        ["ada-enqueues-for-herself", "bob-outside-her-channel"],
    );
    agents.run_as(CLAUDE);
    let talk = &agents.talk;
    let (cloud, ada) = (talk.cloud(), &talk.ada);
    let solo = with_channel(cloud, talk.space.id, ada, &[]);
    received(&ada.machine);
    let at = (talk.space.id, solo);
    let queue = default_queue(cloud, ada, at);
    let channel = solo.to_string();

    let ran = enqueued(
        &ada.machine,
        (&agents.folder, Some(CLAUDE)),
        &[CHANNEL, &channel, ADA_ALONE],
    );

    let stored = the_one_task(cloud, ada, at, queue);
    assert_says_reference(&ran, stored.task.task);
    assert_eq!(
        stored.task.sched.deadline, None,
        "a deadline in {:?}",
        stored.task.sched
    );
}
