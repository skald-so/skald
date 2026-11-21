//! A role change whose member another admin removed first is dropped, saying why, and leaves no
//! revision of this device on the server.

use skald_proto::Role;

use crate::edits::Edit;
use crate::fixtures::fronted;
use crate::kit::rosters;
use crate::roles::{RoleCommand, role, signed_by, space_with};
use crate::talk::{ADA_HOST, BOB_HOST, CAROL_HOST};
use crate::world::{ACME, stale_once, words};

/// Ada's machine created `acme`, with Bob a member and Carol an admin. Carol's device removes Bob
/// by the next revision; the first revisions Ada's machine fetches hold not it. Ada's `skald
/// hand-over-admin acme <bob>` fails, saying Bob, by his handle, is `gone`, and the server holds no
/// revision after Carol's.
#[test]
fn hand_over_to_a_member_removed_first_is_dropped() {
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_301_108_001, "ada-hands-to-gone", ADA_HOST);
    let bob = world.sign_up(4_301_108_002, "bob-removed-first", BOB_HOST);
    let carol = world.sign_up(4_301_108_003, "carol-removes-first", CAROL_HOST);
    let (space, _) = space_with(
        cloud,
        &ada,
        ACME,
        &[(&bob, Role::Member), (&carol, Role::Admin)],
    );
    let held = rosters(cloud, &ada.token, space).len();
    signed_by(cloud, space, &carol, |body| {
        Edit::RemoveMember(&bob).apply(body)
    });
    let removed = rosters(cloud, &ada.token, space);
    stale_once(cloud, space, held);

    let handed = role(&ada.machine, RoleCommand::HandOver, ACME, &bob);

    assert!(!handed.success, "{handed:?}");
    let said = handed.shown();
    assert!(
        (said.lines())
            .any(|line| words(line).contains(&bob.handle.as_str()) && line.contains("gone")),
        "it does not say Bob is gone: {handed:?}"
    );
    assert_eq!(
        rosters(cloud, &ada.token, space),
        removed,
        "the space's revisions"
    );
}
