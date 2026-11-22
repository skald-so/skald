//! Master keys: a member's `master` that changes, also a verified one's; one account with two
//! master keys in two spaces; the device's own account with another master key than it pinned.

use skald_proto::Role;

use crate::terminal::Terminal;
use crate::world::{ACME, GLOBEX, MEMBERS, World, create, members, next_by, store};
use crate::{
    BOB_HOST, Pair, assert_says_none, assert_waits_and_shows_once, member_with_master, other_master,
};

/// The command that shows a fingerprint and asks whether it matches, and the person's answer.
const FINGERPRINT: &str = "fingerprint";
const YES: &str = "yes";

/// What the key change of a verified account says.
const VERIFIED: &str = "verified";

/// What the key change of the own account's master key says.
const MASTER: &str = "master";

/// A fixture revision 4, signed by Bob's device, gives Ada another `master`; Bob's `skald members
/// acme` accepts it.
fn adas_master_changes(pair: &Pair) {
    let fourth = next_by(&pair.third, &pair.bob, |list| {
        let ada = (list.iter_mut())
            .find(|member| member.account == pair.ada.account)
            .expect("Ada");
        ada.master = other_master();
    });
    store(&pair.world.cloud, &pair.bob.token, &fourth);
    members(&pair.bob.machine, ACME);
}

/// A revision that changes Ada's `master` leaves a key change on Bob's machine that waits and
/// shows once, naming her.
#[test]
fn a_members_changed_master_key_waits_until_seen() {
    let pair = Pair::new(
        [4_188_101_001, 4_188_101_002],
        ["ada-whose-master-changes", "bob-sees-adas-master-change"],
    );

    adas_master_changes(&pair);

    let machine = &pair.bob.machine;
    assert_waits_and_shows_once(machine, &[MEMBERS, ACME], &pair.adas_names());
}

/// Bob verified Ada at a terminal; a revision that changes her `master` leaves a key change that
/// waits and shows once, naming her and saying `verified`.
#[test]
fn a_verified_members_changed_master_key_waits_until_seen() {
    let pair = Pair::new(
        [4_188_102_001, 4_188_102_002],
        ["ada-trusted-then-changed", "bob-checked-ada-before"],
    );
    let machine = &pair.bob.machine;
    let mut asked = Terminal::start(machine, &[FINGERPRINT, ACME, pair.ada.handle.as_str()]);
    asked.wait_to_show("?");
    asked.type_line(YES);
    let verified = asked.finish();
    assert!(verified.success, "{}", verified.text);
    assert_says_none(&verified.text);

    adas_master_changes(&pair);

    let shown = assert_waits_and_shows_once(machine, &[MEMBERS, ACME], &pair.adas_names());
    assert!(
        shown.text.to_lowercase().contains(VERIFIED),
        "the key change does not say Ada was verified: {}",
        shown.text
    );
}

/// Bob's machine creates `acme` and `globex`; revision 3 of `acme` adds Ada with her master key and
/// revision 3 of `globex` with another. His machine accepts both: a key change waits and shows
/// once, naming Ada.
#[test]
fn one_account_with_two_master_keys_waits_until_seen() {
    let pair = Pair::new(
        [4_188_103_001, 4_188_103_002],
        ["ada-with-two-masters", "bob-sees-two-masters-of-ada"],
    );
    let (cloud, bob) = (&pair.world.cloud, &pair.bob);
    let second = create(cloud, bob, GLOBEX);
    let third = next_by(&second, bob, |list| {
        list.push(member_with_master(&pair.ada, Role::Member, other_master()));
    });
    store(cloud, &bob.token, &third);

    members(&bob.machine, GLOBEX);

    assert_waits_and_shows_once(&bob.machine, &[MEMBERS, ACME], &pair.adas_names());
}

/// Revision 3 of Bob's `acme`, signed by his device, lists his own account with another master key
/// than the one his machine pinned: a key change waits and shows once, saying `master`.
#[test]
fn the_own_account_with_another_master_key_waits_until_seen() {
    let world = World::start();
    let bob = world.sign_up(4_188_104_001, "bob-own-master-swapped", BOB_HOST);
    let second = create(&world.cloud, &bob, ACME);
    assert_says_none(&members(&bob.machine, ACME).shown());
    let third = next_by(&second, &bob, |list| {
        let own = (list.iter_mut())
            .find(|member| member.account == bob.account)
            .expect("Bob");
        own.master = other_master();
    });
    store(&world.cloud, &bob.token, &third);

    members(&bob.machine, ACME);

    assert_waits_and_shows_once(&bob.machine, &[MEMBERS, ACME], &[MASTER.to_owned()]);
}
