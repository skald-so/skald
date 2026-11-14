//! Delivery frames: `docs/protocol/delivery.md`, "Frames".

use std::num::NonZeroU64;

use crate::cbor::{
    self, Cbor, Codec, DecodeError, FieldProblem, Item, decode_embedded, field_error,
};
use crate::envelope::Envelope;
use crate::id::SpaceId;
use crate::receipt::{DeliveryStatus, STORED, SendKey, Stored};

/// A frame between a client and the server. A frame of a type this version does not know decodes
/// as [`Frame::Unknown`], so its reader can ignore it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    /// `send`, client → server: an envelope to store.
    Send {
        envelope: RawEnvelope,
        /// Whether the server issues `delivered` and `expired` for it.
        report: bool,
    },
    /// `stored`, server → client: the answer to a stored `send`.
    Stored(Stored),
    /// `send-refused`, server → client: the answer to a refused `send`.
    SendRefused {
        send_key: SendKey,
        /// The problem, possibly one this version does not know.
        problem: String,
    },
    /// `sync`, client → server: the device's cursor, stated on connecting.
    Sync { cursor: u64 },
    /// `message`, `status` or `space-notice`, server → client.
    Push(Push),
    /// `ack`, client → server: the device's cursor after writing entries.
    Ack { seq: NonZeroU64 },
    /// `batch`, server → client: the answer to a long poll.
    Batch(Vec<Push>),
    /// A frame of a type this version does not know; holds its type. Encodes as an array of the
    /// type alone.
    Unknown(u64),
}

/// A frame the server pushes, alone or in a `batch`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Push {
    /// `message`: a mailbox entry that holds an envelope for the device.
    Message {
        seq: NonZeroU64,
        envelope: RawEnvelope,
    },
    /// `status`: a mailbox entry that holds a server status for the device as a sender.
    Status {
        seq: NonZeroU64,
        status: DeliveryStatus,
    },
    /// `space-notice`: the space has a new roster revision or invite acceptance.
    SpaceNotice { space: SpaceId },
    /// A `batch` entry of a type this version does not know, which its reader ignores.
    Unknown(UnknownPush),
}

/// A `batch` entry of a type this version does not know, with the bytes it came in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownPush {
    frame_type: u64,
    bytes: Vec<u8>,
}

impl UnknownPush {
    /// Its frame type.
    pub fn frame_type(&self) -> u64 {
        self.frame_type
    }

    /// The bytes it came in.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

/// An envelope with the bytes it came in, which the server stores and forwards as they are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawEnvelope {
    bytes: Vec<u8>,
    envelope: Envelope,
}

impl RawEnvelope {
    /// The envelope `bytes` hold.
    ///
    /// # Errors
    /// Why `bytes` are not one envelope.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, DecodeError> {
        Ok(Self {
            envelope: Envelope::from_cbor(&bytes)?,
            bytes,
        })
    }

    /// The bytes it came in.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// The envelope, decoded.
    pub fn envelope(&self) -> &Envelope {
        &self.envelope
    }

    /// The envelope in the byte string of `field`, for CDDL `bstr .cbor envelope`.
    fn decode(item: &Item<'_>, field: &'static str) -> Result<Self, DecodeError> {
        let bytes = item.bytes(field)?;
        Ok(Self {
            envelope: decode_embedded(bytes, field)?,
            bytes: bytes.to_vec(),
        })
    }
}

// The frame types; `stored`'s is [`STORED`].
const SEND: u64 = 1;
const SEND_REFUSED: u64 = 3;
const SYNC: u64 = 4;
const MESSAGE: u64 = 5;
const STATUS: u64 = 6;
const ACK: u64 = 7;
const BATCH: u64 = 8;
const SPACE_NOTICE: u64 = 9;

/// The type of the frame `item`: the first item of an array.
fn frame_type(item: &Item<'_>, field: &'static str) -> Result<u64, DecodeError> {
    item.array(field)?
        .first()
        .ok_or(field_error(field, FieldProblem::Missing))?
        .uint(field)
}

impl Codec for Frame {
    fn encode(&self) -> Vec<u8> {
        match self {
            Self::Send { envelope, report } => cbor::array([
                cbor::uint(SEND),
                cbor::bytes(&envelope.bytes),
                cbor::boolean(*report),
            ]),
            Self::Stored(stored) => stored.encode(),
            Self::SendRefused { send_key, problem } => cbor::array([
                cbor::uint(SEND_REFUSED),
                send_key.encode(),
                cbor::text(problem),
            ]),
            Self::Sync { cursor } => cbor::array([cbor::uint(SYNC), cbor::uint(*cursor)]),
            Self::Push(push) => push.encode(),
            Self::Ack { seq } => cbor::array([cbor::uint(ACK), cbor::uint(seq.get())]),
            Self::Batch(pushes) => cbor::array([
                cbor::uint(BATCH),
                cbor::array(pushes.iter().map(Push::encode)),
            ]),
            Self::Unknown(frame_type) => cbor::array([cbor::uint(*frame_type)]),
        }
    }

    fn decode(item: &Item<'_>) -> Result<Self, DecodeError> {
        Ok(match frame_type(item, "frame.type")? {
            SEND => {
                let [_, envelope, report] = item.tuple("send")?;
                Self::Send {
                    envelope: RawEnvelope::decode(envelope, "send.envelope")?,
                    report: report.boolean("send.report")?,
                }
            }
            STORED => Self::Stored(Stored::decode(item)?),
            SEND_REFUSED => {
                let [_, send_key, problem] = item.tuple("send-refused")?;
                Self::SendRefused {
                    send_key: SendKey::decode(send_key)?,
                    problem: problem.text("send-refused.problem")?.to_owned(),
                }
            }
            SYNC => {
                let [_, cursor] = item.tuple("sync")?;
                Self::Sync {
                    cursor: cursor.uint("sync.cursor")?,
                }
            }
            MESSAGE | STATUS | SPACE_NOTICE => Self::Push(Push::decode(item)?),
            ACK => {
                let [_, seq] = item.tuple("ack")?;
                Self::Ack {
                    seq: seq.positive("ack.seq")?,
                }
            }
            BATCH => {
                let [_, pushes] = item.tuple("batch")?;
                Self::Batch(
                    pushes
                        .array("batch.frames")?
                        .iter()
                        .map(Push::decode)
                        .collect::<Result<_, _>>()?,
                )
            }
            other => Self::Unknown(other),
        })
    }
}

impl Codec for Push {
    fn encode(&self) -> Vec<u8> {
        match self {
            Self::Message { seq, envelope } => cbor::array([
                cbor::uint(MESSAGE),
                cbor::uint(seq.get()),
                cbor::bytes(&envelope.bytes),
            ]),
            Self::Status { seq, status } => {
                cbor::array([cbor::uint(STATUS), cbor::uint(seq.get()), status.encode()])
            }
            Self::SpaceNotice { space } => cbor::array([cbor::uint(SPACE_NOTICE), cbor::id(space)]),
            Self::Unknown(unknown) => unknown.bytes.clone(),
        }
    }

    fn decode(item: &Item<'_>) -> Result<Self, DecodeError> {
        match frame_type(item, "batch.frame.type")? {
            MESSAGE => {
                let [_, seq, envelope] = item.tuple("message")?;
                Ok(Self::Message {
                    seq: seq.positive("message.seq")?,
                    envelope: RawEnvelope::decode(envelope, "message.envelope")?,
                })
            }
            STATUS => {
                let [_, seq, status] = item.tuple("status")?;
                Ok(Self::Status {
                    seq: seq.positive("status.seq")?,
                    status: DeliveryStatus::decode(status)?,
                })
            }
            SPACE_NOTICE => {
                let [_, space] = item.tuple("space-notice")?;
                Ok(Self::SpaceNotice {
                    space: space.id("space-notice.space")?,
                })
            }
            // A `batch` holds no frame of another known type.
            other @ (SEND | STORED | SEND_REFUSED | SYNC | ACK | BATCH) => {
                Err(field_error("batch.frame.type", FieldProblem::Value(other)))
            }
            other => Ok(Self::Unknown(UnknownPush {
                frame_type: other,
                bytes: item.raw().to_vec(),
            })),
        }
    }
}

cbor::cbor!(Frame);
