//! Signing a revision with a device's signing key over the label `skald roster`
//! (`membership.md`, "Signed objects" and "Roster"), and a new space's revision 1.

use skald_crypto::{check_roster, sign_roster, verify};
use skald_proto::{Cbor, RosterBody};

use crate::fixtures::*;

#[test]
fn signing_a_revision_signs_its_encoded_body_over_the_roster_label() {
    let rev = next();
    let body = RosterBody::from_cbor(&rev.body()).unwrap();
    let roster = sign_roster(&body, &key(ALICE_LAPTOP)).unwrap();

    assert_eq!(roster.body(), &body);
    assert_holds(&roster, &rev);
    assert!(
        verify(
            &public(ALICE_LAPTOP),
            "skald roster",
            &[roster.body_bytes()],
            roster.signature()
        )
        .is_ok()
    );
}

#[test]
fn a_signed_revision_one_the_client_builds_passes_checks_3_and_4() {
    let roster = sign_roster(&first_body(), &key(ALICE_LAPTOP)).unwrap();
    check_roster(&roster, &certs()).unwrap();
    check_roster(&roster, &[]).unwrap();
}

#[test]
fn revision_two_after_a_revision_one_the_client_built_is_accepted() {
    let first = sign_roster(&first_body(), &key(ALICE_LAPTOP)).unwrap();
    let mut body = RosterBody::from_cbor(first.body_bytes()).unwrap();
    body.revision = 2;
    body.prev = Some(first.hash());
    let second = sign_roster(&body, &key(ALICE_LAPTOP)).unwrap();
    skald_crypto::accept_next(&first, &second, &certs(), ALICE, &public(ALICE_MASTER)).unwrap();

    // Signed by a device of an account outside the space: alice is its only admin.
    let mut body = RosterBody::from_cbor(first.body_bytes()).unwrap();
    body.revision = 2;
    body.prev = Some(first.hash());
    body.signer = device(BOB_LAPTOP);
    let by_bob = sign_roster(&body, &key(BOB_LAPTOP)).unwrap();
    assert!(
        skald_crypto::accept_next(&first, &by_bob, &certs(), ALICE, &public(ALICE_MASTER)).is_err()
    );
}
