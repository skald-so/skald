//! Queue keys and sealed tasks apart from the code under test: the key pair HKDF-SHA256 and the
//! `hpke` crate derive from a history key, single-shot HPKE in base mode with the spec's `info` and
//! `aad`, and task bodies encoded with the tests' CBOR writer and signed by the tests' devices.

use hkdf::Hkdf;
use hpke::{
    Deserializable as _, Kem as _, OpModeR, OpModeS, Serializable as _, aead::AesGcm128,
    kdf::HkdfSha256, kem::X25519HkdfSha256,
};
use sha2::Sha256;
use skald_proto::{
    Cbor, ChannelId, Claim, ClaimId, QueueId, QueueMode, QueuedTask, Sched, TaskId, Unknown,
};

use crate::cbor::{Value, bytes};
use crate::devices::device_id;
use crate::items::{
    AT_LEAST_ONCE, EPOCH, GENERATION, Ids, LEASE, SENDER, TASK_LABEL, encoded, sched_fields,
    signed, task_body_fields,
};

/// `info` of a queue key's HKDF and of a task's seal.
pub const QUEUE_INFO: &[u8] = b"skald queue";
pub const PAYLOAD_INFO: &[u8] = b"skald task payload";

/// The history keys of the fixtures' epoch and of another epoch.
pub const HISTORY: [u8; 16] = [0x51; 16];
pub const OTHER_HISTORY: [u8; 16] = [0x52; 16];

/// The queue key pair of the epoch whose history key is `history`: `(sk, pk)`, serialized as RFC
/// 9180 serializes them.
pub fn pair(history: &[u8; 16]) -> ([u8; 32], [u8; 32]) {
    let mut ikm = [0; 32];
    Hkdf::<Sha256>::new(Some(b""), history)
        .expand(QUEUE_INFO, &mut ikm)
        .expect("32 bytes of HKDF output");
    let (sk, pk) = X25519HkdfSha256::derive_keypair(&ikm);
    (sk.to_bytes().into(), pk.to_bytes().into())
}

/// The encoded `[channel, queue, task, epoch]`.
pub fn aad(channel: ChannelId, queue: QueueId, task: TaskId, epoch: u64) -> Vec<u8> {
    Value::Array(vec![
        bytes(channel.as_bytes()),
        bytes(queue.as_bytes()),
        bytes(task.as_bytes()),
        Value::Uint(epoch),
    ])
    .encode()
}

/// `plaintext` sealed to `pk` with `info` `skald task payload` and `aad`: `enc` and `ct`.
pub fn seal(pk: &[u8; 32], aad: &[u8], plaintext: &[u8]) -> ([u8; 32], Vec<u8>) {
    let recipient =
        <X25519HkdfSha256 as hpke::Kem>::PublicKey::from_bytes(pk).expect("an X25519 key");
    let (enc, ct) = hpke::single_shot_seal::<AesGcm128, HkdfSha256, X25519HkdfSha256>(
        &OpModeS::Base,
        &recipient,
        PAYLOAD_INFO,
        plaintext,
        aad,
    )
    .expect("a seal to a derived key");
    (enc.to_bytes().into(), ct)
}

/// What `enc` and `ct` open to with `sk`, `info` `skald task payload` and `aad`, if they open.
pub fn open(sk: &[u8; 32], enc: &[u8; 32], aad: &[u8], ct: &[u8]) -> Option<Vec<u8>> {
    let key = <X25519HkdfSha256 as hpke::Kem>::PrivateKey::from_bytes(sk).ok()?;
    let enc = <X25519HkdfSha256 as hpke::Kem>::EncappedKey::from_bytes(enc).ok()?;
    hpke::single_shot_open::<AesGcm128, HkdfSha256, X25519HkdfSha256>(
        &OpModeR::Base,
        &key,
        &enc,
        PAYLOAD_INFO,
        ct,
        aad,
    )
    .ok()
}

/// A task of the fixtures' epoch: its IDs, its encoded body and plaintext, and the record that
/// seals the plaintext to the epoch's `pk`.
pub struct Sealed {
    pub ids: Ids,
    pub body: Vec<u8>,
    pub plaintext: Vec<u8>,
    pub queued: QueuedTask,
}

/// A record of `ids` with `sched`, under `epoch`, that holds `enc` and `ct`.
pub fn record(
    ids: &Ids,
    sched: &[(i64, Value)],
    epoch: u64,
    enc: [u8; 32],
    ct: Vec<u8>,
) -> QueuedTask {
    QueuedTask {
        task: ids.task,
        queue: ids.queue,
        sched: Sched::from_cbor(&encoded(sched)).expect("the fixtures' sched decodes"),
        epoch,
        enc,
        ct,
        unknown: Unknown::default(),
    }
}

/// `plaintext` sealed as a task of `ids` with `sched` in [`EPOCH`] to the queue key of
/// [`HISTORY`], with the `aad` of `ids`.
pub fn sealed_plaintext(
    ids: Ids,
    sched: &[(i64, Value)],
    body: Vec<u8>,
    plaintext: Vec<u8>,
) -> Sealed {
    let (_, pk) = pair(&HISTORY);
    let (enc, ct) = seal(
        &pk,
        &aad(ids.channel, ids.queue, ids.task, EPOCH),
        &plaintext,
    );
    let queued = record(&ids, sched, EPOCH, enc, ct);
    Sealed {
        ids,
        body,
        plaintext,
        queued,
    }
}

/// A task of `ids` whose body has `body_fields`, signed by device `seed` under `label` and sealed
/// as [`sealed_plaintext`] seals, its record with `sched`.
pub fn sealed_body(
    ids: Ids,
    sched: &[(i64, Value)],
    body_fields: &[(i64, Value)],
    seed: u8,
    label: &str,
) -> Sealed {
    let body = encoded(body_fields);
    let plaintext = signed(&body, seed, label);
    sealed_plaintext(ids, sched, body, plaintext)
}

/// The fields of the fixtures' task body: from [`SENDER`], with every scheduling field, in an
/// `at-least-once` queue.
pub fn body_fields(ids: &Ids) -> Vec<(i64, Value)> {
    task_body_fields(ids, sched_fields(), AT_LEAST_ONCE)
}

/// The fixtures' task: [`body_fields`] signed by [`SENDER`] under `skald task`, its record with
/// the same `sched`.
pub fn sealed() -> Sealed {
    let ids = Ids::new();
    let fields = body_fields(&ids);
    sealed_body(ids, &sched_fields(), &fields, SENDER, TASK_LABEL)
}

/// `fields` with `value` at `key` in place of the field there.
pub fn replaced(fields: &[(i64, Value)], key: i64, value: Value) -> Vec<(i64, Value)> {
    let mut out = fields.to_vec();
    let at = out
        .iter()
        .position(|(k, _)| *k == key)
        .expect("a field to replace");
    out[at].1 = value;
    out
}

/// The claim of `queued` in a queue of `mode`, uploaded by [`SENDER`].
pub fn claim(queued: &QueuedTask, mode: QueueMode) -> Claim {
    Claim {
        claim_id: ClaimId::generate(),
        generation: GENERATION,
        attempt: 1,
        lease: LEASE,
        mode,
        uploader: device_id(SENDER),
        task: queued.clone(),
        unknown: Unknown::default(),
    }
}

/// The verdict a check gives `result`: `Ok(())` or the error's `Debug`, so a refusal reads in a
/// failed assertion.
pub fn verdict<T, E: std::fmt::Debug>(result: Result<T, E>) -> Result<(), String> {
    result.map(|_| ()).map_err(|error| format!("{error:?}"))
}

/// Asserts that `result` is a refusal.
pub fn assert_refused<T, E: std::fmt::Debug>(result: Result<T, E>, what: &str) {
    assert!(verdict(result).is_err(), "{what} is refused");
}
