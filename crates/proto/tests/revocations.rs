//! A revocation's body decodes into the fields it encodes, and the revocations a Commit carries
//! round-trip: `docs/protocol/membership.md`, "Revocation" and "Group changes". The acceptance
//! tests of revocations are in `crates/crypto/tests/revocations/`.

mod common;

use common::{account, alice_device, bob_device, server_device};
use skald_proto::{Cbor, NonEmpty, Revocation, RevocationBody, Revocations, Unknown};

#[test]
fn a_revocation_body_decodes_into_the_fields_it_encodes() {
    let ids = || Some(NonEmpty::new(server_device()));
    for (signer, also) in [(Some(alice_device()), None), (None, ids())] {
        let body = RevocationBody {
            account: account(),
            device: bob_device(),
            keep: ids(),
            signer,
            timestamp: 1_762_819_200,
            also,
            unknown: Unknown::default(),
        };
        assert_eq!(RevocationBody::from_cbor(&body.to_cbor()), Ok(body));
    }
}

/// A revocation of `device` by Alice's device, its signature `byte` repeated.
fn revocation(device: skald_proto::DeviceId, byte: u8) -> Revocation {
    let body = RevocationBody {
        account: account(),
        device,
        keep: None,
        signer: Some(alice_device()),
        timestamp: 1_762_819_200,
        also: None,
        unknown: Unknown::default(),
    };
    Revocation::from_parts(body.to_cbor(), [byte; 64]).expect("a revocation")
}

#[test]
fn revocations_a_commit_carries_round_trip_as_an_array_of_revocations() {
    let carried = Revocations(
        NonEmpty::from_vec(vec![
            revocation(bob_device(), 1),
            revocation(server_device(), 2),
        ])
        .expect("revocations"),
    );
    let bytes = carried.to_cbor();
    let mut expected = vec![0x82];
    for one in carried.0.iter() {
        expected.extend(one.to_cbor());
    }

    assert_eq!(bytes, expected);
    assert_eq!(Revocations::from_cbor(&bytes), Ok(carried));
}

#[test]
fn revocations_a_commit_carries_are_refused_unless_a_non_empty_array_of_revocations() {
    let one = revocation(bob_device(), 1).to_cbor();
    let mut cut = [&[0x81][..], &one].concat();
    cut.pop();
    for bytes in [
        vec![0x80],
        one.clone(),
        [&[0x81][..], &one[..1]].concat(),
        cut,
        [&[0x81][..], &one, &[0x00]].concat(),
        [&[0x81][..], &[0x01]].concat(),
        Vec::new(),
    ] {
        assert!(Revocations::from_cbor(&bytes).is_err(), "{bytes:?}");
    }
}
