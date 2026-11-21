//! A member device and a role change by another admin's revision: before it is an admin's, Bob's
//! device refuses the role commands and gets no only-admin warning; it shows a role change once,
//! only from an admin's revision; once an admin's, it removes members.

use skald_proto::{Role, Roster};

use crate::edits::{Edit, assert_change, edited, last_two};
use crate::fixtures::fronted;
use crate::kit::{latest, received, rosters};
use crate::roles::{
    MAKE_ADMIN, RoleCommand, assert_refused_sending_nothing, assert_told_admin_once, role, role_by,
    space_with,
};
use crate::talk::{ADA_HOST, BOB_HOST, CAROL_HOST};
use crate::world::{
    ACME, ADMIN, MEMBER, assert_role, members, next_by, revisions_request, serve_revisions,
    set_role, spaces, warnings,
};

/// Bob's machine created `acme` and its device signed the revision that adds Ada as an admin and
/// Carol as a member; Ada's device signed the next one, which makes Bob a member. Bob's device,
/// which accepts it, shows no only-admin warning in `skald members` or `skald spaces`, and refuses
/// `skald make-admin acme <carol>` and `skald hand-over-admin acme <carol>`, saying Bob is no
/// admin, and sends nothing. Ada's device then signs the revision that makes Bob an admin: Bob's
/// `skald members` lists him as an admin, his runs tell once that he is an admin by Ada's revision,
/// and his `skald remove-member acme <carol>` signs the revision that removes Carol.
#[test]
fn member_s_device_made_an_admin_is_told_once_and_then_removes_a_member() {
    let (world, front) = fronted();
    let cloud = &world.cloud;
    let bob = world.sign_up(4_301_103_001, "bob-promoted-later", BOB_HOST);
    let ada = world.sign_up(4_301_103_002, "ada-promotes-bob", ADA_HOST);
    let carol = world.sign_up(4_301_103_003, "carol-removed-later", CAROL_HOST);
    let (space, _) = space_with(
        cloud,
        &bob,
        ACME,
        &[(&ada, Role::Admin), (&carol, Role::Member)],
    );
    role_by(cloud, space, &ada, (bob.account, Role::Member));
    let demoted = members(&bob.machine, ACME);
    assert_role(&demoted, &bob.handle, MEMBER);
    let listed = spaces(&bob.machine);
    for shown in [&demoted, &listed] {
        assert!(
            !shown.shown().contains(MAKE_ADMIN),
            "an only-admin warning on a member's device: {shown:?}"
        );
    }
    let held = rosters(cloud, &bob.token, space);
    for command in [RoleCommand::MakeAdmin, RoleCommand::HandOver] {
        let (since, from) = (cloud.relay.mark(), front.sent().len());

        let refused = role(&bob.machine, command, ACME, &carol);

        assert_refused_sending_nothing(
            (cloud, &front),
            (since, from),
            (space, &bob.token, &held),
            &refused,
        );
    }

    role_by(cloud, space, &ada, (bob.account, Role::Admin));

    let shown = members(&bob.machine, ACME);
    assert_role(&shown, &bob.handle, ADMIN);
    let later = received(&bob.machine);
    let again = members(&bob.machine, ACME);
    assert_told_admin_once(&[&shown, &later, &again], &bob.handle, &ada.handle);
    let held = rosters(cloud, &bob.token, space).len();
    edited(&bob.machine, ACME, Edit::RemoveMember(&carol));
    let (previous, next) = last_two(cloud, &bob.token, space, held);
    assert_change(cloud, (&previous, &next), &bob, Edit::RemoveMember(&carol));
}

/// Carol's machine created `acme` and its device signed the revision that adds Ada as an admin and
/// Bob as a member; Ada's device signed the next one, which makes Carol a member. The proxy gives
/// Carol's device a revision that makes Bob an admin, signed by Bob's device, a member's: Carol's
/// `skald members` warns and lists Bob as a member. Once the proxy gives the revisions the server
/// holds, Ada's device signs the revision that makes Bob an admin: Carol's `skald members` lists
/// him as an admin, and her runs tell once that Bob is an admin by Ada's revision.
#[test]
fn member_s_device_shows_a_role_change_once_and_only_from_an_admin_s_revision() {
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let carol = world.sign_up(4_301_103_101, "carol-watches-roles", CAROL_HOST);
    let ada = world.sign_up(4_301_103_102, "ada-the-only-admin", ADA_HOST);
    let bob = world.sign_up(4_301_103_103, "bob-claims-admin", BOB_HOST);
    let (space, _) = space_with(
        cloud,
        &carol,
        ACME,
        &[(&ada, Role::Admin), (&bob, Role::Member)],
    );
    role_by(cloud, space, &ada, (carol.account, Role::Member));
    received(&carol.machine);
    let forged = next_by(&latest(cloud, &carol.token, space), &bob, |members| {
        set_role(members, bob.account, Role::Admin);
    });
    let held = rosters(cloud, &carol.token, space);
    let mut served: Vec<&Roster> = held.iter().collect();
    served.push(&forged);
    serve_revisions(cloud, space, &served);

    let refused = members(&carol.machine, ACME);

    assert!(!warnings(&refused).is_empty(), "no warning: {refused:?}");
    assert_role(&refused, &bob.handle, MEMBER);
    cloud
        .relay
        .rewrite(&revisions_request(space), str::to_owned);

    role_by(cloud, space, &ada, (bob.account, Role::Admin));

    let shown = members(&carol.machine, ACME);
    assert_role(&shown, &bob.handle, ADMIN);
    let later = received(&carol.machine);
    let again = members(&carol.machine, ACME);
    assert_told_admin_once(&[&shown, &later, &again], &bob.handle, &ada.handle);
}
