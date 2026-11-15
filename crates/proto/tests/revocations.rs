//! A revocation's body decodes into the fields it encodes: `docs/protocol/membership.md`,
//! "Revocation". The acceptance tests of revocations are in `crates/crypto/tests/revocations/`.

mod common;

use common::{account, alice_device, bob_device, server_device};
use skald_proto::{Cbor, NonEmpty, RevocationBody, Unknown};

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
