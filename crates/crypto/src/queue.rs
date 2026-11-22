//! Queue keys and sealed tasks: `docs/protocol/queues.md`, "Queue key", "Queue key object" and
//! "Claim checks". An epoch's queue key pair derives from its history key; a task plaintext is
//! sealed to its `pk` with single-shot HPKE in base mode.

use std::fmt;

use hkdf::Hkdf;
use sha2::Sha256;
use skald_proto::{
    Cbor, ChannelId, Claim, DecodeError, QUEUE_KEY_LABEL, QueueId, QueueKey, QueueKeyBody,
    QueuedTask, TASK_LABEL, TaskBody, TaskPlaintext,
};
use zeroize::Zeroizing;

use crate::history::HistoryKey;
use crate::hpke::{HpkeError, HpkeKey};
use crate::sign::{SignatureError, SigningKey, verify};

/// `info` of the HKDF that derives a queue key pair's `ikm`.
const QUEUE_INFO: &[u8] = b"skald queue";
/// `info` of a task's seal.
const PAYLOAD_INFO: &[u8] = b"skald task payload";

/// An epoch's queue key pair, an HPKE key pair of DHKEM(X25519, HKDF-SHA256). Its private key is
/// zeroed on drop and never shown.
#[derive(Debug)]
pub struct QueueKeyPair(HpkeKey);

impl HistoryKey {
    /// The queue key pair of this key's epoch: `DeriveKeyPair(ikm)` (RFC 9180, §7.1.3), `ikm =
    /// HKDF-SHA256(salt = "", IKM = this key, info = "skald queue", L = 32)`.
    pub fn queue_key(&self) -> QueueKeyPair {
        let mut ikm = Zeroizing::new([0; 32]);
        Hkdf::<Sha256>::new(Some(b""), self.to_bytes().as_ref())
            .expand(QUEUE_INFO, ikm.as_mut())
            .expect("32 bytes of HKDF-SHA256");
        QueueKeyPair(HpkeKey::from_ikm(&ikm))
    }
}

impl QueueKeyPair {
    /// The `pk`.
    pub fn public_key(&self) -> [u8; 32] {
        self.0.public_key()
    }

    /// The task plaintext `queued` seals, opened with this key pair, which is `queued.epoch`'s,
    /// and the `aad` of `channel`, `queue` and the record's `task` and `epoch`: claim check 1.
    ///
    /// # Errors
    /// [`QueueError::Open`] if it does not open; [`QueueError::Malformed`] if it opens to no task
    /// plaintext.
    pub fn open_task(
        &self,
        queued: &QueuedTask,
        channel: ChannelId,
        queue: QueueId,
    ) -> Result<TaskPlaintext, QueueError> {
        let aad = QueuedTask::associated_data(channel, queue, queued.task, queued.epoch);
        let opened = self
            .0
            .open(&queued.enc, PAYLOAD_INFO, &aad, &queued.ct)
            .map_err(|_| QueueError::Open)?;
        TaskPlaintext::from_cbor(&opened).map_err(QueueError::Malformed)
    }

    /// The body of the task of `claim`, claimed from `channel`'s queue `queue`, once it passes the
    /// claim checks that need nothing of the device's state: 1, as [`QueueKeyPair::open_task`]
    /// makes it; of 2, the signature, as [`check_task`] makes it with `signing`, the signing key
    /// of the body's `sender`; and 3. The caller makes the rest.
    ///
    /// # Errors
    /// Those of [`QueueKeyPair::open_task`]; [`QueueError::Signature`] if check 2 fails;
    /// [`QueueError::Mismatch`] if check 3 fails.
    pub fn check_claim(
        &self,
        claim: &Claim,
        channel: ChannelId,
        queue: QueueId,
        signing: &[u8; 32],
    ) -> Result<TaskBody, QueueError> {
        let queued = &claim.task;
        let plaintext = self.open_task(queued, channel, queue)?;
        check_task(&plaintext, signing).map_err(|_| QueueError::Signature)?;
        let body = plaintext.body();
        if body.channel != channel
            || body.queue != queue
            || body.task != queued.task
            || body.sched != queued.sched
            || body.mode != claim.mode
        {
            return Err(QueueError::Mismatch);
        }
        Ok(body.clone())
    }
}

/// The task plaintext of `body`, signed by `sender`, the signing key of `body.sender`, over
/// `["skald task", body]`.
///
/// # Errors
/// Why `body` breaks the spec's CDDL.
pub fn sign_task(body: &TaskBody, sender: &SigningKey) -> Result<TaskPlaintext, DecodeError> {
    let bytes = body.to_cbor();
    let signature = sender.sign(TASK_LABEL, &[&bytes]);
    TaskPlaintext::from_parts(bytes, signature)
}

/// Checks that `signing` is the signing key of the body's `sender` and that its signature over
/// `["skald task", body]` verifies with it.
///
/// # Errors
/// [`SignatureError`] if `sender` does not derive from `signing` or the signature does not verify.
pub fn check_task(plaintext: &TaskPlaintext, signing: &[u8; 32]) -> Result<(), SignatureError> {
    if !plaintext.body().sender.is_derived_from(signing) {
        return Err(SignatureError);
    }
    verify(
        signing,
        TASK_LABEL,
        &[plaintext.body_bytes()],
        plaintext.signature(),
    )
}

/// The record of `plaintext` under `epoch`: its body's `task`, `queue` and `sched`, and the
/// plaintext sealed to `pk`, the queue key of `epoch`, with the `aad` of its body's `channel`,
/// `queue` and `task` and `epoch`. Each seal takes a new ephemeral key.
///
/// # Errors
/// [`QueueError::Bounds`] for epoch 0; [`QueueError::RecipientKey`] if `pk` is of small order.
///
/// # Panics
/// If the random generator fails.
pub fn seal_task(
    plaintext: &TaskPlaintext,
    epoch: u64,
    pk: &[u8; 32],
) -> Result<QueuedTask, QueueError> {
    seal(plaintext, epoch, pk, None)
}

/// [`seal_task`] with the ephemeral key pair `DeriveKeyPair(ephemeral)`, for a test vector.
///
/// # Errors
/// As [`seal_task`].
pub fn seal_task_with_ephemeral(
    plaintext: &TaskPlaintext,
    epoch: u64,
    pk: &[u8; 32],
    ephemeral: &[u8; 32],
) -> Result<QueuedTask, QueueError> {
    seal(plaintext, epoch, pk, Some(ephemeral))
}

fn seal(
    plaintext: &TaskPlaintext,
    epoch: u64,
    pk: &[u8; 32],
    ephemeral: Option<&[u8; 32]>,
) -> Result<QueuedTask, QueueError> {
    if epoch == 0 {
        return Err(QueueError::Bounds);
    }
    let body = plaintext.body();
    let aad = QueuedTask::associated_data(body.channel, body.queue, body.task, epoch);
    let encoded = Zeroizing::new(plaintext.to_cbor());
    let (enc, ct) = HpkeKey::seal_with(pk, PAYLOAD_INFO, &aad, &encoded, ephemeral)
        .map_err(|_: HpkeError| QueueError::RecipientKey)?;
    Ok(QueuedTask {
        task: body.task,
        queue: body.queue,
        sched: body.sched.clone(),
        epoch,
        enc,
        ct,
        unknown: Default::default(),
    })
}

/// The queue key object of `body`, signed by `signer`, the signing key of `body.signer`, over
/// `["skald queue key", body]`.
///
/// # Errors
/// Why `body` breaks the spec's CDDL.
pub fn sign_queue_key(body: &QueueKeyBody, signer: &SigningKey) -> Result<QueueKey, DecodeError> {
    let bytes = body.to_cbor();
    let signature = signer.sign(QUEUE_KEY_LABEL, &[&bytes]);
    QueueKey::from_parts(bytes, signature)
}

/// Checks that `signing` is the signing key of the object's `signer` and that its signature over
/// `["skald queue key", body]` verifies with it.
///
/// # Errors
/// [`SignatureError`] if `signer` does not derive from `signing` or the signature does not verify.
pub fn check_queue_key(object: &QueueKey, signing: &[u8; 32]) -> Result<(), SignatureError> {
    if !object.body().signer.is_derived_from(signing) {
        return Err(SignatureError);
    }
    verify(
        signing,
        QUEUE_KEY_LABEL,
        &[object.body_bytes()],
        object.signature(),
    )
}

/// Why a task is not sealed or does not pass a claim check.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum QueueError {
    /// A seal under epoch 0.
    Bounds,
    /// The queue key `pk` is of small order, so the shared secret is all zeros.
    RecipientKey,
    /// Check 1: `ct` does not open with the key pair and the `aad`.
    Open,
    /// Check 1: `ct` opens to no task plaintext.
    Malformed(DecodeError),
    /// Check 2: `sender` is not the signing key's device, or the signature does not verify.
    Signature,
    /// Check 3: the body's `channel`, `queue`, `task`, `sched` or `mode` is not the claim's.
    Mismatch,
}

impl fmt::Display for QueueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bounds => f.write_str("a task is not sealed under epoch 0"),
            Self::RecipientKey => f.write_str("the queue key is of small order"),
            Self::Open => f.write_str("the task does not open with the queue key"),
            Self::Malformed(error) => write!(f, "a malformed task plaintext: {error}"),
            Self::Signature => {
                f.write_str("the task's signature does not verify with its sender's key")
            }
            Self::Mismatch => {
                f.write_str("the task's channel, queue, task, sched or mode is not the claim's")
            }
        }
    }
}

impl std::error::Error for QueueError {}

#[cfg(test)]
mod tests {
    use skald_proto::{Body, Content, DeviceId, QueueMode, Sched, TaskId};

    use super::*;

    const EPOCH: u64 = 2;

    fn sender() -> SigningKey {
        SigningKey::from_bytes(&[71; 32])
    }

    fn key() -> QueueKeyPair {
        HistoryKey::from_bytes(&[0x61; 16]).queue_key()
    }

    /// A task of [`sender`], signed by it and sealed to [`key`] in [`EPOCH`], and its channel.
    fn sealed() -> (QueuedTask, ChannelId) {
        let channel = ChannelId::generate();
        let body = TaskBody {
            sender: DeviceId::from_signing_key(&sender().public_key()),
            channel,
            queue: QueueId::generate(),
            task: TaskId::generate(),
            sched: Sched::default(),
            mode: QueueMode::AtLeastOnce,
            enqueued: 1_800_000_000,
            content: Content {
                from: "acme/alice/claude@skald.example".parse().unwrap(),
                to: None,
                counter: std::num::NonZeroU64::new(1),
                hops: 0,
                reply_to: None,
                body: Some(Body::A2a("{}".to_owned())),
                delegation: None,
                grant: None,
                attachments: None,
                unknown: Default::default(),
            },
            unknown: Default::default(),
        };
        let plaintext = sign_task(&body, &sender()).unwrap();
        (
            seal_task(&plaintext, EPOCH, &key().public_key()).unwrap(),
            channel,
        )
    }

    fn claim(task: QueuedTask, mode: QueueMode) -> Claim {
        Claim {
            claim_id: skald_proto::ClaimId::generate(),
            generation: 1,
            attempt: 1,
            lease: 300,
            mode,
            uploader: DeviceId::from_signing_key(&sender().public_key()),
            task,
            unknown: Default::default(),
        }
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn each_failure_has_its_reason() {
        let (task, channel) = sealed();
        let queue = task.queue;
        let plaintext = key().open_task(&task, channel, queue).unwrap();
        let public = sender().public_key();

        assert_eq!(
            seal_task(&plaintext, 0, &key().public_key()).unwrap_err(),
            QueueError::Bounds
        );
        assert_eq!(
            seal_task(&plaintext, EPOCH, &[0; 32]).unwrap_err(),
            QueueError::RecipientKey
        );
        let other = HistoryKey::from_bytes(&[0x62; 16]).queue_key();
        assert_eq!(
            other.open_task(&task, channel, queue).unwrap_err(),
            QueueError::Open
        );
        let (enc, ct) = HpkeKey::seal(
            &key().public_key(),
            PAYLOAD_INFO,
            &QueuedTask::associated_data(channel, queue, task.task, EPOCH),
            b"",
        )
        .unwrap();
        let empty = QueuedTask {
            enc,
            ct,
            ..task.clone()
        };
        assert!(matches!(
            key().open_task(&empty, channel, queue),
            Err(QueueError::Malformed(_))
        ));
        let at_least_once = claim(task.clone(), QueueMode::AtLeastOnce);
        assert_eq!(
            key()
                .check_claim(
                    &at_least_once,
                    channel,
                    queue,
                    &SigningKey::from_bytes(&[72; 32]).public_key()
                )
                .unwrap_err(),
            QueueError::Signature
        );
        assert_eq!(
            key()
                .check_claim(&claim(task, QueueMode::AtMostOnce), channel, queue, &public)
                .unwrap_err(),
            QueueError::Mismatch
        );
        assert!(
            key()
                .check_claim(&at_least_once, channel, queue, &public)
                .is_ok()
        );
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn errors_explain_themselves() {
        let error = TaskPlaintext::from_cbor(b"").unwrap_err();
        for (error, text) in [
            (
                QueueError::Bounds,
                "a task is not sealed under epoch 0".to_owned(),
            ),
            (
                QueueError::RecipientKey,
                "the queue key is of small order".to_owned(),
            ),
            (
                QueueError::Open,
                "the task does not open with the queue key".to_owned(),
            ),
            (
                QueueError::Malformed(error.clone()),
                format!("a malformed task plaintext: {error}"),
            ),
            (
                QueueError::Signature,
                "the task's signature does not verify with its sender's key".to_owned(),
            ),
            (
                QueueError::Mismatch,
                "the task's channel, queue, task, sched or mode is not the claim's".to_owned(),
            ),
        ] {
            assert_eq!(error.to_string(), text);
        }
    }
}
