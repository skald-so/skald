//! A second admin: Ada's device makes Bob an admin by one revision, which every member device
//! accepts and Ada's shows, and Bob's next revision, which removes Carol, counts: Ada's device then
//! removes Carol's from each channel.

use crate::edits::{Edit, assert_group_is};
use crate::kit::received;
use crate::roles::{RoleCommand, assert_told_admin_once, role_changed, signed_by};
use crate::talk::Talk;
use crate::world::{ADMIN, assert_role, lines_naming, members};

/// Ada's machine created `acme`, with Bob and Carol members, whose devices, played, are in its
/// default channel and in a channel with Ada. Ada's `skald make-admin acme <bob>` signs the next
/// revision with Bob an admin alone, which Ada's, Bob's and Carol's devices accept; Ada's `skald
/// members` lists Bob as an admin, and her runs tell once that Bob is an admin by her revision.
/// Bob's device then signs the revision that removes Carol from the space and from every channel:
/// Ada's next run accepts it and no longer lists Carol, and her device removes Carol's from both
/// channels, whose groups, on the server and at Bob's device, hold Ada's and Bob's devices alone.
#[test]
fn second_admin_s_revision_removes_a_member_and_her_devices() {
    let mut talk = Talk::new(
        &[4_301_101_001, 4_301_101_002, 4_301_101_003],
        &[
            "ada-names-second",
            "bob-made-second",
            "carol-removed-by-bob",
        ],
        false,
    );
    let (space, default, channel) = (talk.space.id, talk.space.default, talk.channel);
    let cloud = &talk.world.cloud;
    let (ada, bob, carol) = (&talk.ada, &talk.played[0].person, &talk.played[1].person);

    let made = role_changed(
        cloud,
        (ada, RoleCommand::MakeAdmin, bob),
        (space, &talk.space.name),
        &[bob, carol],
    );

    let shown = members(&ada.machine, &talk.space.name);
    assert_role(&shown, &bob.handle, ADMIN);
    assert_role(&shown, &ada.handle, ADMIN);
    let later = received(&ada.machine);
    let again = members(&ada.machine, &talk.space.name);
    assert_told_admin_once(&[&made, &shown, &later, &again], &bob.handle, &ada.handle);

    signed_by(cloud, space, bob, |body| {
        Edit::RemoveMember(carol).apply(body)
    });
    received(&ada.machine);

    let after = members(&ada.machine, &talk.space.name);
    assert!(
        lines_naming(&after, carol.handle.as_str()).is_empty(),
        "Carol is still listed: {after:?}"
    );
    assert_role(&after, &bob.handle, ADMIN);
    let devices = [talk.ada.device, talk.played[0].device()];
    talk.played[0].take_in(cloud);
    for group_of in [default, channel] {
        let bob_s = (&talk.played[0], talk.ada.token.as_str());
        assert_group_is(cloud, bob_s, (space, group_of), &devices);
    }
}
