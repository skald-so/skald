//! The proof of an HTTPS request, `docs/protocol/delivery.md`, "Connecting": `[challenge: bstr,
//! device: id, signature: bstr .size 64]`. It round-trips byte for byte, and an `auth` frame, a
//! short signature or a missing item is an error.

mod common;

use common::{alice_device, hex, to_hex};
use skald_proto::{Cbor, RequestProof};

/// Alice's device ID, in hex.
const ALICE: &str = "19ac5f784938823586a6c5809a18f6dd";

fn proof(text: &str) -> Option<RequestProof> {
    RequestProof::from_cbor(&hex(text)).ok()
}

#[test]
fn a_proof_round_trips() {
    let proof = RequestProof {
        challenge: vec![7; 16],
        device: alice_device(),
        signature: [0x5a; 64],
    };
    let text = format!("83 50{} 50{ALICE} 5840{}", "07".repeat(16), "5a".repeat(64));

    assert_eq!(to_hex(&proof.to_cbor()), to_hex(&hex(&text)));
    assert_eq!(RequestProof::from_cbor(&hex(&text)), Ok(proof));
}

#[test]
fn an_auth_frame_a_short_signature_or_a_missing_item_is_no_proof() {
    let challenge = format!("50{}", "07".repeat(16));
    for text in [
        format!("84 0a {challenge} 50{ALICE} 5840{}", "5a".repeat(64)),
        format!("83 {challenge} 50{ALICE} 583f{}", "5a".repeat(63)),
        format!("82 {challenge} 50{ALICE}"),
    ] {
        assert_eq!(proof(&text), None, "{text}");
    }
}
