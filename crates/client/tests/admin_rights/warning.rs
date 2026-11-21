//! The only-admin warning: on a device of the only admin of a space with another member, `skald
//! members` and `skald spaces` warn, naming the command that makes a member an admin; a space with
//! no other member, the personal space among them, or with a second admin gets no warning.

use skald_proto::Role;

use crate::fixtures::fronted;
use crate::machine::Finished;
use crate::roles::{MAKE_ADMIN, RECOVERY_KEY, RoleCommand, role_changed};
use crate::talk::{ADA_HOST, BOB_HOST};
use crate::world::{ACME, ADMIN, create, members, next_by, spaces, store, warnings, words};

/// Fails unless `shown` warns that its space has one admin: a warning line holding `admin`, and,
/// with `space`, the space's name as a word, and the output holding the recovery key and the
/// command that makes a member an admin.
#[track_caller]
fn assert_warns(shown: &Finished, space: Option<&str>) {
    let lines = warnings(shown);
    let of_space: Vec<&String> = (lines.iter())
        .filter(|line| line.contains(ADMIN))
        .filter(|line| space.is_none_or(|name| words(line).contains(&name)))
        .collect();
    assert!(!of_space.is_empty(), "no only-admin warning: {shown:?}");
    let text = shown.shown().to_lowercase();
    for part in [RECOVERY_KEY, MAKE_ADMIN] {
        assert!(
            text.contains(part),
            "the warning does not say {part}: {shown:?}"
        );
    }
}

/// Fails if `shown` names the command that makes a member an admin.
#[track_caller]
fn assert_no_warning(shown: &Finished) {
    assert!(
        !shown.shown().contains(MAKE_ADMIN),
        "an only-admin warning: {shown:?}"
    );
}

/// Ada's machine created `acme`. With Ada alone in it and in her personal space, neither `skald
/// members` nor `skald spaces` warns. Once her device's revision adds Bob as a member, both warn,
/// `skald spaces` naming `acme`. Once her `skald make-admin acme <bob>` makes Bob an admin, neither
/// warns.
#[test]
fn only_admin_of_a_space_with_another_member_is_warned_until_a_second_admin() {
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_301_104_001, "ada-alone-at-first", ADA_HOST);
    let bob = world.sign_up(4_301_104_002, "bob-joins-ada", BOB_HOST);
    let created = create(cloud, &ada, ACME);
    let personal = ada.handle.as_str();
    for shown in [
        members(&ada.machine, ACME),
        members(&ada.machine, personal),
        spaces(&ada.machine),
    ] {
        assert_no_warning(&shown);
    }

    let with_bob = next_by(&created, &ada, |members| {
        members.push(bob.member(Role::Member));
    });
    store(cloud, &ada.token, &with_bob);

    assert_warns(&members(&ada.machine, ACME), None);
    assert_warns(&spaces(&ada.machine), Some(ACME));

    role_changed(
        cloud,
        (&ada, RoleCommand::MakeAdmin, &bob),
        (created.body().space, ACME),
        &[&bob],
    );

    for shown in [members(&ada.machine, ACME), spaces(&ada.machine)] {
        assert_no_warning(&shown);
    }
}
