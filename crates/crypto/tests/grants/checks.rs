//! Checks 1 to 3 of "Service grant": the grant names the sending device and lists the channel; its
//! signer is an admin's device in the latest revision the client accepted, a `client` or `browser`
//! device with a valid certificate of an admin account that its `excluded` does not list, and the
//! signature verifies with that certificate; `expires` has not passed.

use skald_crypto::GrantError;

use crate::cbor::Value;
use crate::fixtures::*;
use crate::grant::{Draft, NOW, judged, judged_in, signed_by, valid};

/// Fails unless the grant [`valid`] signed by `seed` fails check 2 for its signer.
#[track_caller]
fn assert_no_admins_device(seed: u8) {
    assert_eq!(
        judged(&signed_by(seed)),
        Err(GrantError::Signer(device(seed))),
        "signed by device {seed}"
    );
}

// Grants that pass.

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_grant_for_its_device_and_channel_by_an_admins_client_device_passes() {
    judged(&valid().grant()).unwrap();
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_grant_that_lists_the_channel_among_others_passes() {
    let grant = Draft {
        channels: vec![SECOND_CHANNEL, CHANNEL],
        ..valid()
    }
    .grant();

    judged(&grant).unwrap();
    judged_in(&grant, (device(BOB_CI), SECOND_CHANNEL), &current(), NOW).unwrap();
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_grant_by_an_admins_browser_passes() {
    judged(&signed_by(ALICE_BROWSER)).unwrap();
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_grant_by_a_device_of_another_admin_account_passes() {
    judged(&signed_by(BOB_LAPTOP)).unwrap();
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_grant_with_a_field_of_a_later_version_passes() {
    let grant = Draft {
        extra: vec![(5, Value::Uint(7))],
        ..valid()
    }
    .grant();

    judged(&grant).unwrap();
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_grant_passes_until_it_expires() {
    let draft = valid();
    let grant = draft.grant();

    judged_in(
        &grant,
        (device(BOB_CI), CHANNEL),
        &current(),
        draft.expires - 1,
    )
    .unwrap();
}

// Check 1: the sending device and the channel.

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_grant_in_content_from_another_device_fails() {
    let grant = valid().grant();

    // Another service device of the space, and a client device of the admin who signed it.
    for sender in [ALICE_SERVICE, ALICE_LAPTOP] {
        assert_eq!(
            judged_in(&grant, (device(sender), CHANNEL), &current(), NOW),
            Err(GrantError::Service),
            "from device {sender}"
        );
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_grant_that_names_another_service_device_fails() {
    let grant = Draft {
        service: device(DAVE_CI),
        ..valid()
    }
    .grant();

    assert_eq!(judged(&grant), Err(GrantError::Service));
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_grant_in_content_of_a_channel_it_does_not_list_fails() {
    let grant = valid().grant();
    assert_eq!(
        judged_in(&grant, (device(BOB_CI), SECOND_CHANNEL), &current(), NOW),
        Err(GrantError::Channel)
    );

    let elsewhere = Draft {
        channels: vec![SECOND_CHANNEL],
        ..valid()
    }
    .grant();
    assert_eq!(judged(&elsewhere), Err(GrantError::Channel));
}

// Check 2: the signer and the signature.

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_grant_by_a_member_that_is_no_admin_fails() {
    assert_no_admins_device(CAROL_LAPTOP);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_grant_by_an_account_outside_the_space_fails() {
    assert_no_admins_device(DAVE_LAPTOP);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_grant_by_a_service_device_of_an_admin_fails() {
    assert_no_admins_device(ALICE_SERVICE);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_grant_by_a_device_without_a_valid_certificate_fails() {
    // Only the server signed its certificate; none at all.
    assert_no_admins_device(ALICE_BY_SERVER);
    assert_no_admins_device(UNCERTIFIED);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_grant_by_a_device_chained_to_another_master_key_fails() {
    // Against alice's pinned master key, and against the master key the revision gives bob.
    assert_no_admins_device(ALICE_ROGUE);
    assert_no_admins_device(BOB_ROGUE);
    // Chained to another member's master key, or to an admin's while certified on bob's account.
    assert_no_admins_device(BOB_BY_CAROL);
    assert_no_admins_device(BOB_BY_ALICE);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_grant_by_a_device_the_revision_excludes_fails() {
    let mut latest = current();
    latest.members[0].excluded = vec![device(ALICE_BROWSER)];

    assert_eq!(
        judged_in(
            &signed_by(ALICE_BROWSER),
            (device(BOB_CI), CHANNEL),
            &latest,
            NOW
        ),
        Err(GrantError::Signer(device(ALICE_BROWSER)))
    );
    // Another device of the same admin still signs grants.
    judged_in(&valid().grant(), (device(BOB_CI), CHANNEL), &latest, NOW).unwrap();
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_grant_fails_once_a_later_revision_makes_its_signer_a_member() {
    let grant = signed_by(BOB_LAPTOP);
    let mut demoted = successor(&current());
    demoted.members[1].role = MEMBER;

    assert_eq!(
        judged_in(&grant, (device(BOB_CI), CHANNEL), &demoted, NOW),
        Err(GrantError::Signer(device(BOB_LAPTOP)))
    );
    // A grant of an admin that stays one still passes there.
    judged_in(&valid().grant(), (device(BOB_CI), CHANNEL), &demoted, NOW).unwrap();
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_grant_whose_signature_does_not_verify_with_its_signers_key_fails() {
    let mut broken = valid().grant();
    broken.signature[0] ^= 1;
    // Bob's laptop, an admin's device, signs a grant that names alice's laptop as its signer.
    let by_another = valid().signed_with(BOB_LAPTOP);
    let other_label = valid().signed_as(ALICE_LAPTOP, "skald roster");

    for grant in [broken, by_another, other_label] {
        assert_eq!(judged(&grant), Err(GrantError::Signature), "{grant:?}");
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_grant_whose_body_changed_after_signing_fails() {
    let mut later = valid().grant();
    later.body.expires += 30 * DAY;
    assert_eq!(judged(&later), Err(GrantError::Signature));

    let mut wider = Draft {
        channels: vec![SECOND_CHANNEL, CHANNEL],
        ..valid()
    }
    .grant();
    let narrow = valid().grant();
    wider.signature = narrow.signature;
    assert_eq!(judged(&wider), Err(GrantError::Signature));
}

// Check 3: the expiry.

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_grant_fails_after_it_expires() {
    let draft = valid();
    let grant = draft.grant();
    assert_eq!(
        judged_in(
            &grant,
            (device(BOB_CI), CHANNEL),
            &current(),
            draft.expires + 1
        ),
        Err(GrantError::Expired)
    );

    let expired = Draft {
        expires: NOW - DAY,
        ..valid()
    }
    .grant();
    assert_eq!(judged(&expired), Err(GrantError::Expired));
}
