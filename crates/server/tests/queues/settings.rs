//! Who creates, changes and deletes a queue: a `client` device of a member of the channel creates
//! one; one of the creator's account or of an admin of the space changes a setting other than
//! `mode`, or deletes the queue with its tasks; others get `not-allowed`. A setting out of range
//! and a name the channel has are refused; a session or a token alone does none of it.

use serde_json::json;

use crate::kit::{
    AT_LEAST_ONCE, AT_MOST_ONCE, DEFAULT, LEASE, MAX_ATTEMPTS, NOT_ALLOWED, UNKNOWN_QUEUE, World,
    assert_gone, assert_problem, named, names,
};
use crate::presence::{assert_kept, assert_refused_with, assert_unproved};
use crate::tokens::{QUEUES, SEND};

/// carol, a member who is no admin, creates `builds` with the default settings and then changes
/// its lease and attempts, each to its bounds; alice's laptop gets each change. carol also creates
/// `deploys`, at-most-once with the shortest lease and one attempt.
#[test]
fn a_member_device_creates_a_queue_and_its_creator_changes_it() {
    let w = World::new(3_907_200_001);
    let [alice, _, carol, ..] = w.laptops();

    let builds = carol.creates(&named("builds", json!({})));
    assert_eq!(builds.name, "builds");
    builds.assert_settings(AT_LEAST_ONCE, LEASE, MAX_ATTEMPTS);
    assert_eq!(alice.queue("builds"), builds, "builds as alice gets it");

    let changed = carol.changes(builds.id, &json!({ "lease": 600 }));
    changed.assert_settings(AT_LEAST_ONCE, 600, MAX_ATTEMPTS);
    assert_eq!(changed.id, builds.id, "the same queue");
    assert_eq!(alice.queue("builds"), changed, "the lease as alice gets it");
    for (settings, lease, attempts) in [
        (json!({ "lease": 3600, "max_attempts": 100 }), 3600, 100),
        (json!({ "lease": 30, "max_attempts": 1 }), 30, 1),
    ] {
        carol.changes(builds.id, &settings);
        alice
            .queue("builds")
            .assert_settings(AT_LEAST_ONCE, lease, attempts);
    }

    let settings = json!({ "mode": AT_MOST_ONCE, "lease": 30, "max_attempts": 1 });
    let deploys = carol.creates(&named("deploys", settings));
    deploys.assert_settings(AT_MOST_ONCE, 30, 1);
    assert_eq!(
        names(&alice.queues()),
        ["builds", DEFAULT, "deploys"],
        "the channel's queues"
    );
}

/// carol creates `builds` and enqueues a task in it; alice's laptop, an admin's, deletes it with
/// the task: the channel lists it no more, the database holds the task no more, and an enqueue or
/// a read of its tasks gets `unknown-queue`.
#[test]
fn an_admins_device_deletes_a_members_queue_with_its_tasks() {
    let w = World::new(3_907_200_011);
    let [alice, _, carol, ..] = w.laptops();
    let builds = carol.creates(&named("builds", json!({})));
    let task = w.task(builds.id);
    carol.stores(&task);

    assert_kept(&alice.delete(builds.id), "alice deletes builds");
    assert_eq!(
        names(&carol.queues()),
        [DEFAULT],
        "the queues after the delete"
    );
    assert_gone(w.db(), &task, "builds's task");
    let later = w.task(builds.id);
    assert_problem(&carol.enqueue(&later), UNKNOWN_QUEUE, "an enqueue after");
    let response = carol.list_tasks(builds.id, true);
    assert_problem(&response, UNKNOWN_QUEUE, "a read after");
}

/// Refused with `not-allowed`, changing nothing: carol's change and delete of alice's `deploys`, as
/// carol is no admin; alice's change of its `mode`, and carol's of her own queue's; a queue created
/// by the bot, a `service` device, even on a token with `send` and `queues`, and its change and
/// delete of `deploys`; one created by dave's laptop, outside the channel. eve's laptop, outside the
/// space, creates none either.
#[test]
fn others_and_mode_changes_get_not_allowed() {
    let w = World::new(3_907_200_021);
    let [alice, _, carol, dave, eve] = w.laptops();
    let bot = w.bot(&[SEND, QUEUES]);
    let deploys = alice.creates(&named("deploys", json!({ "lease": 120 })));
    let builds = carol.creates(&named("builds", json!({})));

    let refusals = [
        (
            carol.change(deploys.id, &json!({ "lease": 600 })),
            "carol changes deploys",
        ),
        (carol.delete(deploys.id), "carol deletes deploys"),
        (
            alice.change(deploys.id, &json!({ "mode": AT_MOST_ONCE })),
            "alice changes the mode",
        ),
        (
            carol.change(builds.id, &json!({ "mode": AT_MOST_ONCE })),
            "carol changes the mode",
        ),
        (bot.create(&named("bots", json!({}))), "the bot creates"),
        (
            bot.change(deploys.id, &json!({ "lease": 600 })),
            "the bot changes deploys",
        ),
        (bot.delete(deploys.id), "the bot deletes deploys"),
        (dave.create(&named("daves", json!({}))), "dave creates"),
    ];
    for (response, what) in &refusals {
        assert_problem(response, NOT_ALLOWED, what);
    }
    let response = eve.create(&named("eves", json!({})));
    assert_refused_with(&response, None, "eve creates");

    let queues = alice.queues();
    assert_eq!(
        names(&queues),
        ["builds", DEFAULT, "deploys"],
        "the queues after the refusals"
    );
    assert_eq!(queues[0], builds, "builds after the refusals");
    assert_eq!(queues[2], deploys, "deploys after the refusals");
}

/// Refused, creating or changing nothing: a lease under 30 or over 3600 seconds, `max_attempts`
/// of 0 or over 100, an unknown mode, a name that is no `name`, and a name the channel has, at
/// creation; out of range values and a name the channel has in a change.
#[test]
fn settings_out_of_range_and_names_the_channel_has_are_refused() {
    let w = World::new(3_907_200_031);
    let [alice, _, carol, ..] = w.laptops();
    let builds = carol.creates(&named("builds", json!({})));

    let creations = [
        named("short", json!({ "lease": 29 })),
        named("long", json!({ "lease": 3601 })),
        named("none", json!({ "max_attempts": 0 })),
        named("many", json!({ "max_attempts": 101 })),
        named("exactly", json!({ "mode": "exactly-once" })),
        named("Builds", json!({})),
        named("-builds", json!({})),
        named("builds", json!({ "lease": 60 })),
        named(DEFAULT, json!({})),
    ];
    for settings in &creations {
        let response = carol.create(settings);
        assert_refused_with(&response, None, &format!("a creation of {settings}"));
    }
    let changes = [
        json!({ "lease": 29 }),
        json!({ "lease": 3601 }),
        json!({ "max_attempts": 0 }),
        json!({ "max_attempts": 101 }),
        json!({ "name": DEFAULT }),
    ];
    for settings in &changes {
        let response = carol.change(builds.id, settings);
        assert_refused_with(&response, None, &format!("a change of {settings}"));
    }

    let queues = alice.queues();
    assert_eq!(
        names(&queues),
        ["builds", DEFAULT],
        "the queues after the refusals"
    );
    assert_eq!(queues[0], builds, "builds after the refusals");
}

/// carol's session without her laptop's proof creates no queue, and alice's changes and deletes
/// none: `401` with `auth-required`. Neither does the bot's token alone.
#[test]
fn a_session_or_token_alone_creates_changes_and_deletes_no_queue() {
    let w = World::new(3_907_200_041);
    let [alice, _, carol, ..] = w.laptops();
    let bot = w.bot(&[SEND, QUEUES]);
    let builds = carol.creates(&named("builds", json!({})));
    let lease = json!({ "lease": 600 });

    let unproved = [
        (
            carol.create_with(&named("tests", json!({})), false),
            "carol's session creates",
        ),
        (
            alice.change_with(builds.id, &lease, false),
            "alice's session changes",
        ),
        (
            alice.delete_with(builds.id, false),
            "alice's session deletes",
        ),
        (
            bot.create_with(&named("bots", json!({})), false),
            "the token creates",
        ),
        (
            bot.change_with(builds.id, &lease, false),
            "the token changes",
        ),
        (bot.delete_with(builds.id, false), "the token deletes"),
    ];
    for (response, what) in &unproved {
        assert_unproved(response, what);
    }
    let queues = alice.queues();
    assert_eq!(
        names(&queues),
        ["builds", DEFAULT],
        "the queues after the refusals"
    );
    assert_eq!(queues[0], builds, "builds after the refusals");
}
