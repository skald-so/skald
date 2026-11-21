//! The person restarts a `dead` task and cancels a task in the CLI: a restart makes it `queued` with
//! attempts 0, a cancel of a claimed one makes its holder's complete get `claim-lost`; the server's
//! `not-dead` and `not-cancellable` are shown.

use serde_json::json;

use crate::agents::CLAUDE;
use crate::queuing::{DEFAULT, enqueued, names_problem, run_in, the_one_task};
use crate::states::{
    CANCEL_TASK, CANCELLED, CLAIM_LOST, CLAIMED, DEAD, DONE, FAILED, NOT_CANCELLABLE, NOT_DEAD,
    QUEUED, RESTART_TASK, STARTED, Scene, acts, assert_listed, claims, extends, fails, listed,
    problem, read, reference_of, taken,
};

/// What claude enqueues; no text is anywhere else.
const RETRIED: &str = "restart test: claude asks for a deploy that failed once";
const DROPPED: &str = "cancel test: claude asks for a migration it no longer needs";
const FINISHED: &str = "cancel test: claude asks for a lint run that is over";

/// As claude in `~/work/acme`, Ada's machine puts a task, which Bob's device claims and fails
/// without retry: it is `dead` with `failed` and attempts 1. In `~/work/acme`, the person's `skald
/// restart-task --channel <channel> #<ref>`, the reference `skald enqueue` said, makes it `queued`
/// with attempts 0, and `skald tasks --channel <channel>` lists it so. The same restart again fails
/// naming `not-dead`, and the task stays `queued`. Bob's device then claims it again.
#[test]
fn person_restarts_a_dead_task_and_a_restart_of_a_queued_one_shows_not_dead() {
    let scene = Scene::new(
        [7_402_003_001, 7_402_003_002],
        ["ada-restarts-her-deploy", "bob-fails-ada-s-deploy"],
    );
    let (cloud, ada, bob, at, queue) = scene.parts();
    let person = scene.as_person();
    let ran = enqueued(
        scene.machine(),
        scene.as_agent(CLAUDE),
        &scene.in_channel(&[RETRIED]),
    );
    let task = the_one_task(cloud, ada, at, queue).task.task;
    let reference = reference_of(&ran, task);
    let held = claims(cloud, bob, at, queue);
    fails(cloud, bob, &held, false);
    let before = read(cloud, ada, (at, queue), task);
    assert_eq!(before.shows(), (DEAD, 1, None, Some(FAILED)));

    let restarted = run_in(
        scene.machine(),
        person,
        RESTART_TASK,
        &scene.in_channel(&[&reference]),
    );

    assert!(restarted.success, "{restarted:?}");
    let after = read(cloud, ada, (at, queue), task);
    assert_eq!(after.shows(), (QUEUED, 0, None, None), "not restarted");
    let shown = listed(scene.machine(), person, &scene.in_channel(&[]));
    assert_listed(&shown, task, &[QUEUED, DEFAULT, "0"], &[DEAD, FAILED]);

    let again = run_in(
        scene.machine(),
        person,
        RESTART_TASK,
        &scene.in_channel(&[&reference]),
    );

    assert!(!again.success, "a queued task restarted: {again:?}");
    assert!(names_problem(&again, NOT_DEAD), "{again:?}");
    let kept = read(cloud, ada, (at, queue), task);
    assert_eq!(kept.shows(), (QUEUED, 0, None, None));
    let claimed = claims(cloud, bob, at, queue);
    assert_eq!(claimed.task, task, "Bob's device claimed another task");
    assert!(claimed.generation > held.generation, "{claimed:?}");
}

/// As claude in `~/work/acme`, Ada's machine puts a task, which Bob's device claims and extends with
/// `started`. In `~/work/acme`, the person's `skald cancel-task --channel <channel> <task ID>` makes
/// it `cancelled`; Bob's device's complete then gets `claim-lost`, and `skald tasks --channel
/// <channel>` lists it `cancelled`. A second task, which Bob's device claims and completes: `skald
/// cancel-task --channel <channel> #<ref>` fails naming `not-cancellable`, and it stays `done`.
#[test]
fn person_cancels_a_claimed_task_and_its_holder_s_complete_gets_claim_lost() {
    let scene = Scene::new(
        [7_402_004_001, 7_402_004_002],
        ["ada-cancels-her-migration", "bob-holds-ada-s-migration"],
    );
    let (cloud, ada, bob, at, queue) = scene.parts();
    let person = scene.as_person();
    enqueued(
        scene.machine(),
        scene.as_agent(CLAUDE),
        &scene.in_channel(&[DROPPED]),
    );
    let task = the_one_task(cloud, ada, at, queue).task.task;
    let held = claims(cloud, bob, at, queue);
    extends(cloud, bob, &held, STARTED);
    let id = task.to_string();

    let cancelled = run_in(
        scene.machine(),
        person,
        CANCEL_TASK,
        &scene.in_channel(&[&id]),
    );

    assert!(cancelled.success, "{cancelled:?}");
    assert_eq!(read(cloud, ada, (at, queue), task).state, CANCELLED);
    let (status, body) = acts(cloud, bob, &held, ("complete", json!({})));
    assert_eq!(
        (status, problem(&body)),
        (409, CLAIM_LOST),
        "the holder's complete after the cancel: {body}"
    );
    assert_eq!(read(cloud, ada, (at, queue), task).state, CANCELLED);
    let shown = listed(scene.machine(), person, &scene.in_channel(&[]));
    assert_listed(&shown, task, &[CANCELLED, DEFAULT], &[CLAIMED, DONE]);

    let ran = enqueued(
        scene.machine(),
        scene.as_agent(CLAUDE),
        &scene.in_channel(&[FINISHED]),
    );
    let held = claims(cloud, bob, at, queue);
    let done = held.task;
    let reference = reference_of(&ran, done);
    taken(cloud, bob, &held, ("complete", json!({})));

    let refused = run_in(
        scene.machine(),
        person,
        CANCEL_TASK,
        &scene.in_channel(&[&reference]),
    );

    assert!(!refused.success, "a done task cancelled: {refused:?}");
    assert!(names_problem(&refused, NOT_CANCELLABLE), "{refused:?}");
    assert_eq!(read(cloud, ada, (at, queue), done).state, DONE);
}
