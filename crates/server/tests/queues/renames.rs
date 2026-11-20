//! A queue's name is one of its settings: its creator renames it, and `default` keeps its name. A
//! change keeps a queue's mode.

use serde_json::json;

use crate::kit::{AT_MOST_ONCE, DEFAULT, NOT_ALLOWED, World, assert_problem, named, names};

/// carol renames her `builds` to `ci`, which alice's laptop gets; alice's laptop, an admin's,
/// renames `default`: `not-allowed`, and it keeps its name.
#[test]
fn a_queue_is_renamed_and_default_keeps_its_name() {
    let w = World::new(3_907_600_001);
    let [alice, _, carol, ..] = w.laptops();
    let builds = carol.creates(&named("builds", json!({})));

    let ci = carol.changes(builds.id, &json!({ "name": "ci" }));
    assert_eq!(
        (ci.id, ci.name.as_str()),
        (builds.id, "ci"),
        "the renamed queue"
    );
    assert_eq!(alice.queue("ci"), ci, "ci as alice gets it");

    let default = alice.queue(DEFAULT);
    let response = alice.change(default.id, &json!({ "name": "main" }));
    assert_problem(&response, NOT_ALLOWED, "alice renames default");
    assert_eq!(names(&alice.queues()), ["ci", DEFAULT], "the queues");
}

/// carol's `deploys`, at-most-once, keeps its mode in the queue list and after a change of its
/// lease.
#[test]
fn an_at_most_once_queue_keeps_its_mode_when_listed_and_changed() {
    let w = World::new(3_907_600_002);
    let [alice, _, carol, ..] = w.laptops();
    let deploys = carol.creates(&named("deploys", json!({ "mode": AT_MOST_ONCE })));

    assert_eq!(
        alice.queue("deploys").mode,
        AT_MOST_ONCE,
        "deploys as listed"
    );
    let changed = carol.changes(deploys.id, &json!({ "lease": 60 }));
    assert_eq!(
        (changed.mode.as_str(), changed.lease),
        (AT_MOST_ONCE, 60),
        "deploys after the change"
    );
}
