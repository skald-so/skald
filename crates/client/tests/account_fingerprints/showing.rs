//! Each device shows the fingerprint of its own account and of its spaces' members, and the two
//! people's devices show the same one for an account.

use skald_crypto::account_fingerprint;

use crate::world::{ACME, members};
use crate::{Pair, fingerprints_in, member_block, own_fingerprint, shown_for};

/// Ada's fingerprint on her device, that of her account ID and master key, equals the one Bob's
/// device shows for her, by `skald fingerprint acme <handle>` and in `skald members`; Bob's own
/// differs, and `skald members` shows it for him. No account is verified yet.
#[test]
fn a_members_fingerprint_on_another_device_equals_the_one_on_its_own() {
    let pair = Pair::new(
        [4_181_101_001, 4_181_101_002],
        ["ada-compares-fingerprints", "bob-compares-fingerprints"],
    );
    let (ada, bob) = (&pair.ada, &pair.bob);

    let bobs = own_fingerprint(bob);
    let for_ada = shown_for(&bob.machine, pair.ada());
    let listed = members(&bob.machine, ACME);

    assert_eq!(pair.adas, account_fingerprint(ada.account, &ada.master));
    assert_eq!(bobs, account_fingerprint(bob.account, &bob.master));
    assert_ne!(pair.adas, bobs, "two accounts with one fingerprint");
    pair.assert_shows_adas(&for_ada.shown());
    pair.assert_ada(&listed, false);
    let block = member_block(&listed, bob.handle.as_str(), &pair.handles());
    assert!(
        fingerprints_in(&block).contains(&bobs),
        "Bob's own fingerprint {bobs:?} is not shown: {listed:?}"
    );
}
