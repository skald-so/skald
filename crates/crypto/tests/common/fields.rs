//! Encoded maps of fixture fields, changed field by field, and the fields of content, for the
//! tests of the history's objects. Uses the tests' CBOR writer, `crate::cbor`.

use crate::cbor::{Value, text};

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
