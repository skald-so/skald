//! A lost answer to the revision a command sends: the next run leaves one revision with the change.

use skald_proto::Role;

use crate::edits::{Edit, ada_and_bob, assert_change, last_two, run_losing_the_answer};
use crate::fixtures::{fronted, strs};
use crate::kit::{create_space, received, with_channel};
use crate::relay::Lose;
use crate::world::ACME;

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
fn one_revision_after_a_lost_answer(command: Command, how: Lose, people: ([u64; 2], [&str; 2])) {
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let (ada, bob) = ada_and_bob(&world, people);
    let space = create_space(cloud, &ada, ACME).id;
    let shared = with_channel(cloud, space, &ada, &[(&bob, Role::Member)]);
    let alone = with_channel(cloud, space, &ada, &[]);
    received(&ada.machine);
    let edit = match command {
        Command::RemoveMember => Edit::RemoveMember(&bob),
        Command::Exclude => Edit::Exclude(&bob),
        Command::AddToChannel => Edit::AddToChannel(alone, &bob, false),
        Command::RemoveFromChannel => Edit::RemoveFromChannel(shared, &bob),
    };

    let held = run_losing_the_answer(cloud, (space, ACME), &ada, &strs(&edit.args(ACME)), how);

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
