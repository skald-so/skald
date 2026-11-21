//! A verified account whose master key changes is no longer verified, and the device warns; a
//! verification shows only on the device that made it.

use skald_crypto::SigningKey;

use crate::devices::second_machine;
use skald_proto::Roster;

use crate::terminal::Terminal;
use crate::world::{ACME, members, next_by, store, warnings, words};
use crate::{
    FINGERPRINT, Pair, YES, assert_member, fingerprints_in, member_block, says_verified, shown_for,
};

/// The host name of Bob's second machine.
const BOB_LAPTOP: &str = "bob-laptop";

/// Bob verified Ada; a fixture revision 4, signed by his device, changes her `master`. His next
/// `skald members` warns, naming her, that a verified account's key changed, and shows her with
/// the fingerprint of the new key, not verified, also in a later run.
#[test]
fn a_verified_accounts_changed_master_key_is_warned_of_and_no_longer_verified() {
    let pair = Pair::new(
        [4_181_108_001, 4_181_108_002],
        ["ada-whose-key-changes", "bob-sees-a-key-change"],
    );
    let bob = &pair.bob;
    let handle = pair.ada();
    let handles = pair.handles();
    pair.bob_verifies_ada();
    pair.assert_ada_on(&bob.machine, true);

    stored_with_adas_master(&pair, &pair.third, SigningKey::generate().public_key());
    let shown = members(&bob.machine, ACME);

    assert!(
        warnings(&shown)
            .iter()
            .any(|line| words(line).contains(&handle) && line.contains("verified")),
        "no warning that {handle}'s verified key changed: {shown:?}"
    );
    let block = member_block(&shown, handle, &handles);
    assert!(!says_verified(&block), "{handle} still verified: {shown:?}");
    let now = fingerprints_in(&block);
    assert_eq!(now.len(), 1, "not one fingerprint of {handle}: {shown:?}");
    assert_ne!(
        now[0], pair.adas,
        "the fingerprint of the old key: {shown:?}"
    );
    assert!(
        fingerprints_in(&shown_for(&bob.machine, handle).shown()).contains(&now[0]),
        "skald fingerprint shows another than skald members"
    );
    let later = members(&bob.machine, ACME);
    assert_member(&later, (handle, &handles), &now[0], false);
}

/// Bob verified Ada; fixture revisions 4 and 5, signed by his device, change her `master` and
/// change it back. His machine accepts each; then it shows her not verified.
#[test]
fn a_master_key_changed_back_is_not_verified_again() {
    let pair = Pair::new(
        [4_181_111_001, 4_181_111_002],
        ["ada-whose-key-changes-back", "bob-sees-it-change-back"],
    );
    let adas = adas_master(&pair.third, &pair);
    pair.bob_verifies_ada();

    let fourth = stored_with_adas_master(&pair, &pair.third, SigningKey::generate().public_key());
    members(&pair.bob.machine, ACME);
    stored_with_adas_master(&pair, &fourth, adas);

    pair.assert_ada_on(&pair.bob.machine, false);
}

/// Bob's machine asks him about Ada's fingerprint; before he answers yes, a fixture revision 4
/// changes her `master` and his machine accepts it. She is verified for neither key: not with
/// the new one, nor once a revision 5 changes it back.
#[test]
fn a_yes_after_the_master_key_changed_marks_neither_key() {
    let pair = Pair::new(
        [4_181_112_001, 4_181_112_002],
        ["ada-whose-key-changes-unseen", "bob-says-yes-too-late"],
    );
    let (bob, handle, handles) = (&pair.bob, pair.ada(), pair.handles());
    let adas = adas_master(&pair.third, &pair);
    let mut terminal = Terminal::start(&bob.machine, &[FINGERPRINT, ACME, handle]);
    terminal.wait_for("Ada's fingerprint and a question", |shown| {
        fingerprints_in(shown).contains(&pair.adas) && shown.contains('?')
    });
    let fourth = stored_with_adas_master(&pair, &pair.third, SigningKey::generate().public_key());
    let changed = members(&bob.machine, ACME);
    let new = fingerprints_in(&member_block(&changed, handle, &handles));
    let [new] = new.as_slice() else {
        panic!("not one fingerprint of {handle}: {changed:?}");
    };
    assert_ne!(*new, pair.adas, "the key did not change: {changed:?}");

    terminal.type_line(YES);
    terminal.finish();

    assert_member(&members(&bob.machine, ACME), (handle, &handles), new, false);
    stored_with_adas_master(&pair, &fourth, adas);
    pair.assert_ada_on(&bob.machine, false);
}

/// Ada's `master` in `roster`.
fn adas_master(roster: &Roster, pair: &Pair) -> [u8; 32] {
    (roster.body().members.iter())
        .find(|member| member.account == pair.ada.account)
        .expect("Ada")
        .master
}

/// Stores the revision after `previous`, signed by Bob's device, that gives Ada `master`; gives it.
fn stored_with_adas_master(pair: &Pair, previous: &Roster, master: [u8; 32]) -> Roster {
    let next = next_by(previous, &pair.bob, |members| {
        members
            .iter_mut()
            .find(|member| member.account == pair.ada.account)
            .expect("Ada")
            .master = master;
    });
    store(&pair.world.cloud, &pair.bob.token, &next);
    next
}

/// Bob's second machine, linked by hand, shows Ada not verified after his first machine verified
/// her, and his first machine still shows her verified.
#[test]
fn a_verification_on_one_device_shows_on_no_other() {
    let pair = Pair::new(
        [4_181_109_001, 4_181_109_002],
        ["ada-verified-on-one-device", "bob-with-two-devices"],
    );
    let laptop = second_machine(&pair.world, &pair.bob, BOB_LAPTOP);
    pair.assert_ada_on(&laptop.machine, false);

    pair.bob_verifies_ada();

    pair.assert_ada_on(&laptop.machine, false);
    pair.assert_ada_on(&pair.bob.machine, true);
}
