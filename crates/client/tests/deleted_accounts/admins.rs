//! A deleted admin counts for no only-admin warning: the admin left warns.

use skald_proto::Role;

use crate::fixtures::fronted;
use crate::kit::{Change, create_space, latest, received, revise};
use crate::machine::Finished;
use crate::roles::{MAKE_ADMIN, RECOVERY_KEY};
use crate::scene::delete_account;
use crate::talk::{ADA_HOST, BOB_HOST, CAROL_HOST};
use crate::world::{ACME, ADMIN, members, spaces, warnings, words};

/// Whether `shown` warns that its space has one admin, as the tests of admin rights check it: its
/// output names the recovery key and the command that makes a member an admin, and a warning line
/// holds `admin` and, with `space`, the space's name as a word.
fn warns_of_one_admin(shown: &Finished, space: Option<&str>) -> bool {
    let text = shown.shown().to_lowercase();
    let names_both = [RECOVERY_KEY, MAKE_ADMIN]
        .iter()
        .all(|part| text.contains(part));
    let of_space = |line: &String| space.is_none_or(|name| words(line).contains(&name));
    names_both && (warnings(shown).iter()).any(|line| line.contains(ADMIN) && of_space(line))
}

/// Bob's machine created `acme`, whose next revision, signed by his device, adds Ada as an admin
/// and Carol as a member. Bob's `skald members acme` and `skald spaces` name no command that makes
/// a member an admin. Ada deletes her account: then both warn that Bob is the only admin, `skald
/// spaces` naming `acme`.
#[test]
fn admin_left_alone_by_another_admin_s_deletion_is_warned() {
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let bob = world.sign_up(4_305_105_001, "bob-admin-left-alone", BOB_HOST);
    let ada = world.sign_up(4_305_105_002, "ada-admin-deleted", ADA_HOST);
    let carol = world.sign_up(4_305_105_003, "carol-member-stays", CAROL_HOST);
    let space = create_space(cloud, &bob, ACME);
    let change = Change {
        members: vec![(&ada, Role::Admin), (&carol, Role::Member)],
        ..Change::default()
    };
    revise(cloud, &latest(cloud, &bob.token, space.id), &bob, &change);
    received(&bob.machine);
    for shown in [members(&bob.machine, ACME), spaces(&bob.machine)] {
        assert!(
            !shown.shown().contains(MAKE_ADMIN),
            "an only-admin warning with two admins: {shown:?}"
        );
    }

    delete_account(cloud, &ada);

    let listed = members(&bob.machine, ACME);
    assert!(warns_of_one_admin(&listed, None), "no warning: {listed:?}");
    let listed = spaces(&bob.machine);
    assert!(
        warns_of_one_admin(&listed, Some(ACME)),
        "no warning: {listed:?}"
    );
}
