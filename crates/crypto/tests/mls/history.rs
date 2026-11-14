//! A channel's history, apart from the crate under test: `docs/protocol/membership.md`, "Key
//! chain" and "History entry". AES-128-GCM is that of the profile's ciphersuite on mls-rs's
//! RustCrypto provider, objects and associated data are encoded with the tests' CBOR writer, and
//! signatures are made over `[label, body]` with the tests' keys.

use mls_rs::{CipherSuite, CipherSuiteProvider, CryptoProvider};
use mls_rs_crypto_rustcrypto::RustCryptoProvider;
use skald_crypto::{HistoryKey, SealedState};
use skald_proto::{Cbor, ChannelId, DeviceId, HistoryBody, HistoryEntry, HistoryLink, MessageId};

use crate::cbor::{Value, bytes, text};
use crate::commits::{empty, process, store};
use crate::fields::a2a_fields;
use crate::fixtures::{admitted, device_id, signing_key};
use crate::steps;

/// The label of a history plaintext's signature.
pub const LABEL: &str = "skald history";
/// The time a history body states, in seconds since the Unix epoch.
pub const SENT: u64 = 1_800_000_123;

/// Map fields of a later version, which decoding keeps and encoding gives back.
pub fn unknown() -> Vec<(i64, Value)> {
    vec![(-3, text("later")), (24, Value::Bool(true))]
}

/// The history key of `channel` in `epoch` as device `seed` holds it in `state`.
pub fn key(seed: u8, state: &SealedState, channel: ChannelId, epoch: u64) -> HistoryKey {
    HistoryKey::from_bytes(&steps::history_key(seed, state, channel, epoch))
}

/// Device `seeds[0]` commits an empty Commit, the server stores it and every other device of
/// `seeds` processes it: the next epoch, with the same members.
pub fn next_epoch(seeds: &[u8], states: &mut [SealedState], channel: ChannelId) {
    let commit = empty(seeds[0], &mut states[0], channel, b"");
    store(seeds[0], &mut states[0], channel);
    for (&seed, state) in seeds.iter().zip(states.iter_mut()).skip(1) {
        process(seed, state, channel, &commit);
    }
}

fn aead() -> impl CipherSuiteProvider {
    RustCryptoProvider::new()
        .cipher_suite_provider(CipherSuite::CURVE25519_AES128)
        .expect("the profile's ciphersuite")
}

/// AES-128-GCM of `plaintext` under `key` with `nonce` and associated data `aad`.
pub fn seal(key: &[u8; 16], nonce: &[u8; 12], aad: &[u8], plaintext: &[u8]) -> Vec<u8> {
    aead()
        .aead_seal(key, plaintext, Some(aad), nonce)
        .expect("AES-128-GCM seals")
}

/// The AES-128-GCM plaintext of `ciphertext` under `key` with `nonce` and `aad`, if it opens.
pub fn open(key: &[u8; 16], nonce: &[u8; 12], aad: &[u8], ciphertext: &[u8]) -> Option<Vec<u8>> {
    aead()
        .aead_open(key, ciphertext, Some(aad), nonce)
        .ok()
        .map(|plaintext| plaintext.to_vec())
}

/// The associated data of a link: the encoded `["skald history link", channel, e, prev]`.
pub fn link_aad(channel: ChannelId, epoch: u64, prev: u64) -> Vec<u8> {
    Value::Array(vec![
        text("skald history link"),
        bytes(channel.as_bytes()),
        Value::Uint(epoch),
        Value::Uint(prev),
    ])
    .encode()
}

/// The associated data of an entry: the encoded `["skald history entry", channel, epoch]`.
pub fn entry_aad(channel: ChannelId, epoch: u64) -> Vec<u8> {
    Value::Array(vec![
        text("skald history entry"),
        bytes(channel.as_bytes()),
        Value::Uint(epoch),
    ])
    .encode()
}

/// What a link and an entry both hold.
pub struct Sealed<'a> {
    pub channel: ChannelId,
    pub epoch: u64,
    pub nonce: &'a [u8],
    pub ciphertext: &'a [u8],
}

/// The fields a link and an entry begin with: `channel` and `epoch`.
fn head(sealed: &Sealed<'_>) -> Vec<(i64, Value)> {
    vec![
        (1, bytes(sealed.channel.as_bytes())),
        (2, Value::Uint(sealed.epoch)),
    ]
}

/// The encoded map of `fields` and `unknown` ones.
fn map(mut fields: Vec<(i64, Value)>, unknown: Vec<(i64, Value)>) -> Vec<u8> {
    fields.extend(unknown);
    Value::Map(fields).encode()
}

/// An encoded `history-link` of `sealed`, `prev` and `unknown` fields.
pub fn link_cbor(sealed: Sealed<'_>, prev: u64, unknown: Vec<(i64, Value)>) -> Vec<u8> {
    let mut fields = head(&sealed);
    fields.extend([
        (3, Value::Uint(prev)),
        (4, bytes(sealed.nonce)),
        (5, bytes(sealed.ciphertext)),
    ]);
    map(fields, unknown)
}

/// An encoded `history-entry` of `sealed` and `unknown` fields.
pub fn entry_cbor(sealed: Sealed<'_>, unknown: Vec<(i64, Value)>) -> Vec<u8> {
    let mut fields = head(&sealed);
    fields.extend([(3, bytes(sealed.nonce)), (4, bytes(sealed.ciphertext))]);
    map(fields, unknown)
}

/// An encoded `history-body` of these fields, `content` already encoded, and `unknown` ones.
pub fn body_cbor(
    sender: DeviceId,
    channel: ChannelId,
    msg_id: MessageId,
    content: &[u8],
    unknown: Vec<(i64, Value)>,
) -> Vec<u8> {
    let fields = vec![
        (1, bytes(sender.as_bytes())),
        (2, bytes(channel.as_bytes())),
        (3, bytes(msg_id.as_bytes())),
        (4, Value::Uint(SENT)),
        (5, bytes(content)),
    ];
    map(fields, unknown)
}

/// An encoded `history-plaintext`: `body` and its `signature`.
pub fn plaintext_cbor(body: &[u8], signature: &[u8]) -> Vec<u8> {
    Value::Array(vec![bytes(body), bytes(signature)]).encode()
}

/// `body` signed by device `seed` over `[label, body]`, as a `history-plaintext`.
pub fn signed(seed: u8, label: &str, body: &[u8]) -> Vec<u8> {
    plaintext_cbor(body, &signing_key(seed).sign(label, &[body]))
}

/// The fields of content with an A2A message, its text "Here is the diff".
pub fn content_fields() -> Vec<(i64, Value)> {
    a2a_fields("Here is the diff")
}

/// The encoded content of [`content_fields`].
pub fn content() -> Vec<u8> {
    Value::Map(content_fields()).encode()
}

/// The body of a message device `seed` sends in `channel`.
pub fn body(seed: u8, channel: ChannelId, msg_id: MessageId) -> HistoryBody {
    HistoryBody::from_cbor(&body_cbor(
        device_id(seed),
        channel,
        msg_id,
        &content(),
        vec![],
    ))
    .expect("a history body")
}

/// A link of `channel` from epoch `epoch` to `prev`, sealed apart from the crate under `key` with
/// the associated data of its own fields, holding `held`.
pub fn forged_link(
    key: &[u8; 16],
    channel: ChannelId,
    epoch: u64,
    prev: u64,
    held: &[u8],
) -> HistoryLink {
    let nonce = [0x6e; 12];
    let ciphertext = seal(key, &nonce, &link_aad(channel, epoch, prev), held);
    // The fields are set after decoding, so a link that breaks the CDDL bounds can be made too.
    let mut link = HistoryLink::from_cbor(&link_cbor(
        Sealed {
            channel,
            epoch: 3,
            nonce: &nonce,
            ciphertext: &ciphertext,
        },
        2,
        vec![],
    ))
    .expect("a history link");
    link.epoch = epoch;
    link.prev = prev;
    link
}

/// An entry of `channel` in `epoch`, sealed apart from the crate under `key` with the associated
/// data of its own fields, holding `plaintext`.
pub fn forged_entry(
    key: &[u8; 16],
    channel: ChannelId,
    epoch: u64,
    plaintext: &[u8],
) -> HistoryEntry {
    let nonce = [0x6f; 12];
    let ciphertext = seal(key, &nonce, &entry_aad(channel, epoch), plaintext);
    HistoryEntry::from_cbor(&entry_cbor(
        Sealed {
            channel,
            epoch,
            nonce: &nonce,
            ciphertext: &ciphertext,
        },
        vec![],
    ))
    .expect("a history entry")
}

/// What a reader gets from `entry` with `key`, as the caller does it: the body, once `entry` opens
/// and its signature verifies with the certificate of its `sender` among the tests' devices; the
/// first refusal otherwise.
pub fn read(key: &HistoryKey, entry: &HistoryEntry) -> Result<HistoryBody, String> {
    let message = key
        .open_entry(entry)
        .map_err(|error| format!("open: {error:?}"))?;
    let sender = message.sender();
    let cert = admitted()
        .iter()
        .find(|cert| cert.body().device == sender)
        .ok_or_else(|| format!("no certificate of {sender}"))?;
    message
        .verify(cert)
        .map_err(|error| format!("verify: {error:?}"))
}
