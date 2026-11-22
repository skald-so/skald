//! Check 2 with the revocations the server returns: a revoked device is no admin's device, so a
//! grant it signed fails, also one signed before its revocation. Check 3 at the expiry's second.

use skald_crypto::{GrantError, sign_revocation};
use skald_proto::{AccountId, Revocation, RevocationBody, Unknown};

use crate::fixtures::*;
use crate::grant::{NOW, judged_revoking, signed_by, valid};

/// The revocation of the device `seed` of `account` by the account's master key `master`.
fn revoked(account: AccountId, seed: u8, master: u8) -> Revocation {
    let body = RevocationBody {
        account,
        device: device(seed),
        keep: None,
        signer: None,
        timestamp: TIMESTAMP,
        also: None,
        unknown: Unknown::default(),
    };
    sign_revocation(&body, &key(master)).expect("a revocation")
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_grant_by_a_revoked_device_fails() {
    // On alice's account, checked against her pinned master key, and on bob's, against the master
    // key the revision gives him.
    let revocations = [
        revoked(ALICE, ALICE_BROWSER, ALICE_MASTER),
        revoked(BOB, BOB_LAPTOP, BOB_MASTER),
    ];
    let judged = |grant| {
        judged_revoking(
            &grant,
            (device(BOB_CI), CHANNEL),
            &current(),
            &revocations,
            NOW,
        )
    };

    for seed in [ALICE_BROWSER, BOB_LAPTOP] {
        assert_eq!(
            judged(signed_by(seed)),
            Err(GrantError::Signer(device(seed))),
            "signed by device {seed}"
        );
    }
    // A device of the same admin that is not revoked still signs grants.
    judged(valid().grant()).unwrap();
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_grant_passes_in_its_expiry_second() {
    let draft = valid();

    judged_revoking(
        &draft.grant(),
        (device(BOB_CI), CHANNEL),
        &current(),
        &[],
        draft.expires,
    )
    .unwrap();
}
