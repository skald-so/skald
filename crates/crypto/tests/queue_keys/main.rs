//! Queue keys and sealed tasks, natively and in WASM: `docs/protocol/queues.md`, "Queue key" and
//! "Claim checks". An epoch's queue key pair is `DeriveKeyPair(ikm)` of DHKEM(X25519,
//! HKDF-SHA256), `ikm = HKDF-SHA256(salt = "", IKM = the epoch's history key, info = "skald queue",
//! L = 32)`; a task plaintext, its body signed by its sender over `["skald task", body]`, is sealed
//! to an epoch's `pk` with single-shot HPKE in base mode, `info` `skald task payload` and `aad` the
//! encoded `[channel, queue, task, epoch]`; a queue key object is signed by its `signer` over
//! `["skald queue key", body]`. A claiming device's checks 1, 3 and, of 2, the signature by a given
//! key refuse a task the spec says it must not run. The vectors of
//! `docs/protocol/test-vectors.md`, "Queue key pair", "Sealed task" and "Queue key object", are
//! checked apart from the crate and against it.
//!
//! The API, as these tests use it, each error type `Debug`:
//! - `skald_crypto::HistoryKey::queue_key(&self) -> QueueKeyPair`: the queue key pair of the key's
//!   epoch;
//! - `skald_crypto::QueueKeyPair`: `public_key(&self) -> [u8; 32]`, the `pk`;
//!   `open_task(&self, &QueuedTask, channel: ChannelId, queue: QueueId) -> Result<TaskPlaintext, _>`,
//!   check 1: `ct` opened with the `sk` and the `aad` of `channel`, `queue` and the record's `task`
//!   and `epoch`; `check_claim(&self, &Claim, channel: ChannelId, queue: QueueId, signing:
//!   &[u8; 32]) -> Result<TaskBody, _>`, checks 1, 3 and, of 2, the signature: the claimed task
//!   opens as `open_task` opens it with the claimed `queue`, its body's `sender` derives from
//!   `signing` and its signature verifies with it, and the body's `channel`, `queue`, `task`,
//!   `sched` and `mode` are `channel`, `queue` and the claim's;
//! - `skald_crypto::sign_task(&TaskBody, &SigningKey) -> Result<TaskPlaintext, _>` and
//!   `check_task(&TaskPlaintext, signing: &[u8; 32]) -> Result<(), _>`: the body's `sender` derives
//!   from `signing` and the signature verifies with it;
//! - `skald_crypto::seal_task(&TaskPlaintext, epoch: u64, pk: &[u8; 32]) -> Result<QueuedTask, _>`:
//!   the record of the body's `task`, `queue` and `sched` under `epoch`; refuses epoch 0 and a `pk`
//!   of small order; `seal_task_with_ephemeral(.., ephemeral: &[u8; 32])`, whose ephemeral key pair
//!   is `DeriveKeyPair(ephemeral)`, for the vector;
//! - `skald_crypto::sign_queue_key(&QueueKeyBody, &SigningKey) -> Result<QueueKey, _>` and
//!   `check_queue_key(&QueueKey, signing: &[u8; 32]) -> Result<(), _>`: the body's `signer` derives
//!   from `signing` and the signature verifies with it.
//!
//! The fixtures derive keys with HKDF-SHA256 and the `hpke` crate, seal and open with the `hpke`
//! crate and encode and sign items with the tests' CBOR writer and devices, apart from the code
//! under test.

#[path = "../rosters/cbor.rs"]
mod cbor;
mod claims;
#[allow(dead_code)]
#[path = "../common/mod.rs"]
mod common;
#[allow(dead_code)]
#[path = "../common/devices.rs"]
mod devices;
#[allow(dead_code)]
#[path = "../common/fields.rs"]
mod fields;
mod fixtures;
#[allow(dead_code)]
#[path = "../queues/fixtures.rs"]
mod items;
mod keys;
mod objects;
mod sealing;
mod signing;
mod vectors;
