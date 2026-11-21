//! `skald tasks` shows each `dead` task with its reason: its lease expired, it failed, it was
//! released 5 times, or its deadline passed, one its user set or the 7 days a sending device sets
//! when another account may claim the task; `--queue` lists one queue's tasks.

use serde_json::json;
use skald_proto::{QueueId, TaskId};

use crate::agents::CLAUDE;
use crate::lines::{JSON_FLAG, json_objects};
use crate::queuing::{
    DEADLINE_FLAG, DEFAULT, LEASE, QUEUE_FLAG, WEEK, creates_queue, enqueued, referred, rfc3339,
    tasks,
};
use crate::states::{
    DEAD, DEADLINE_PASSED, FAILED, LEASE_EXPIRED, RELEASED, Scene, assert_listed,
    assert_not_listed, awaits, claims, fails, listed, object_of, passes, read, taken,
};
use crate::world::now;

/// The queues the test creates: one of a single attempt, and one each for a fail and releases.
const EXPIRING: &str = "expiring";
const FAILING: &str = "failing";
const RELEASING: &str = "releasing";

/// What claude enqueues; no text is anywhere else.
const LEASED: &str = "dead test: claude asks for a backup whose holder stops";
const BROKEN: &str = "dead test: claude asks for a build that fails";
const HANDED_BACK: &str = "dead test: claude asks for a task every claimer hands back";
const SET_DEADLINE: &str = "dead test: claude asks for a report by a set time";
const DEFAULT_DEADLINE: &str = "dead test: claude asks bob's devices too for a review";

/// Three days, in seconds: how far ahead the deadline the user sets is.
const THREE_DAYS: u64 = 3 * 24 * 60 * 60;

/// The reasons of a `dead` task.
const REASONS: [&str; 4] = [LEASE_EXPIRED, FAILED, RELEASED, DEADLINE_PASSED];

/// The reasons other than `reason`.
fn others(reason: &str) -> Vec<&'static str> {
    REASONS.into_iter().filter(|one| *one != reason).collect()
}

/// As claude in `~/work/acme`, Ada's machine puts five tasks: one in `expiring`, a queue of one
/// attempt, Ada's device created; one each in `failing` and `releasing`; and two in `default`, one
/// with `--deadline` three days ahead and one without, which gets the 7-day deadline as Bob may
/// claim it. Bob's device claims the first and lets its lease end, claims the second and fails it
/// without retry, and claims and releases the third 5 times, each time after its wait; three days
/// pass for the fourth and 7 days for the fifth. The person's `skald tasks --channel <channel>`
/// lists each `dead` with its reason, `lease-expired`, `failed`, `released`, `deadline-passed` and
/// `deadline-passed`, its queue and its attempts, 1, 1, 0, 0 and 0. `skald tasks --channel
/// <channel> --queue failing` lists only the failed task, and with `--json` gives its state, reason
/// and attempts.
#[test]
fn each_dead_task_shows_its_reason() {
    let scene = Scene::new(
        [7_402_002_001, 7_402_002_002],
        ["ada-reads-her-dlq", "bob-drops-ada-s-tasks"],
    );
    let (cloud, ada, bob, at, queue) = scene.parts();
    let person = scene.as_person();
    let expiring = creates_queue(
        cloud,
        ada,
        at,
        &json!({ "name": EXPIRING, "max_attempts": 1 }),
    )
    .id;
    let failing = creates_queue(cloud, ada, at, &json!({ "name": FAILING })).id;
    let releasing = creates_queue(cloud, ada, at, &json!({ "name": RELEASING })).id;
    let enqueues = |(name, id): (&str, QueueId), args: &[&str]| -> TaskId {
        let mut all = scene.in_channel(&[QUEUE_FLAG, name]);
        all.extend_from_slice(args);
        let ran = enqueued(scene.machine(), scene.as_agent(CLAUDE), &all);
        referred(&tasks(cloud, ada, at, id), &ran).task.task
    };
    let leased = enqueues((EXPIRING, expiring), &[LEASED]);
    let broken = enqueues((FAILING, failing), &[BROKEN]);
    let handed_back = enqueues((RELEASING, releasing), &[HANDED_BACK]);
    let time = rfc3339(now() + THREE_DAYS);
    let set = enqueues((DEFAULT, queue), &[DEADLINE_FLAG, &time, SET_DEADLINE]);
    let week = enqueues((DEFAULT, queue), &[DEFAULT_DEADLINE]);

    let held = claims(cloud, bob, at, expiring);
    assert_eq!(held.task, leased, "Bob's device claimed another task");
    passes(cloud, leased, LEASE + 1);
    let now_leased = awaits(cloud, ada, (at, expiring), (leased, DEAD));
    assert_eq!(now_leased.shows(), (DEAD, 1, None, Some(LEASE_EXPIRED)));

    let held = claims(cloud, bob, at, failing);
    fails(cloud, bob, &held, false);
    let now_broken = read(cloud, ada, (at, failing), broken);
    assert_eq!(now_broken.shows(), (DEAD, 1, None, Some(FAILED)));

    for n in 1..=5 {
        let held = claims(cloud, bob, at, releasing);
        taken(cloud, bob, &held, ("release", json!({})));
        if n < 5 {
            passes(cloud, handed_back, (10 << (n - 1)) + 1);
        }
    }
    let now_handed_back = read(cloud, ada, (at, releasing), handed_back);
    assert_eq!(now_handed_back.shows(), (DEAD, 0, None, Some(RELEASED)));

    passes(cloud, set, THREE_DAYS + 1);
    passes(cloud, week, WEEK + 1);
    for task in [set, week] {
        let past = awaits(cloud, ada, (at, queue), (task, DEAD));
        assert_eq!(past.shows(), (DEAD, 0, None, Some(DEADLINE_PASSED)));
    }

    let shown = listed(scene.machine(), person, &scene.in_channel(&[]));

    for (task, reason, name, attempts) in [
        (leased, LEASE_EXPIRED, EXPIRING, "1"),
        (broken, FAILED, FAILING, "1"),
        (handed_back, RELEASED, RELEASING, "0"),
        (set, DEADLINE_PASSED, DEFAULT, "0"),
        (week, DEADLINE_PASSED, DEFAULT, "0"),
    ] {
        assert_listed(
            &shown,
            task,
            &[DEAD, reason, name, attempts],
            &others(reason),
        );
    }

    let only = listed(
        scene.machine(),
        person,
        &scene.in_channel(&[QUEUE_FLAG, FAILING]),
    );

    assert_listed(&only, broken, &[DEAD, FAILED, FAILING, "1"], &[]);
    for task in [leased, handed_back, set, week] {
        assert_not_listed(&only, task);
    }

    let json = listed(
        scene.machine(),
        person,
        &scene.in_channel(&[QUEUE_FLAG, FAILING, JSON_FLAG]),
    );

    let object = object_of(&json, broken);
    assert_eq!(
        json_objects(&json.stdout).len(),
        1,
        "more than the failing queue's task: {json:?}"
    );
    assert_eq!(object["queue_name"].as_str(), Some(FAILING), "{object}");
    assert_eq!(object["state"].as_str(), Some(DEAD), "{object}");
    assert_eq!(object["reason"].as_str(), Some(FAILED), "{object}");
    assert_eq!(object["attempts"].as_u64(), Some(1), "{object}");
}
