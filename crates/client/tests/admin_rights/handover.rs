//! Handing over admin rights: one revision makes the member an admin and this account a member;
//! this device then refuses each admin command and sends nothing, and the new admin's revisions
//! count.

use skald_proto::Role;

use crate::edits::{Edit, HISTORY};
use crate::fixtures::fronted;
use crate::kit::{received, rosters};
use crate::machine::YES;
use crate::roles::{
    HAND_OVER_ADMIN, MAKE_ADMIN, RoleCommand, assert_refused_sending_nothing,
    assert_told_admin_once, role_changed, signed_by, space_with,
};
use crate::talk::{ADA_HOST, BOB_HOST, CAROL_HOST};
use crate::world::{ACME, ADMIN, CREATE_CHANNEL, MEMBER, assert_role, lines_naming, members, run};

/// The command that sets a space's history period, and a period it takes.
const SET_HISTORY_PERIOD: &str = "set-history-period";
const PERIOD: &str = "2d";

/// Ada's machine created `acme`, with Bob and Carol members in a channel with her, and a channel
/// with her alone. Ada's `skald hand-over-admin acme <bob>` signs one revision, with Bob an admin
/// and Ada a member and nothing else, which Ada's, Bob's and Carol's devices accept; Ada's `skald
/// members` lists Bob as an admin and Ada as a member, and her runs tell once that Bob is an admin
/// by her revision. Ada's device then refuses each admin command, saying she is no admin, and
/// sends nothing: removing Carol, barring her device, adding her to the channel with Ada alone,
/// removing Bob from the shared one, setting the history period, creating a channel, making Carol
/// an admin and handing the rights to her. Bob's device then signs the revision that removes
/// Carol, which Ada's next run accepts: it no longer lists her.
#[test]
fn handed_over_rights_leave_this_account_a_member_and_the_new_admin_removes_members() {
    let (world, front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_301_102_001, "ada-hands-over", ADA_HOST);
    let bob = world.sign_up(4_301_102_002, "bob-takes-over", BOB_HOST);
    let carol = world.sign_up(4_301_102_003, "carol-under-bob", CAROL_HOST);
    let (space, [shared, alone]) = space_with(
        cloud,
        &ada,
        ACME,
        &[(&bob, Role::Member), (&carol, Role::Member)],
    );

    let handed = role_changed(
        cloud,
        (&ada, RoleCommand::HandOver, &bob),
        (space, ACME),
        &[&bob, &carol],
    );

    let shown = members(&ada.machine, ACME);
    assert_role(&shown, &bob.handle, ADMIN);
    assert_role(&shown, &ada.handle, MEMBER);
    let later = received(&ada.machine);
    assert_told_admin_once(&[&handed, &shown, &later], &bob.handle, &ada.handle);

    let held = rosters(cloud, &ada.token, space);
    let (carol_handle, bob_handle) = (carol.handle.as_str(), bob.handle.as_str());
    let (shared, alone, device) = (
        shared.to_string(),
        alone.to_string(),
        carol.device.to_string(),
    );
    let commands: [&[&str]; 8] = [
        &["remove-member", ACME, carol_handle],
        &["exclude-device", ACME, &device],
        &["add-to-channel", ACME, &alone, carol_handle, HISTORY, YES],
        &["remove-from-channel", ACME, &shared, bob_handle],
        &[SET_HISTORY_PERIOD, ACME, PERIOD],
        &[CREATE_CHANNEL, ACME],
        &[MAKE_ADMIN, ACME, carol_handle],
        &[HAND_OVER_ADMIN, ACME, carol_handle],
    ];
    for args in commands {
        let (since, from) = (cloud.relay.mark(), front.sent().len());

        let refused = run(&ada.machine, args);

        assert_refused_sending_nothing(
            (cloud, &front),
            (since, from),
            (space, &ada.token, &held),
            &refused,
        );
    }

    signed_by(cloud, space, &bob, |body| {
        Edit::RemoveMember(&carol).apply(body)
    });
    let after = members(&ada.machine, ACME);
    assert!(
        lines_naming(&after, carol.handle.as_str()).is_empty(),
        "Carol is still listed: {after:?}"
    );
    assert_role(&after, &bob.handle, ADMIN);
    assert_role(&after, &ada.handle, MEMBER);
}

/// Ada and Bob are admins of Ada's `acme`. Ada's `skald hand-over-admin acme <bob>` signs one
/// revision, with Ada a member and nothing else, which both devices accept; Ada's `skald members`
/// lists Bob as an admin and Ada as a member.
#[test]
fn handing_over_to_an_admin_leaves_this_account_a_member() {
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_301_102_101, "ada-steps-down", ADA_HOST);
    let bob = world.sign_up(4_301_102_102, "bob-already-admin", BOB_HOST);
    let (space, _) = space_with(cloud, &ada, ACME, &[(&bob, Role::Admin)]);

    role_changed(
        cloud,
        (&ada, RoleCommand::HandOver, &bob),
        (space, ACME),
        &[&bob],
    );

    let shown = members(&ada.machine, ACME);
    assert_role(&shown, &bob.handle, ADMIN);
    assert_role(&shown, &ada.handle, MEMBER);
}
