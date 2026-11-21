//! The person lists, creates, changes and deletes a channel's queues; an agent lists them and is
//! refused the rest; a member who is neither a queue's creator nor an admin is refused its change
//! with `not-allowed`.

use serde_json::json;
use skald_proto::Role;

use crate::agents::{Agents, CLAUDE};
use crate::fixtures::{CHANNEL, fronted};
use crate::kit::{create_space, latest, received, with_channel};
use crate::queuing::{
    AT_LEAST_ONCE, AT_MOST_ONCE, CHANGE_QUEUE, CREATE_QUEUE, DEFAULT, DELETE_QUEUE, LEASE,
    LEASE_FLAG, MAX_ATTEMPTS, MAX_ATTEMPTS_FLAG, MODE_FLAG, NOT_ALLOWED, QUEUES, Queue,
    creates_queue, lists_queue, names_problem, names_queue, queue_named, queue_writes, queues,
    run_in,
};
use crate::talk::{ADA_HOST, BOB_HOST};
use crate::world::{ACME, ADD_RULE, dir_of, next_by, path, run, set_role, store};

/// The queues the tests create.
const DEPLOYS: &str = "deploys";
const BUILDS: &str = "builds";

/// The names of `queues`, in their order.
fn names(queues: &[Queue]) -> Vec<&str> {
    queues.iter().map(|queue| queue.name.as_str()).collect()
}

/// In `~/work/acme`, the person's `skald create-queue --channel <channel> --mode at-most-once
/// --lease 120 --max-attempts 3 deploys` creates `deploys` with those settings, and `skald queues
/// --channel <channel>` lists it beside `default` with the default settings; `skald queues`, of
/// the default channel of `acme`, lists `default` but not `deploys`. `skald change-queue --channel
/// <channel> --lease 600 --max-attempts 7 deploys` changes those settings and keeps the mode.
/// `skald delete-queue --channel <channel> deploys` deletes it: the server and `skald queues` have
/// only `default` left. `skald delete-queue --channel <channel> default` fails, and `default` stays.
#[test]
fn person_lists_creates_changes_and_deletes_queues() {
    let agents = Agents::new(
        [4_265_001_001, 4_265_001_002],
        ["ada-keeps-the-queues", "bob-sees-the-queues"],
    );
    let talk = &agents.talk;
    let (cloud, at, ada) = (talk.cloud(), talk.at(), &talk.ada);
    let person = (agents.folder.as_path(), None);
    let channel = at.1.to_string();
    let in_channel = [CHANNEL, channel.as_str()];

    let created = run_in(
        &ada.machine,
        person,
        CREATE_QUEUE,
        &[
            CHANNEL,
            &channel,
            MODE_FLAG,
            AT_MOST_ONCE,
            LEASE_FLAG,
            "120",
            MAX_ATTEMPTS_FLAG,
            "3",
            DEPLOYS,
        ],
    );

    assert!(created.success, "{created:?}");
    let deploys = queue_named(cloud, ada, at, DEPLOYS).expect("deploys created");
    assert_eq!(deploys.settings(), (AT_MOST_ONCE, 120, 3), "{deploys:?}");
    let listed = run_in(&ada.machine, person, QUEUES, &in_channel);
    assert!(listed.success, "{listed:?}");
    assert!(
        lists_queue(&listed, (DEFAULT, AT_LEAST_ONCE, LEASE, MAX_ATTEMPTS)),
        "default not listed with its settings: {listed:?}"
    );
    assert!(
        lists_queue(&listed, (DEPLOYS, AT_MOST_ONCE, 120, 3)),
        "deploys not listed with its settings: {listed:?}"
    );
    let of_default = run_in(&ada.machine, person, QUEUES, &[]);
    assert!(of_default.success, "{of_default:?}");
    assert!(
        lists_queue(&of_default, (DEFAULT, AT_LEAST_ONCE, LEASE, MAX_ATTEMPTS)),
        "the default channel's default not listed: {of_default:?}"
    );
    assert!(
        !names_queue(&of_default, DEPLOYS),
        "deploys listed in the default channel: {of_default:?}"
    );

    let changed = run_in(
        &ada.machine,
        person,
        CHANGE_QUEUE,
        &[
            CHANNEL,
            &channel,
            LEASE_FLAG,
            "600",
            MAX_ATTEMPTS_FLAG,
            "7",
            DEPLOYS,
        ],
    );

    assert!(changed.success, "{changed:?}");
    let after = queue_named(cloud, ada, at, DEPLOYS).expect("deploys kept");
    assert_eq!(
        (after.id, after.settings()),
        (deploys.id, (AT_MOST_ONCE, 600, 7)),
        "{after:?}"
    );

    let deleted = run_in(
        &ada.machine,
        person,
        DELETE_QUEUE,
        &[CHANNEL, &channel, DEPLOYS],
    );

    assert!(deleted.success, "{deleted:?}");
    assert_eq!(
        names(&queues(cloud, ada, at)),
        [DEFAULT],
        "deploys not deleted"
    );
    let listed = run_in(&ada.machine, person, QUEUES, &in_channel);
    assert!(listed.success, "{listed:?}");
    assert!(!names_queue(&listed, DEPLOYS), "deploys listed: {listed:?}");
    assert!(
        lists_queue(&listed, (DEFAULT, AT_LEAST_ONCE, LEASE, MAX_ATTEMPTS)),
        "default not listed: {listed:?}"
    );

    let refused = run_in(
        &ada.machine,
        person,
        DELETE_QUEUE,
        &[CHANNEL, &channel, DEFAULT],
    );

    assert!(!refused.success, "default deleted: {refused:?}");
    let default = queue_named(cloud, ada, at, DEFAULT).expect("default kept");
    assert_eq!(default.settings(), (AT_LEAST_ONCE, LEASE, MAX_ATTEMPTS));
}

/// Ada's device creates `deploys` in the channel through the API. As claude in `~/work/acme`,
/// `skald queues --channel <channel>` lists `default` and `deploys` with their settings;
/// `create-queue builds`, `change-queue --lease 600 deploys` and `delete-queue deploys` each fail,
/// no request creates, changes or deletes a queue, and the server's queues stay as they were.
#[test]
fn agent_lists_queues_and_is_refused_the_rest() {
    let agents = Agents::new(
        [4_265_002_001, 4_265_002_002],
        ["ada-lets-claude-look", "bob-shares-the-queues"],
    );
    agents.run_as(CLAUDE);
    let talk = &agents.talk;
    let (cloud, at, ada) = (talk.cloud(), talk.at(), &talk.ada);
    let settings = json!({ "name": DEPLOYS, "lease": 90 });
    creates_queue(cloud, ada, at, &settings);
    let before = queues(cloud, ada, at);
    let claude = (agents.folder.as_path(), Some(CLAUDE));
    let channel = at.1.to_string();
    let since = cloud.relay.mark();

    let listed = run_in(&ada.machine, claude, QUEUES, &[CHANNEL, &channel]);

    assert!(listed.success, "{listed:?}");
    assert!(
        lists_queue(&listed, (DEFAULT, AT_LEAST_ONCE, LEASE, MAX_ATTEMPTS)),
        "default not listed to claude: {listed:?}"
    );
    assert!(
        lists_queue(&listed, (DEPLOYS, AT_LEAST_ONCE, 90, MAX_ATTEMPTS)),
        "deploys not listed to claude: {listed:?}"
    );

    let refusals = [
        (CREATE_QUEUE, vec![CHANNEL, &channel, BUILDS]),
        (
            CHANGE_QUEUE,
            vec![CHANNEL, &channel, LEASE_FLAG, "600", DEPLOYS],
        ),
        (DELETE_QUEUE, vec![CHANNEL, &channel, DEPLOYS]),
    ];
    for (command, args) in refusals {
        let refused = run_in(&ada.machine, claude, command, &args);
        assert!(!refused.success, "claude's {command}: {refused:?}");
    }

    let writes = queue_writes(cloud, since);
    assert!(writes.is_empty(), "a queue written as claude: {writes:?}");
    assert_eq!(queues(cloud, ada, at), before, "the queues changed");
}

/// Bob's machine creates `acme` with a channel of Ada's and Bob's, in which Bob's device signs Ada
/// in as an admin and itself down to a member, as the tests of the history period do. Ada's device
/// creates `deploys` there through the API. On Bob's machine, `skald create-queue --channel
/// <channel> builds` creates `builds`, as any member's device does; `skald change-queue --channel
/// <channel> --lease 60 deploys` and `skald delete-queue --channel <channel> deploys` each fail
/// naming `not-allowed`, and `deploys` stays as Ada's device created it.
#[test]
fn change_of_another_account_s_queue_by_a_member_who_is_no_admin_shows_not_allowed() {
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let bob = world.sign_up(4_265_003_001, "bob-hands-acme-over", BOB_HOST);
    let ada = world.sign_up(4_265_003_002, "ada-owns-deploys", ADA_HOST);
    let space = create_space(cloud, &bob, ACME).id;
    let channel = with_channel(cloud, space, &bob, &[(&ada, Role::Admin)]);
    let handed_over = next_by(&latest(cloud, &bob.token, space), &bob, |list| {
        set_role(list, bob.account, Role::Member);
    });
    store(cloud, &bob.token, &handed_over);
    received(&bob.machine);
    let at = (space, channel);
    let deploys = creates_queue(cloud, &ada, at, &json!({ "name": DEPLOYS }));
    let folder = dir_of(&bob.machine, "work/acme");
    let bound = run(&bob.machine, &[ADD_RULE, path(&folder), ACME]);
    assert!(bound.success, "not bound: {bound:?}");
    let person = (folder.as_path(), None);
    let id = channel.to_string();

    let created = run_in(&bob.machine, person, CREATE_QUEUE, &[CHANNEL, &id, BUILDS]);

    assert!(created.success, "{created:?}");
    let builds = queue_named(cloud, &ada, at, BUILDS).expect("builds created");
    assert_eq!(builds.settings(), (AT_LEAST_ONCE, LEASE, MAX_ATTEMPTS));

    let changed = run_in(
        &bob.machine,
        person,
        CHANGE_QUEUE,
        &[CHANNEL, &id, LEASE_FLAG, "60", DEPLOYS],
    );

    assert!(!changed.success, "changed: {changed:?}");
    assert!(names_problem(&changed, NOT_ALLOWED), "{changed:?}");

    let deleted = run_in(&bob.machine, person, DELETE_QUEUE, &[CHANNEL, &id, DEPLOYS]);

    assert!(!deleted.success, "deleted: {deleted:?}");
    assert!(names_problem(&deleted, NOT_ALLOWED), "{deleted:?}");
    let kept = queue_named(cloud, &ada, at, DEPLOYS).expect("deploys kept");
    assert_eq!(kept, deploys, "deploys changed");
}
