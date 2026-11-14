//! A channel's history: `docs/protocol/membership.md`, "Key chain", "Wraps" and "History entry".

use crate::cbor::{self, Codec, DecodeError, FieldProblem, Item, MapOut, Unknown, field_error};
use crate::content::Content;
use crate::id::{AccountId, ChannelId, DeviceId, MessageId};
use crate::signed::{Signed, SignedBody};

/// The label of a history plaintext's signature.
pub const HISTORY_LABEL: &str = "skald history";

/// A link of the key chain: the key of epoch `prev`, encrypted under the key of `epoch`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryLink {
    pub channel: ChannelId,
    /// At least 2.
    pub epoch: u64,
    /// The latest epoch before `epoch` its uploader judged clean; at least 1.
    pub prev: u64,
    pub nonce: [u8; 12],
    /// The key of `prev`, encrypted.
    pub ciphertext: Vec<u8>,
    pub unknown: Unknown,
}

impl HistoryLink {
    /// The associated data of a link: the encoded `["skald history link", channel, epoch, prev]`.
    pub fn associated_data(channel: ChannelId, epoch: u64, prev: u64) -> Vec<u8> {
        cbor::array([
            cbor::text("skald history link"),
            cbor::id(&channel),
            cbor::uint(epoch),
            cbor::uint(prev),
        ])
    }
}

/// A copy of a message under the history key of its epoch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEntry {
    pub channel: ChannelId,
    /// At least 1.
    pub epoch: u64,
    pub nonce: [u8; 12],
    /// The encoded [`HistoryPlaintext`], encrypted.
    pub ciphertext: Vec<u8>,
    pub unknown: Unknown,
}

impl HistoryEntry {
    /// The associated data of an entry: the encoded `["skald history entry", channel, epoch]`.
    pub fn associated_data(channel: ChannelId, epoch: u64) -> Vec<u8> {
        cbor::array([
            cbor::text("skald history entry"),
            cbor::id(&channel),
            cbor::uint(epoch),
        ])
    }
}

/// An epoch's history key sealed to a browser or an account's recovery key, signed by `signer`
/// over `["skald wrap", body]`.
pub type HistoryWrap = Signed<HistoryWrapBody>;

/// The body of a [`HistoryWrap`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryWrapBody {
    pub channel: ChannelId,
    /// At least 1.
    pub epoch: u64,
    pub recipient: WrapRecipient,
    /// A member device of the epoch.
    pub signer: DeviceId,
    /// The HPKE encapsulated key.
    pub enc: [u8; 32],
    /// The epoch's key, sealed.
    pub ct: Vec<u8>,
    pub unknown: Unknown,
}

impl HistoryWrapBody {
    /// The associated data of a wrap's seal: the encoded `[channel, epoch, recipient]`.
    pub fn associated_data(channel: ChannelId, epoch: u64, recipient: &WrapRecipient) -> Vec<u8> {
        cbor::array([cbor::id(&channel), cbor::uint(epoch), recipient.encode()])
    }
}

impl SignedBody for HistoryWrapBody {
    const LABEL: &'static str = "skald wrap";
    const FIELDS: [&'static str; 3] = [
        "history-wrap",
        "history-wrap.body",
        "history-wrap.signature",
    ];
}

/// Whom a wrap is sealed to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WrapRecipient {
    /// A `browser` device, to the `hpke` key of its certificate.
    Browser(DeviceId),
    /// An account's recovery key, to the recovery HPKE key of its record.
    Recovery(AccountId),
}

/// What a [`HistoryEntry`] encrypts: its body signed by `sender` over `["skald history", body]`.
pub type HistoryPlaintext = Signed<HistoryBody>;

/// The body of a [`HistoryPlaintext`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryBody {
    /// The sending device.
    pub sender: DeviceId,
    pub channel: ChannelId,
    pub msg_id: MessageId,
    /// When it was sent, as the sender states it, in seconds since the Unix epoch.
    pub timestamp: u64,
    pub content: Content,
    pub unknown: Unknown,
}

impl SignedBody for HistoryBody {
    const LABEL: &'static str = HISTORY_LABEL;
    const FIELDS: [&'static str; 3] = [
        "history-plaintext",
        "history-plaintext.body",
        "history-plaintext.signature",
    ];
}

/// An unsigned integer of at least `min`, for CDDL `uint .ge min`.
fn at_least(item: &Item<'_>, field: &'static str, min: u64) -> Result<u64, DecodeError> {
    let value = item.uint(field)?;
    if value < min {
        return Err(field_error(
            field,
            FieldProblem::Below { min, actual: value },
        ));
    }
    Ok(value)
}

/// The fields a link, an entry and a wrap begin with, `channel` and `epoch`, and the `unknown` ones.
fn head(unknown: &Unknown, channel: &ChannelId, epoch: u64) -> MapOut {
    MapOut::new(unknown)
        .put(1, cbor::id(channel))
        .put(2, cbor::uint(epoch))
}

impl Codec for HistoryLink {
    fn encode(&self) -> Vec<u8> {
        head(&self.unknown, &self.channel, self.epoch)
            .put(3, cbor::uint(self.prev))
            .put(4, cbor::bytes(&self.nonce))
            .put(5, cbor::bytes(&self.ciphertext))
            .finish()
    }

    fn decode(item: &Item<'_>) -> Result<Self, DecodeError> {
        let mut fields = item.map("history-link")?;
        Ok(Self {
            channel: fields
                .required(1, "history-link.channel")?
                .id("history-link.channel")?,
            epoch: at_least(
                fields.required(2, "history-link.epoch")?,
                "history-link.epoch",
                2,
            )?,
            prev: at_least(
                fields.required(3, "history-link.prev")?,
                "history-link.prev",
                1,
            )?,
            nonce: fields
                .required(4, "history-link.nonce")?
                .fixed("history-link.nonce")?,
            ciphertext: fields
                .required(5, "history-link.ciphertext")?
                .bytes("history-link.ciphertext")?
                .to_vec(),
            unknown: fields.unknown(),
        })
    }
}

impl Codec for HistoryEntry {
    fn encode(&self) -> Vec<u8> {
        head(&self.unknown, &self.channel, self.epoch)
            .put(3, cbor::bytes(&self.nonce))
            .put(4, cbor::bytes(&self.ciphertext))
            .finish()
    }

    fn decode(item: &Item<'_>) -> Result<Self, DecodeError> {
        let mut fields = item.map("history-entry")?;
        Ok(Self {
            channel: fields
                .required(1, "history-entry.channel")?
                .id("history-entry.channel")?,
            epoch: at_least(
                fields.required(2, "history-entry.epoch")?,
                "history-entry.epoch",
                1,
            )?,
            nonce: fields
                .required(3, "history-entry.nonce")?
                .fixed("history-entry.nonce")?,
            ciphertext: fields
                .required(4, "history-entry.ciphertext")?
                .bytes("history-entry.ciphertext")?
                .to_vec(),
            unknown: fields.unknown(),
        })
    }
}

impl Codec for HistoryBody {
    fn encode(&self) -> Vec<u8> {
        MapOut::new(&self.unknown)
            .put(1, cbor::id(&self.sender))
            .put(2, cbor::id(&self.channel))
            .put(3, cbor::id(&self.msg_id))
            .put(4, cbor::uint(self.timestamp))
            .put(5, cbor::bytes(&self.content.encode()))
            .finish()
    }

    fn decode(item: &Item<'_>) -> Result<Self, DecodeError> {
        let mut fields = item.map("history-body")?;
        Ok(Self {
            sender: fields
                .required(1, "history-body.sender")?
                .id("history-body.sender")?,
            channel: fields
                .required(2, "history-body.channel")?
                .id("history-body.channel")?,
            msg_id: fields
                .required(3, "history-body.msg_id")?
                .id("history-body.msg_id")?,
            timestamp: fields
                .required(4, "history-body.timestamp")?
                .uint("history-body.timestamp")?,
            content: fields
                .required(5, "history-body.content")?
                .embedded("history-body.content")?,
            unknown: fields.unknown(),
        })
    }
}

impl Codec for HistoryWrapBody {
    fn encode(&self) -> Vec<u8> {
        head(&self.unknown, &self.channel, self.epoch)
            .put(3, self.recipient.encode())
            .put(4, cbor::id(&self.signer))
            .put(5, cbor::bytes(&self.enc))
            .put(6, cbor::bytes(&self.ct))
            .finish()
    }

    fn decode(item: &Item<'_>) -> Result<Self, DecodeError> {
        let mut fields = item.map("history-wrap-body")?;
        Ok(Self {
            channel: fields
                .required(1, "history-wrap-body.channel")?
                .id("history-wrap-body.channel")?,
            epoch: at_least(
                fields.required(2, "history-wrap-body.epoch")?,
                "history-wrap-body.epoch",
                1,
            )?,
            recipient: WrapRecipient::decode(fields.required(3, "history-wrap-body.recipient")?)?,
            signer: fields
                .required(4, "history-wrap-body.signer")?
                .id("history-wrap-body.signer")?,
            enc: fields
                .required(5, "history-wrap-body.enc")?
                .fixed("history-wrap-body.enc")?,
            ct: fields
                .required(6, "history-wrap-body.ct")?
                .bytes("history-wrap-body.ct")?
                .to_vec(),
            unknown: fields.unknown(),
        })
    }
}

impl Codec for WrapRecipient {
    fn encode(&self) -> Vec<u8> {
        match self {
            Self::Browser(device) => cbor::array([cbor::uint(1), cbor::id(device)]),
            Self::Recovery(account) => cbor::array([cbor::uint(2), cbor::id(account)]),
        }
    }

    fn decode(item: &Item<'_>) -> Result<Self, DecodeError> {
        let [kind, id] = item.tuple("wrap-recipient")?;
        match kind.uint("wrap-recipient.kind")? {
            1 => Ok(Self::Browser(id.id("wrap-recipient.device")?)),
            2 => Ok(Self::Recovery(id.id("wrap-recipient.account")?)),
            other => Err(field_error(
                "wrap-recipient.kind",
                FieldProblem::Value(other),
            )),
        }
    }
}

cbor::cbor!(HistoryLink, HistoryEntry, HistoryBody, HistoryWrapBody);
