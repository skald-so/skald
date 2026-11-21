//! A lost answer to the revision a role command sends: the next run leaves one revision with the
//! change.

use std::thread;
use std::time::Duration;

use skald_proto::Role;

use crate::edits::last_two;
use crate::fixtures::{fronted, strs};
use crate::kit::{channels, rosters};
use crate::relay::Lose;
use crate::roles::{RoleCommand, assert_roles, space_with};
use crate::talk::{ADA_HOST, BOB_HOST};
use crate::world::{ACME, store_request};

/// Ada's machine created `acme`, with Bob a member. The relay loses the server's answer to the
/// revision of Ada's `command` about Bob as `how` says: it closes the connection, or it withholds
/// the answer and the test kills the CLI. The next run, `skald channels acme`, leaves the space
/// with one more revision, the command's roles alone, signed by Ada's device.
fn one_revision_after_a_lost_answer(
    command: RoleCommand,
    how: Lose,
    (ids, logins): ([u64; 2], [&str; 2]),
) {
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(ids[0], logins[0], ADA_HOST);
    let bob = world.sign_up(ids[1], logins[1], BOB_HOST);
    let (space, _) = space_with(cloud, &ada, ACME, &[(&bob, Role::Member)]);
    let held = rosters(cloud, &ada.token, space).len();
    let request = store_request(space);
    cloud.relay.lose_answer(&request, how);

    let started = ada.machine.start(&strs(&command.args(ACME, &bob.handle)));
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

    let (previous, next) = last_two(cloud, &ada.token, space, held);
    let roles = command.roles(ada.account, bob.account);
    assert_roles(cloud, (&previous, &next), ada.device, &roles, &[&ada, &bob]);
}

#[test]
fn make_admin_answer_dropped_by_the_proxy_leaves_one_revision() {
    one_revision_after_a_lost_answer(
        RoleCommand::MakeAdmin,
        Lose::Close,
        (
            [4_301_106_001, 4_301_106_002],
            ["ada-lost-promotion", "bob-lost-promotion"],
        ),
    );
}

#[test]
fn hand_over_admin_answer_lost_with_the_cli_killed_leaves_one_revision() {
    one_revision_after_a_lost_answer(
        RoleCommand::HandOver,
        Lose::Withhold,
        (
            [4_301_106_101, 4_301_106_102],
            ["ada-killed-handover", "bob-killed-handover"],
        ),
    );
}
