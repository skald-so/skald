//! Restart and cancel: a `client` device of the uploader's account or of an admin restarts a `dead`
//! task and cancels a `queued`, `claimed` or `dead` one; a member who is neither and a `service`
//! device get `not-allowed`; a restart of a task that is not `dead` gets `not-dead`, a cancel of a
//! `done` or `cancelled` one `not-cancellable`. The holder of a cancelled claim gets `claim-lost`.

use skald_proto::{DeviceKind, QueueId, QueuedTask, TaskId};

use crate::holder::{
    ALL, CANCELLED, CLAIMED, DEAD, DONE, FAILED, Holder, NOT_CANCELLABLE, NOT_DEAD, STARTED,
    assert_claim_lost, assert_taken, client_of, device_of, enqueued, takes,
};
use crate::kit::{Client, DEFAULT, NOT_ALLOWED, QUEUED, UNKNOWN_TASK, World, assert_problem};
use crate::tokens::{QUEUES, RECEIVE, SEND};

/// `claimer` claims the first task of `queue`, which must be `task`, and fails it without retry.
#[track_caller]
fn dies(claimer: &Client, queue: QueueId, task: &QueuedTask) {
    let claim = claimer.claims(queue, &takes(&ALL));
    assert_eq!(claim.task.task, task.task, "the task to fail");
    assert_taken(&claimer.fail(&claim, false), "the fail");
}

/// carol's laptop enqueues three tasks. bob's laptop fails the first and second without retry.
/// carol's desktop, another device of her account, restarts the first, and alice's laptop, an
/// admin's, the second: each `queued` with attempts 0. carol's desktop cancels the first,
/// `queued`; alice's laptop cancels the second once bob's laptop holds it, and bob's extend and
/// complete then get `claim-lost`; bob fails the third, which alice's laptop cancels while `dead`.
/// Each is `cancelled`, and no claim gets one.
#[test]
fn the_uploaders_other_device_and_an_admin_restart_and_cancel_its_tasks() {
    let w = World::new(3_926_600_001);
    let [alice, bob, carol, ..] = w.laptops();
    let default = alice.queue(DEFAULT).id;
    let team = &w.fixture().team;
    let desktop = device_of(&w, &team.carol, DeviceKind::Client, "Desktop");
    let desktop = client_of(&w, w.server(), &team.carol, &desktop);
    let [first, second, third] = [(), (), ()].map(|()| {
        let task = w.task(default);
        carol.stores(&task);
        task
    });

    dies(&bob, default, &first);
    dies(&bob, default, &second);
    for (client, task) in [(&desktop, &first), (&alice, &second)] {
        assert_taken(&client.restart(default, task.task), &client.who);
        let read = alice.state_of(default, task.task);
        assert_eq!(
            read.shows(),
            (QUEUED, 0, None, None),
            "{}'s restart",
            client.who
        );
    }

    assert_taken(
        &desktop.cancel(default, first.task),
        "the cancel of a queued task",
    );
    let held = bob.claims(default, &takes(&ALL));
    assert_eq!(held.task.task, second.task, "bob holds the second");
    assert_taken(
        &alice.cancel(default, second.task),
        "the cancel of a claimed task",
    );
    assert_claim_lost(&bob.extend(&held, STARTED), "the holder's extend");
    assert_claim_lost(&bob.complete(&held), "the holder's complete");
    dies(&bob, default, &third);
    assert_taken(
        &alice.cancel(default, third.task),
        "the cancel of a dead task",
    );

    for task in [&first, &second, &third] {
        assert_eq!(alice.state_of(default, task.task).state, CANCELLED);
    }
    bob.gets_none(default, &takes(&ALL));
}

/// carol's laptop enqueues a task that bob's laptop fails without retry. bob's laptop, of a member
/// who is no admin, and the bot, a `service` device of alice's account, get `not-allowed` for a
/// restart and a cancel; reads show the task still `dead` with `failed`.
#[test]
fn a_member_who_is_neither_and_a_service_device_may_not_restart_or_cancel() {
    let w = World::new(3_926_600_011);
    let [alice, bob, carol, ..] = w.laptops();
    let default = alice.queue(DEFAULT).id;
    let task = w.task(default);
    carol.stores(&task);
    dies(&bob, default, &task);
    let bot = w.bot(&[SEND, RECEIVE, QUEUES]);

    for client in [&bob, &bot] {
        let restart = client.restart(default, task.task);
        assert_problem(&restart, NOT_ALLOWED, &format!("{}'s restart", client.who));
        let cancel = client.cancel(default, task.task);
        assert_problem(&cancel, NOT_ALLOWED, &format!("{}'s cancel", client.who));
    }
    let read = alice.state_of(default, task.task);
    assert_eq!(
        read.shows(),
        (DEAD, 1, None, Some(FAILED)),
        "after the refusals"
    );
}

/// alice's laptop restarts a `queued`, a `claimed`, a `done` and a `cancelled` task: `not-dead`;
/// cancels the `done` and the `cancelled` one: `not-cancellable`; reads show each as before. A
/// restart and a cancel of a task the queue does not have get `unknown-task`.
#[test]
fn a_restart_of_a_task_not_dead_or_a_cancel_of_a_finished_one_is_refused() {
    let w = World::new(3_926_600_021);
    let [alice, _, carol, ..] = w.laptops();
    let default = alice.queue(DEFAULT).id;
    let [done, claimed, cancelled, queued] = enqueued(&w, &alice, default);
    let claim = carol.claims(default, &takes(&ALL));
    assert_taken(&carol.complete(&claim), "the complete");
    carol.claims(default, &takes(&ALL));
    assert_taken(&alice.cancel(default, cancelled.task), "the cancel");

    let expected = [
        (&done, DONE),
        (&claimed, CLAIMED),
        (&cancelled, CANCELLED),
        (&queued, QUEUED),
    ];
    let before: Vec<_> = (expected.iter())
        .map(|(task, state)| {
            let read = alice.state_of(default, task.task);
            assert_eq!(read.state, *state, "the task {state}");
            read
        })
        .collect();
    for (task, state) in expected {
        let response = alice.restart(default, task.task);
        assert_problem(&response, NOT_DEAD, &format!("a restart of a {state} task"));
    }
    for (task, state) in [(&done, DONE), (&cancelled, CANCELLED)] {
        let response = alice.cancel(default, task.task);
        assert_problem(
            &response,
            NOT_CANCELLABLE,
            &format!("a cancel of a {state} task"),
        );
    }
    let after: Vec<_> = (expected.iter())
        .map(|(task, _)| alice.state_of(default, task.task))
        .collect();
    assert_eq!(after, before, "after the refusals");

    for action in ["restart", "cancel"] {
        let response = alice.on_task(default, TaskId::generate(), action, true);
        assert_problem(&response, UNKNOWN_TASK, &format!("a {action} of no task"));
    }
}
