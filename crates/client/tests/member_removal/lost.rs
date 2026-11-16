//! A lost answer to the revision a command sends: the next run leaves one revision with the change.

use std::thread;
use std::time::Duration;

use skald_proto::Role;

use crate::edits::{Edit, assert_change, last_two};
use crate::fixtures::{fronted, strs};
use crate::kit::{channels, create_space, received, rosters, with_channel};
use crate::relay::Lose;
use crate::talk::{ADA_HOST, BOB_HOST};
use crate::world::{ACME, store_request};

/// Which command loses its answer.
#[derive(Clone, Copy)]
enum Command {
    RemoveMember,
    Exclude,
    AddToChannel,
    RemoveFromChannel,
}

/// Ada's machine creates `acme`, with Bob a member in the default channel and in a channel with
/// Ada, and a channel with Ada alone. The relay loses the server's answer to the revision of Ada's
/// `command` about Bob as `how` says: it closes the connection, or it withholds the answer and the
/// test kills the CLI. The next run, `skald channels acme`, leaves the space with one more
/// revision, the change alone, signed by Ada's device.
fn one_revision_after_a_lost_answer(
    command: Command,
    how: Lose,
    (ids, logins): ([u64; 2], [&str; 2]),
) {
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(ids[0], logins[0], ADA_HOST);
    let bob = world.sign_up(ids[1], logins[1], BOB_HOST);
    let space = create_space(cloud, &ada, ACME).id;
    let shared = with_channel(cloud, space, &ada, &[(&bob, Role::Member)]);
    let alone = with_channel(cloud, space, &ada, &[]);
    received(&ada.machine);
    let held = rosters(cloud, &ada.token, space).len();
    let edit = match command {
        Command::RemoveMember => Edit::RemoveMember(&bob),
        Command::Exclude => Edit::Exclude(&bob),
        Command::AddToChannel => Edit::AddToChannel(alone, &bob, false),
        Command::RemoveFromChannel => Edit::RemoveFromChannel(shared, &bob),
    };
    let request = store_request(space);
    cloud.relay.lose_answer(&request, how);

    let started = ada.machine.start(&strs(&edit.args(ACME)));
    if how == Lose::Close {
        // It may try again itself and finish.
        drop(started.finish());
    } else {
        cloud.kill_once_lost(started, &request);
    }
    assert_eq!(cloud.relay.lost(), 1, "{request} lost no answer");
    // Signed again, a revision has another timestamp, and so other bytes.
    thread::sleep(Duration::from_millis(1100));
    channels(&ada.machine, ACME);

    let revisions = last_two(cloud, &ada.token, space, held);
    assert_change(cloud, (&revisions.0, &revisions.1), &ada, edit);
}

#[test]
fn remove_member_answer_dropped_by_the_proxy_leaves_one_revision() {
    one_revision_after_a_lost_answer(
        Command::RemoveMember,
        Lose::Close,
        (
            [4_140_105_001, 4_140_105_002],
            ["ada-lost-removal", "bob-lost-removal"],
        ),
    );
}

#[test]
fn exclude_device_answer_lost_with_the_cli_killed_leaves_one_revision() {
    one_revision_after_a_lost_answer(
        Command::Exclude,
        Lose::Withhold,
        (
            [4_140_105_101, 4_140_105_102],
            ["ada-killed-bar", "bob-killed-bar"],
        ),
    );
}

#[test]
fn add_to_channel_answer_lost_with_the_cli_killed_leaves_one_revision() {
    one_revision_after_a_lost_answer(
        Command::AddToChannel,
        Lose::Withhold,
        (
            [4_140_105_201, 4_140_105_202],
            ["ada-killed-add", "bob-killed-add"],
        ),
    );
}

#[test]
fn remove_from_channel_answer_dropped_by_the_proxy_leaves_one_revision() {
    one_revision_after_a_lost_answer(
        Command::RemoveFromChannel,
        Lose::Close,
        (
            [4_140_105_301, 4_140_105_302],
            ["ada-lost-drop", "bob-lost-drop"],
        ),
    );
}
