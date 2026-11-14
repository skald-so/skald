//! A wrap of an epoch's history key: `docs/protocol/membership.md`, "Wraps", byte for byte. The
//! acceptance tests of the wraps are in `crates/crypto/tests/wraps/`.

mod common;

use common::{alice_device, bob_device, channel, hex, to_hex};
use skald_proto::{AccountId, Cbor, HistoryWrapBody, Unknown, WrapRecipient};

const ACCOUNT: AccountId = AccountId::from_bytes([0xac; 16]);

fn recipients() -> [(WrapRecipient, String); 2] {
    [
        (
            WrapRecipient::Browser(bob_device()),
            format!("82 01 50{}", to_hex(bob_device().as_bytes())),
        ),
        (
            WrapRecipient::Recovery(ACCOUNT),
            format!("82 02 50{}", "ac".repeat(16)),
        ),
    ]
}

#[test]
fn a_wrap_body_encodes_as_its_cddl_says() {
    for (recipient, recipient_hex) in recipients() {
        let body = HistoryWrapBody {
            channel: channel(),
            epoch: 7,
            recipient,
            signer: alice_device(),
            enc: [0xe1; 32],
            ct: vec![0xc7; 3],
            unknown: Unknown::default(),
        };
        let encoded = hex(&format!(
            "a6 0150{channel} 0207 03{recipient_hex} 0450{signer} 055820{enc} 0643c7c7c7",
            channel = to_hex(channel().as_bytes()),
            signer = to_hex(alice_device().as_bytes()),
            enc = "e1".repeat(32),
        ));
        assert_eq!(body.to_cbor(), encoded);
        assert_eq!(HistoryWrapBody::from_cbor(&encoded), Ok(body));
    }
}

#[test]
fn the_associated_data_of_a_wrap_is_its_channel_epoch_and_recipient() {
    for (recipient, recipient_hex) in recipients() {
        assert_eq!(
            HistoryWrapBody::associated_data(channel(), 7, &recipient),
            hex(&format!(
                "83 50{channel} 07 {recipient_hex}",
                channel = to_hex(channel().as_bytes()),
            ))
        );
    }
}
