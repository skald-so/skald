//! A lost answer to the revision a role command sends: the next run leaves one revision with the
//! change.

use skald_proto::Role;

use crate::edits::{ada_and_bob, last_two, run_losing_the_answer};
use crate::fixtures::{fronted, strs};
use crate::relay::Lose;
use crate::roles::{RoleCommand, assert_roles, space_with};
use crate::world::ACME;

/// Ada's machine created `acme`, with Bob a member. The relay loses the server's answer to the
/// revision of Ada's `command` about Bob as `how` says: it closes the connection, or it withholds
/// the answer and the test kills the CLI. The next run, `skald channels acme`, leaves the space
/// with one more revision, the command's roles alone, signed by Ada's device.
fn one_revision_after_a_lost_answer(
    command: RoleCommand,
    how: Lose,
    people: ([u64; 2], [&str; 2]),
) {
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let (ada, bob) = ada_and_bob(&world, people);
    let (space, _) = space_with(cloud, &ada, ACME, &[(&bob, Role::Member)]);

    let args = command.args(ACME, &bob.handle);
    let held = run_losing_the_answer(cloud, (space, ACME), &ada, &strs(&args), how);

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
