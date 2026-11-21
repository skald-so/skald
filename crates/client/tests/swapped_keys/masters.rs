//! The revisions a device accepted in its spaces give one account different master keys, or its
//! own account one other than the one it pinned: the device warns once, and checks each space's
//! devices against that space's revisions.

use skald_crypto::SigningKey;
use skald_proto::{AccountId, Member, Role};

use crate::devices::second_machine;
use crate::world::{
    ACME, GLOBEX, Person, World, create, members, next_by, plain_lines, store, warnings, words,
};
use crate::{ADA_HOST, BOB_HOST, Finished, shown_in_members};

/// What the warning of an account's master keys says.
const MASTER: &str = "master";

/// The host name of Bob's second machine.
const BOB_LAPTOP: &str = "bob-laptop";

/// The warning lines of `shown` that say `master` and hold `person`'s handle or account ID.
fn master_warnings(shown: &Finished, person: &Person) -> Vec<String> {
    let handle = person.handle.to_string().to_lowercase();
    let account = person.account.to_string().to_lowercase();
    warnings(shown)
        .into_iter()
        .filter(|line| {
            line.contains(MASTER)
                && (words(line).contains(&handle.as_str()) || line.contains(&account))
        })
        .collect()
}

/// `person`'s entry in a roster, with `role` and the master key `master`.
fn member_with(person: &Person, role: Role, master: [u8; 32]) -> Member {
    Member {
        master,
        ..person.member(role)
    }
}

/// Sets the master key of `account` in `members` to `master`.
fn set_master(members: &mut [Member], account: AccountId, master: [u8; 32]) {
    members
        .iter_mut()
        .find(|member| member.account == account)
        .expect("a member")
        .master = master;
}

/// Bob's machine creates `acme` and `globex`. Revision 3 of `acme`, signed by his device, adds Ada
/// with her master key; revision 3 of `globex` adds her with another key. Over four runs of `skald
/// members`, `acme`, `globex`, then both again, Bob's machine warns of Ada's master keys once. In
/// `acme` it shows Ada's machine, whose certificate chains to her master key; in `globex`, never.
#[test]
fn spaces_that_give_one_account_two_master_keys_warn_once() {
    let world = World::start();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_957_206_001, "ada-two-masters", ADA_HOST);
    let bob = world.sign_up(4_957_206_002, "bob-sees-two-masters", BOB_HOST);
    let acme = create(cloud, &bob, ACME);
    let globex = create(cloud, &bob, GLOBEX);
    let genuine = next_by(&acme, &bob, |members| {
        members.push(ada.member(Role::Member))
    });
    let other = SigningKey::generate().public_key();
    let swapped = next_by(&globex, &bob, |members| {
        members.push(member_with(&ada, Role::Member, other));
    });
    store(cloud, &bob.token, &genuine);
    store(cloud, &bob.token, &swapped);

    let shown = [ACME, GLOBEX, ACME, GLOBEX].map(|space| members(&bob.machine, space));

    let warned: Vec<String> = shown
        .iter()
        .flat_map(|finished| master_warnings(finished, &ada))
        .collect();
    assert_eq!(
        warned.len(),
        1,
        "not one warning of Ada's master keys: {shown:?}"
    );
    for in_acme in [&shown[0], &shown[2]] {
        assert!(
            shown_in_members(in_acme, ada.device, ADA_HOST),
            "Ada's machine is not shown in acme: {in_acme:?}"
        );
    }
    for in_globex in [&shown[1], &shown[3]] {
        assert!(
            !(plain_lines(in_globex).iter()).any(|line| words(line).contains(&ADA_HOST)),
            "Ada's machine is shown in globex: {in_globex:?}"
        );
    }
}

/// Bob's machine creates `acme`, and revision 3, signed by his device, lists his own account with
/// another master key: his machine warns of it. A second machine of his account, linked by hand
/// afterwards and starting from revision 3, warns of it too, though no revision it accepts changes
/// the key.
#[test]
fn revision_that_gives_the_own_account_another_master_key_warns() {
    let world = World::start();
    let cloud = &world.cloud;
    let bob = world.sign_up(4_957_207_001, "bob-own-master", BOB_HOST);
    let acme = create(cloud, &bob, ACME);
    let other = SigningKey::generate().public_key();
    let third = next_by(&acme, &bob, |members| {
        set_master(members, bob.account, other);
    });
    store(cloud, &bob.token, &third);

    let first = members(&bob.machine, ACME);
    let laptop = second_machine(&world, &bob, BOB_LAPTOP);
    let second = members(&laptop.machine, ACME);

    for shown in [&first, &second] {
        assert!(
            warnings(shown).iter().any(|line| line.contains(MASTER)),
            "no warning of the own account's master key: {shown:?}"
        );
    }
}
