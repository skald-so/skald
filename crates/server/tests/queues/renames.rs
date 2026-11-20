//! A queue's name is one of its settings: its creator renames it, and `default` keeps its name.

use serde_json::json;

use crate::kit::{DEFAULT, NOT_ALLOWED, World, assert_problem, named, names};

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
