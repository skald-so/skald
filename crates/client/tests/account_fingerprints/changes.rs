//! A verified account whose master key changes is no longer verified, and the device warns; a
//! verification shows only on the device that made it.

use skald_crypto::SigningKey;

use crate::devices::second_machine;
use crate::world::{ACME, members, next_by, store, warnings, words};
use crate::{Pair, assert_member, fingerprints_in, member_block, says_verified, shown_for};

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
    let (ada, bob) = (&pair.ada, &pair.bob);
    let handle = pair.ada();
    let handles = pair.handles();
    pair.bob_verifies_ada();
    pair.assert_ada_on(&bob.machine, true);

    let fourth = next_by(&pair.third, bob, |members| {
        members
            .iter_mut()
            .find(|member| member.account == ada.account)
            .expect("Ada")
            .master = SigningKey::generate().public_key();
    });
    store(&pair.world.cloud, &bob.token, &fourth);
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
