//! `skald tasks` as the person: a task `queued`, then `claimed` with the progress of its holder's
//! last extend, `started`, then `awaiting-approval`, then `done`; with `--json`, every field.

use serde_json::json;

use crate::agents::CLAUDE;
use crate::lines::{JSON_FLAG, holds_text, refers_to};
use crate::queuing::{DEFAULT, as_message, enqueued, refers_to_task, the_one_task};
use crate::states::{
    AWAITING_APPROVAL, CLAIMED, DONE, QUEUED, STARTED, Scene, assert_listed, claims, extends,
    line_of, listed, object_of, read, taken,
};

/// What claude enqueues; no text is anywhere else.
const REVIEW: &str = "listing test: claude asks for a review of the parser";

/// As claude in `~/work/acme`, `skald enqueue --channel <channel> <text>` puts a task. In
/// `~/work/acme`, the person's `skald tasks --channel <channel>` lists it `queued` in `default` with
/// attempts 0, on a line holding its `#<ref>` and its ID. Bob's device claims it and extends it
/// with `started`: the line says `claimed`, `started` and attempts 1, and `skald tasks --channel
/// <channel> --json` gives one object for it with its ID, `#<ref>`, channel, queue ID and name,
/// state, progress, attempts and Bob's device as its holder. Bob's device extends it with
/// `awaiting-approval`: the line says `awaiting-approval` and not `started`. Bob's device completes
/// it: the line says `done`.
#[test]
fn person_sees_a_task_queued_then_claimed_with_its_holder_s_progress_then_done() {
    let scene = Scene::new(
        [7_402_001_001, 7_402_001_002],
        ["ada-follows-her-tasks", "bob-works-on-ada-s-task"],
    );
    let (cloud, ada, bob, at, queue) = scene.parts();
    let person = scene.as_person();
    let in_channel = scene.in_channel(&[]);
    enqueued(
        scene.machine(),
        scene.as_agent(CLAUDE),
        &scene.in_channel(&[REVIEW]),
    );
    let task = the_one_task(cloud, ada, at, queue).task.task;

    let shown = listed(scene.machine(), person, &in_channel);

    assert_listed(
        &shown,
        task,
        &[QUEUED, DEFAULT, "0"],
        &[CLAIMED, STARTED, AWAITING_APPROVAL],
    );
    let line = line_of(&shown, task);
    assert!(
        line.contains(&task.to_string().to_lowercase()) && refers_to_task(&line, task),
        "the person's line holds no ID or #<ref> of {task}: {line:?}"
    );

    let held = claims(cloud, bob, at, queue);
    assert_eq!(held.task, task, "Bob's device claimed another task");
    extends(cloud, bob, &held, STARTED);
    let now = read(cloud, ada, (at, queue), task);
    assert_eq!(now.shows(), (CLAIMED, 1, Some(STARTED), None));

    let shown = listed(scene.machine(), person, &in_channel);

    assert_listed(
        &shown,
        task,
        &[CLAIMED, STARTED, DEFAULT, "1"],
        &[QUEUED, AWAITING_APPROVAL],
    );

    let json = listed(scene.machine(), person, &scene.in_channel(&[JSON_FLAG]));

    let object = object_of(&json, task);
    let short = object["ref"].as_str().unwrap_or_default();
    let short = short.strip_prefix('#').unwrap_or(short);
    let as_message = as_message(task);
    assert!(
        refers_to(short, as_message),
        "a ref not of {task}: {object}"
    );
    let queue_id = queue.to_string();
    for (field, value) in [
        ("channel", scene.channel.as_str()),
        ("queue", queue_id.as_str()),
        ("queue_name", DEFAULT),
        ("state", CLAIMED),
        ("progress", STARTED),
    ] {
        assert_eq!(object[field].as_str(), Some(value), "{field} in {object}");
    }
    assert_eq!(object["attempts"].as_u64(), Some(1), "{object}");
    assert!(
        holds_text(&object, &bob.device.to_string()),
        "no holder's device in {object}"
    );

    extends(cloud, bob, &held, AWAITING_APPROVAL);

    let shown = listed(scene.machine(), person, &in_channel);

    assert_listed(
        &shown,
        task,
        &[CLAIMED, AWAITING_APPROVAL, DEFAULT, "1"],
        &[QUEUED, STARTED],
    );

    taken(cloud, bob, &held, ("complete", json!({})));

    let shown = listed(scene.machine(), person, &in_channel);

    assert_listed(
        &shown,
        task,
        &[DONE, DEFAULT, "1"],
        &[QUEUED, CLAIMED, STARTED, AWAITING_APPROVAL],
    );
}
