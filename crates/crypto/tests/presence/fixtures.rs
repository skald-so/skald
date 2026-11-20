//! Presence values apart from the crates under test: bodies encoded with the tests' CBOR writer from
//! their spec fields, signed by the tests' devices over `["skald presence", body]`, and sealed with
//! AES-128-GCM under the presence key HKDF-SHA256 derives from a history key, with the associated
//! data `["skald presence", device, channel, epoch]`.

use aes_gcm::{
    Aes128Gcm, KeyInit as _, Nonce,
    aead::{Aead as _, Payload},
};
use hkdf::Hkdf;
use sha2::Sha256;
use skald_crypto::{HistoryKey, PresenceError};
use skald_proto::{Cbor, ChannelId, DeviceId, Presence, PresenceBody, QueueId};

use crate::cbor::{Value, bytes, text};
use crate::devices::{NOW, device_id, signing_key};
use crate::fields::signed_cbor;

/// The label of a presence plaintext's signature.
pub const LABEL: &str = "skald presence";
/// A value's `expires`: 900 seconds after [`NOW`].
pub const EXPIRES: u64 = NOW + 900;
/// When the fixtures' agents entered their state.
pub const SINCE: u64 = NOW - 300;
/// The plaintext is padded to a multiple of this many bytes.
pub const BLOCK: usize = 4096;
/// The history key of the fixtures' epoch.
pub const HISTORY: [u8; 16] = [0x41; 16];
/// The history key of another epoch.
pub const OTHER_HISTORY: [u8; 16] = [0x42; 16];
/// The tests' devices, by seed.
pub const ALICE: u8 = 61;
pub const BOB: u8 = 62;

/// Agent states and queue tiers as the spec encodes them.
pub const AVAILABLE: u64 = 1;
pub const BUSY: u64 = 2;
pub const WAITING: u64 = 3;
pub const OFFLINE: u64 = 4;
pub const OFF: u64 = 1;
pub const READ_ONLY: u64 = 2;
pub const APPROVE: u64 = 3;
pub const FULL: u64 = 4;

/// An agent of Alice's account, and Alice herself.
pub const AGENT: &str = "acme/alice/claude-api@skald.example";
pub const OTHER_AGENT: &str = "acme/alice/codex@skald.example";
pub const PERSON: &str = "acme/alice/user@skald.example";

/// The presence key of the epoch whose history key is `history`, apart from the crate:
/// `HKDF-SHA256(salt = "", IKM = history, info = "skald presence", L = 16)`.
pub fn derived(history: &[u8; 16]) -> [u8; 16] {
    let mut key = [0; 16];
    Hkdf::<Sha256>::new(Some(b""), history)
        .expand(LABEL.as_bytes(), &mut key)
        .expect("16 bytes of HKDF output");
    key
}

/// The encoded `["skald presence", device, channel, epoch]`.
pub fn aad(device: DeviceId, channel: ChannelId, epoch: u64) -> Vec<u8> {
    Value::Array(vec![
        text(LABEL),
        bytes(device.as_bytes()),
        bytes(channel.as_bytes()),
        Value::Uint(epoch),
    ])
    .encode()
}

/// `plaintext` followed by the fewest zero bytes that make its length a multiple of [`BLOCK`].
pub fn padded(plaintext: &[u8]) -> Vec<u8> {
    let mut out = plaintext.to_vec();
    out.resize(plaintext.len().div_ceil(BLOCK) * BLOCK, 0);
    out
}

/// AES-128-GCM of `plaintext` under the AES key `key`.
pub fn encrypt(key: &[u8; 16], nonce: &[u8; 12], aad: &[u8], plaintext: &[u8]) -> Vec<u8> {
    Aes128Gcm::new(key.into())
        .encrypt(
            &Nonce::from(*nonce),
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .expect("AES-128-GCM seals")
}

/// The AES-128-GCM plaintext of `ct` under the AES key `key`, if it opens.
pub fn decrypt(key: &[u8; 16], nonce: &[u8; 12], aad: &[u8], ct: &[u8]) -> Option<Vec<u8>> {
    Aes128Gcm::new(key.into())
        .decrypt(&Nonce::from(*nonce), Payload { msg: ct, aad })
        .ok()
}

/// What `value` holds, opened apart from the crate with the presence key of `history` and the
/// associated data of the value's own fields.
pub fn decrypted(history: &[u8; 16], value: &Presence) -> Option<Vec<u8>> {
    decrypt(
        &derived(history),
        &value.nonce,
        &aad(value.device, value.channel, value.epoch),
        &value.ct,
    )
}

/// An `agent-status` of `agent` in `state` since [`SINCE`], without a note or a runtime.
pub fn status(agent: &str, state: u64) -> Vec<(i64, Value)> {
    vec![
        (1, text(agent)),
        (2, Value::Uint(state)),
        (3, Value::Uint(SINCE)),
    ]
}

/// An `agent-status` with a note and a runtime.
pub fn status_with(agent: &str, state: u64, note: &str, runtime: &str) -> Vec<(i64, Value)> {
    let mut fields = status(agent, state);
    fields.extend([(4, text(note)), (5, text(runtime))]);
    fields
}

/// A `served-queue` of `queue` with its tiers for `own`, `peer` and `service` senders.
pub fn queue(queue: QueueId, tiers: [u64; 3]) -> Value {
    Value::Array(vec![
        bytes(queue.as_bytes()),
        Value::Array(tiers.into_iter().map(Value::Uint).collect()),
    ])
}

/// The spec fields of a `presence-body`, changed field by field by the tests.
#[derive(Clone)]
pub struct Body {
    pub device: DeviceId,
    pub channel: ChannelId,
    pub epoch: u64,
    pub revision: u64,
    pub expires: u64,
    pub agents: Vec<Vec<(i64, Value)>>,
    pub serves: Vec<Value>,
    pub unknown: Vec<(i64, Value)>,
}

impl Body {
    /// Device `seed`'s body in epoch 1 of `channel`: an agent working on a PR, another offline,
    /// and a queue it takes `own` tasks from.
    pub fn of(seed: u8, channel: ChannelId) -> Self {
        Self {
            device: device_id(seed),
            channel,
            epoch: 1,
            revision: 7,
            expires: EXPIRES,
            agents: vec![
                status_with(AGENT, BUSY, "reviewing PR 12", "claude-code"),
                status(OTHER_AGENT, OFFLINE),
            ],
            serves: vec![queue(QueueId::generate(), [FULL, READ_ONLY, OFF])],
            unknown: Vec::new(),
        }
    }

    pub fn fields(&self) -> Vec<(i64, Value)> {
        let mut fields = vec![
            (1, bytes(self.device.as_bytes())),
            (2, bytes(self.channel.as_bytes())),
            (3, Value::Uint(self.epoch)),
            (4, Value::Uint(self.revision)),
            (5, Value::Uint(self.expires)),
            (
                6,
                Value::Array(self.agents.iter().cloned().map(Value::Map).collect()),
            ),
            (7, Value::Array(self.serves.clone())),
        ];
        fields.extend(self.unknown.iter().cloned());
        fields
    }

    /// The encoded body.
    pub fn cbor(&self) -> Vec<u8> {
        Value::Map(self.fields()).encode()
    }

    /// The body as `skald-proto` decodes it.
    pub fn decoded(&self) -> PresenceBody {
        PresenceBody::from_cbor(&self.cbor()).expect("a presence body")
    }

    /// The encoded `presence-plaintext`: the body signed by device `seed` over `[label, body]`.
    pub fn signed(&self, seed: u8, label: &str) -> Vec<u8> {
        let body = self.cbor();
        signed_cbor(&body, &signing_key(seed).sign(label, &[&body]))
    }
}

/// An encoded `presence` value of these fields and `unknown` ones.
pub fn value_cbor(
    device: DeviceId,
    channel: ChannelId,
    epoch: u64,
    nonce: &[u8],
    ct: &[u8],
    unknown: Vec<(i64, Value)>,
) -> Vec<u8> {
    let mut fields = vec![
        (1, bytes(device.as_bytes())),
        (2, bytes(channel.as_bytes())),
        (3, Value::Uint(epoch)),
        (4, bytes(nonce)),
        (5, bytes(ct)),
    ];
    fields.extend(unknown);
    Value::Map(fields).encode()
}

/// A value of `device` in `channel` and `epoch`, sealed apart from the crate under `key` as the AES
/// key, with associated data `aad`, holding `plaintext` as it is.
pub fn sealed_with(
    key: &[u8; 16],
    aad: &[u8],
    (device, channel, epoch): (DeviceId, ChannelId, u64),
    plaintext: &[u8],
) -> Presence {
    let nonce = [0x6e; 12];
    let ct = encrypt(key, &nonce, aad, plaintext);
    Presence::from_cbor(&value_cbor(device, channel, epoch, &nonce, &ct, vec![]))
        .expect("a presence value")
}

/// A value of `device` in `channel` and `epoch` as the spec seals it, apart from the crate, under
/// the presence key of `history`, holding `plaintext` padded.
pub fn forged(
    history: &[u8; 16],
    (device, channel, epoch): (DeviceId, ChannelId, u64),
    plaintext: &[u8],
) -> Presence {
    sealed_with(
        &derived(history),
        &aad(device, channel, epoch),
        (device, channel, epoch),
        &padded(plaintext),
    )
}

/// `body` signed by its device and sealed as the spec says under the presence key of [`HISTORY`],
/// with the body's own device, channel and epoch.
pub fn forged_body(body: &Body, seed: u8) -> Presence {
    forged(
        &HISTORY,
        (body.device, body.channel, body.epoch),
        &body.signed(seed, LABEL),
    )
}

/// `value` opened by `skald-crypto` with the presence key of `history`, device `seed`'s signing
/// key and the time `now`.
pub fn open_at(
    history: &[u8; 16],
    value: &Presence,
    seed: u8,
    now: u64,
) -> Result<PresenceBody, PresenceError> {
    HistoryKey::from_bytes(history)
        .presence_key()
        .open(value, &signing_key(seed).public_key(), now)
}

/// `value` opened by `skald-crypto` with the presence key of [`HISTORY`], device `seed`'s signing
/// key and the time [`NOW`].
pub fn open(value: &Presence, seed: u8) -> Result<PresenceBody, PresenceError> {
    open_at(&HISTORY, value, seed, NOW)
}

/// `body` sealed by `skald-crypto` under the presence key of `history`, signed by device `seed`.
pub fn seal(history: &[u8; 16], body: &PresenceBody, seed: u8) -> Presence {
    HistoryKey::from_bytes(history)
        .presence_key()
        .seal(body, &signing_key(seed))
        .expect("a presence value")
}

/// Alice's body in a new channel and the value `skald-crypto` seals of it, which opens to it.
pub fn crate_sealed() -> (Body, Presence) {
    let body = Body::of(ALICE, ChannelId::generate());
    let value = seal(&HISTORY, &body.decoded(), ALICE);
    assert_eq!(
        open(&value, ALICE),
        Ok(body.decoded()),
        "the value as sealed opens"
    );
    (body, value)
}
