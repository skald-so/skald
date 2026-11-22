//! The queue `default`: every channel has it, with the default settings, from when the server
//! issues the channel's ID or, for a channel from before queues, from the migration; nobody deletes
//! it.

use serde_json::json;

use crate::kit::{
    AT_LEAST_ONCE, DEFAULT, LEASE, MAX_ATTEMPTS, NOT_ALLOWED, World, assert_problem, before_queues,
    names,
};

/// `general`, whose ID the server issued, has `default` alone, at-least-once with a lease of 300
/// seconds and 5 attempts, as alice's and carol's laptops get it.
#[test]
fn a_new_channel_has_the_default_queue_with_the_default_settings() {
    let w = World::new(3_907_100_001);
    let [alice, _, carol, ..] = w.laptops();
    for laptop in [&alice, &carol] {
        let queues = laptop.queues();
        assert_eq!(names(&queues), [DEFAULT], "{}: the queues", laptop.who);
        queues[0].assert_settings(AT_LEAST_ONCE, LEASE, MAX_ATTEMPTS);
    }
}

/// A database as a server before queues left it, holding `general` and its group, gets `default`
/// for `general` from the migration, with the default settings: a server on it gives it to alice's
/// laptop, which enqueues in it.
#[test]
fn a_channel_from_before_queues_gets_the_default_queue() {
    let w = World::new(3_907_100_011);
    let old = before_queues(w.db());
    let flow = &w.fixture().team.flow;
    let migrated = flow.stand.instance_at(&old.url(), &[], &[]);
    let alice = w.laptop_on(&migrated, &w.fixture().team.alice);

    let queues = alice.queues();
    assert_eq!(names(&queues), [DEFAULT], "the queues after the migration");
    queues[0].assert_settings(AT_LEAST_ONCE, LEASE, MAX_ATTEMPTS);
    alice.stores(&w.task(queues[0].id));
}

/// alice, an admin, cannot delete `default`: `not-allowed`, and it stays. carol, no admin, changes
/// none of its settings, as it has no creator; alice changes its lease.
#[test]
fn nobody_deletes_the_default_queue_and_only_an_admin_changes_it() {
    let w = World::new(3_907_100_021);
    let [alice, _, carol, ..] = w.laptops();
    let default = alice.queue(DEFAULT);

    assert_problem(
        &alice.delete(default.id),
        NOT_ALLOWED,
        "alice deletes default",
    );
    assert_problem(
        &carol.delete(default.id),
        NOT_ALLOWED,
        "carol deletes default",
    );
    let response = carol.change(default.id, &json!({ "lease": 600 }));
    assert_problem(&response, NOT_ALLOWED, "carol changes default");
    assert_eq!(carol.queue(DEFAULT), default, "default after the refusals");

    let changed = alice.changes(default.id, &json!({ "lease": 600 }));
    changed.assert_settings(AT_LEAST_ONCE, 600, MAX_ATTEMPTS);
    assert_eq!(
        carol.queue(DEFAULT),
        changed,
        "default after alice's change"
    );
}
