//! Encoded maps of fixture fields, changed field by field, the fields of content, history bodies
//! and signed objects, for the tests of the history's objects. Uses the tests' CBOR writer,
//! `crate::cbor`.

use std::fmt::Debug;

use skald_proto::{Cbor, ChannelId, DeviceId, MessageId, Signed, SignedBody};

use crate::cbor::{Value, bytes, text};

/// The time a history body states, in seconds since the Unix epoch.
pub const SENT: u64 = 1_800_000_123;

/// The encoded map of `fields`.
pub fn map(fields: &[(i64, Value)]) -> Vec<u8> {
    Value::Map(fields.to_vec()).encode()
}

/// The encoded map of `fields` with `value` at `key` in place of the field there.
pub fn with(fields: &[(i64, Value)], key: i64, value: Value) -> Vec<u8> {
    let mut fields: Vec<_> = fields.iter().filter(|(k, _)| *k != key).cloned().collect();
    fields.push((key, value));
    map(&fields)
}

/// The encoded map of `fields` without the field at `key`.
pub fn without(fields: &[(i64, Value)], key: i64) -> Vec<u8> {
    let fields: Vec<_> = fields.iter().filter(|(k, _)| *k != key).cloned().collect();
    map(&fields)
}

/// Each prefix of `encoded`, and `encoded` with a byte after it.
pub fn truncated_and_longer(encoded: &[u8]) -> Vec<Vec<u8>> {
    let mut inputs: Vec<Vec<u8>> = (0..encoded.len())
        .map(|at| encoded[..at].to_vec())
        .collect();
    inputs.push([encoded, &[0]].concat());
    inputs
}

/// The fields of content with an A2A message, as in `docs/protocol/test-vectors.md`, "Content
/// with an A2A message and an attachment", without the attachment, its text `said`.
pub fn a2a_fields(said: &str) -> Vec<(i64, Value)> {
    let message = format!(
        r#"{{"messageId":"019a6f20-1a00-7c00-8a00-000000000004","contextId":"019a6f20-0000-7000-8000-00000000c0de","role":"ROLE_USER","parts":[{{"text":"{said}"}}]}}"#
    );
    vec![
        (1, text("acme/alice/claude-api@skald.example")),
        (2, text("acme/bob/codex@skald.example")),
        (3, Value::Uint(7)),
        (4, Value::Uint(1)),
        (6, text(&message)),
    ]
}

/// Map fields of a later version, which decoding keeps and encoding gives back.
pub fn unknown() -> Vec<(i64, Value)> {
    vec![(-3, text("later")), (24, Value::Bool(true))]
}

/// An encoded signed object, such as a `history-plaintext` or a `history-wrap`: `body` and its
/// `signature`.
pub fn signed_cbor(body: &[u8], signature: &[u8]) -> Vec<u8> {
    Value::Array(vec![bytes(body), bytes(signature)]).encode()
}

/// Checks that the signed object of the encoded `body`, decoded as `decoded`, and a signature
/// decodes into them and encodes back byte for byte: a `history-plaintext` or a `history-wrap`.
pub fn assert_signed_round_trip<B: SignedBody + PartialEq + Debug>(body: &[u8], decoded: &B) {
    let encoded = signed_cbor(body, &[0x5a; 64]);
    let signed = Signed::<B>::from_cbor(&encoded).expect("the signed object decodes");
    assert_eq!(signed.body(), decoded);
    assert_eq!(signed.body_bytes(), body);
    assert_eq!(signed.signature(), &[0x5a; 64]);
    assert_eq!(
        signed.to_cbor(),
        encoded,
        "the signed object, byte for byte"
    );
}

/// An encoded `history-body` of these fields, `content` already encoded, and `unknown` ones.
pub fn body_cbor(
    sender: DeviceId,
    channel: ChannelId,
    msg_id: MessageId,
    content: &[u8],
    unknown: Vec<(i64, Value)>,
) -> Vec<u8> {
    let mut fields = vec![
        (1, bytes(sender.as_bytes())),
        (2, bytes(channel.as_bytes())),
        (3, bytes(msg_id.as_bytes())),
        (4, Value::Uint(SENT)),
        (5, bytes(content)),
    ];
    fields.extend(unknown);
    Value::Map(fields).encode()
}
