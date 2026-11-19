//! Delivery frames: `docs/protocol/delivery.md`, "Frames".

use std::num::NonZeroU64;

use crate::cbor::{
    self, Cbor, Codec, DecodeError, FieldProblem, Item, decode_embedded, field_error,
};
use crate::envelope::{Envelope, Header, HpkePayload, Scheme};
use crate::id::{ChannelId, DeviceId, MessageId, SpaceId};
use crate::receipt::{DeliveryStatus, STORED, SendKey, Stored};

/// The label of `auth`'s signature.
pub const AUTH_LABEL: &str = "skald auth";
/// The label of the signature of a [`RequestProof`].
pub const REQUEST_LABEL: &str = "skald request";
/// The HTTPS request header that carries a [`RequestProof`], in base64url without padding.
pub const PROOF_HEADER: &str = "Skald-Device-Proof";

/// The proof of a device in an HTTPS request: `docs/protocol/delivery.md`, "Connecting". As `auth`,
/// without a frame type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestProof {
    /// The challenge the server issued to the client's session.
    pub challenge: Vec<u8>,
    pub device: DeviceId,
    /// By the device's signing key over `["skald request", challenge, origin]`.
    pub signature: [u8; 64],
}

impl Codec for RequestProof {
    fn encode(&self) -> Vec<u8> {
        cbor::array([
            cbor::bytes(&self.challenge),
            cbor::id(&self.device),
            cbor::bytes(&self.signature),
        ])
    }

    fn decode(item: &Item<'_>) -> Result<Self, DecodeError> {
        let [challenge, device, signature] = item.tuple("proof")?;
        Ok(Self {
            challenge: challenge.bytes("proof.challenge")?.to_vec(),
            device: device.id("proof.device")?,
            signature: signature.fixed("proof.signature")?,
        })
    }
}

/// A frame between a client and the server. A frame of a type this version does not know decodes
/// as [`Frame::Unknown`], so its reader can ignore it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    /// `auth`, client → server: the first frame on a WebSocket, which proves the connection's
    /// device.
    Auth {
        /// The challenge the server issued to the client's session.
        challenge: Vec<u8>,
        device: DeviceId,
        /// By the device's signing key over `["skald auth", challenge, origin]`.
        signature: [u8; 64],
    },
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
    /// `message`, `status`, `space-notice` or `link-notice`, server → client.
    Push(Push),
    /// `ack`, client → server: the device's cursor after writing entries.
    Ack { seq: NonZeroU64 },
    /// `batch`, server → client: the answer to a long poll.
    Batch(Vec<Push>),
    /// `history`, client → server: an item of a channel's history to keep.
    History {
        /// The client's number for the upload, which the answer names.
        upload: u64,
        space: SpaceId,
        channel: ChannelId,
        item: HistoryItem,
    },
    /// `history-stored`, server → client: the item of `upload` is kept, now or before.
    HistoryStored { upload: u64 },
    /// `history-refused`, server → client: the item of `upload` is not kept.
    HistoryRefused {
        upload: u64,
        /// The problem, possibly one this version does not know.
        problem: String,
    },
    /// A frame of a type this version does not know; holds its type. Encodes as an array of the
    /// type alone.
    Unknown(u64),
}

/// An item of a channel's history in a `history` frame, as the bytes the client encoded: the server
/// keeps and gives them as they came, and refuses, rather than closing the connection, those that do
/// not decode as their kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryItem {
    /// An encoded `history-link`.
    Link(Vec<u8>),
    /// An encoded `history-wrap`.
    Wrap(Vec<u8>),
    /// An encoded `history-entry` of the message `msg_id`, which the server cannot read in it.
    Entry { msg_id: MessageId, entry: Vec<u8> },
}

// The kinds of a history item.
const LINK: u64 = 1;
const WRAP: u64 = 2;
const ENTRY: u64 = 3;

impl Codec for HistoryItem {
    fn encode(&self) -> Vec<u8> {
        match self {
            Self::Link(link) => cbor::array([cbor::uint(LINK), cbor::bytes(link)]),
            Self::Wrap(wrap) => cbor::array([cbor::uint(WRAP), cbor::bytes(wrap)]),
            Self::Entry { msg_id, entry } => {
                cbor::array([cbor::uint(ENTRY), cbor::id(msg_id), cbor::bytes(entry)])
            }
        }
    }

    fn decode(item: &Item<'_>) -> Result<Self, DecodeError> {
        Ok(match frame_type(item, "history.item.kind")? {
            LINK => {
                let [_, link] = item.tuple("history.item")?;
                Self::Link(link.bytes("history.item.link")?.to_vec())
            }
            WRAP => {
                let [_, wrap] = item.tuple("history.item")?;
                Self::Wrap(wrap.bytes("history.item.wrap")?.to_vec())
            }
            ENTRY => {
                let [_, msg_id, entry] = item.tuple("history.item")?;
                Self::Entry {
                    msg_id: msg_id.id("history.item.msg_id")?,
                    entry: entry.bytes("history.item.entry")?.to_vec(),
                }
            }
            other => {
                return Err(field_error("history.item.kind", FieldProblem::Value(other)));
            }
        })
    }
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
    /// `link-notice`: a device of the account waits for linking.
    LinkNotice,
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
const AUTH: u64 = 10;
const HISTORY: u64 = 11;
const HISTORY_STORED: u64 = 12;
const HISTORY_REFUSED: u64 = 13;
const LINK_NOTICE: u64 = 14;

impl Frame {
    /// The header of the envelope in the `send` frame `bytes`, when all that keeps the frame from
    /// decoding is an `hpke` header whose `recipients` is not one device. A server refuses such a
    /// send by its header, with `unknown-recipient`, rather than as a malformed frame.
    pub fn misaddressed_send(bytes: &[u8]) -> Option<Header> {
        let item = cbor::parse(bytes).ok()?;
        if frame_type(&item, "frame.type").ok()? != SEND {
            return None;
        }
        let [_, envelope, report] = item.tuple("send").ok()?;
        report.boolean("send.report").ok()?;
        let envelope = cbor::parse(envelope.bytes("send.envelope").ok()?).ok()?;
        let [header, payload] = envelope.tuple("envelope").ok()?;
        let header = Header::decode_fields(header).ok()?;
        payload.embedded::<HpkePayload>("envelope.payload").ok()?;
        let one = header
            .recipients
            .as_ref()
            .is_some_and(|devices| devices.len() == 1);
        (header.scheme == Scheme::Hpke && !one).then_some(header)
    }
}

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
            Self::Auth {
                challenge,
                device,
                signature,
            } => cbor::array([
                cbor::uint(AUTH),
                cbor::bytes(challenge),
                cbor::id(device),
                cbor::bytes(signature),
            ]),
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
            Self::History {
                upload,
                space,
                channel,
                item,
            } => cbor::array([
                cbor::uint(HISTORY),
                cbor::uint(*upload),
                cbor::id(space),
                cbor::id(channel),
                item.encode(),
            ]),
            Self::HistoryStored { upload } => {
                cbor::array([cbor::uint(HISTORY_STORED), cbor::uint(*upload)])
            }
            Self::HistoryRefused { upload, problem } => cbor::array([
                cbor::uint(HISTORY_REFUSED),
                cbor::uint(*upload),
                cbor::text(problem),
            ]),
            Self::Unknown(frame_type) => cbor::array([cbor::uint(*frame_type)]),
        }
    }

    fn decode(item: &Item<'_>) -> Result<Self, DecodeError> {
        Ok(match frame_type(item, "frame.type")? {
            AUTH => {
                let [_, challenge, device, signature] = item.tuple("auth")?;
                Self::Auth {
                    challenge: challenge.bytes("auth.challenge")?.to_vec(),
                    device: device.id("auth.device")?,
                    signature: signature.fixed("auth.signature")?,
                }
            }
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
            MESSAGE | STATUS | SPACE_NOTICE | LINK_NOTICE => Self::Push(Push::decode(item)?),
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
            HISTORY => {
                let [_, upload, space, channel, history_item] = item.tuple("history")?;
                Self::History {
                    upload: upload.uint("history.upload")?,
                    space: space.id("history.space")?,
                    channel: channel.id("history.channel")?,
                    item: HistoryItem::decode(history_item)?,
                }
            }
            HISTORY_STORED => {
                let [_, upload] = item.tuple("history-stored")?;
                Self::HistoryStored {
                    upload: upload.uint("history-stored.upload")?,
                }
            }
            HISTORY_REFUSED => {
                let [_, upload, problem] = item.tuple("history-refused")?;
                Self::HistoryRefused {
                    upload: upload.uint("history-refused.upload")?,
                    problem: problem.text("history-refused.problem")?.to_owned(),
                }
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
            Self::LinkNotice => cbor::array([cbor::uint(LINK_NOTICE)]),
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
            LINK_NOTICE => {
                let [_] = item.tuple("link-notice")?;
                Ok(Self::LinkNotice)
            }
            // A `batch` holds no frame of another known type.
            other @ (SEND | STORED | SEND_REFUSED | SYNC | ACK | BATCH | AUTH | HISTORY
            | HISTORY_STORED | HISTORY_REFUSED) => {
                Err(field_error("batch.frame.type", FieldProblem::Value(other)))
            }
            other => Ok(Self::Unknown(UnknownPush {
                frame_type: other,
                bytes: item.raw().to_vec(),
            })),
        }
    }
}

cbor::cbor!(Frame, RequestProof);
