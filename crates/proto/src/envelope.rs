//! Envelopes: `docs/protocol/README.md`, "Envelope".

use std::num::NonZeroU64;

use crate::cbor::{self, Codec, DecodeError, FieldProblem, Item, MapOut, NonEmpty, Unknown};
use crate::content::Content;
use crate::id::{ChannelId, DeviceId, MessageId, SpaceId};

/// A message as it travels: the plaintext header the server routes by and the encrypted payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Envelope {
    pub header: Header,
    /// Encrypted as `header.scheme` says: an MLSMessage under [`Scheme::Mls`], an encoded
    /// [`HpkePayload`] under [`Scheme::Hpke`].
    pub payload: Vec<u8>,
}

/// The plaintext header of an [`Envelope`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    /// `v`: the protocol version of the sending device.
    pub v: NonZeroU64,
    /// The space the sending device sends from.
    pub space: SpaceId,
    pub channel: ChannelId,
    pub msg_id: MessageId,
    pub scheme: Scheme,
    /// The recipient devices; `None`, every member device of the channel but the sending one.
    pub recipients: Option<NonEmpty<DeviceId>>,
    /// How long an undelivered message is kept, in seconds; `None`, the server's retention.
    pub ttl: Option<u64>,
    pub unknown: Unknown,
}

/// How a payload is encrypted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Scheme {
    /// An MLSMessage of the channel's MLS group.
    Mls = 1,
    /// Sealed to one device with HPKE and signed by the sending device: [`HpkePayload`].
    Hpke = 2,
}

/// The payload of an envelope under [`Scheme::Hpke`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HpkePayload {
    /// The HPKE encapsulated key.
    pub enc: [u8; 32],
    /// The encoded [`HpkePlaintext`], sealed.
    pub ct: Vec<u8>,
}

/// What an [`HpkePayload`] seals.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HpkePlaintext {
    /// The sending device.
    pub sender: DeviceId,
    pub content: Content,
    /// By `sender` over `["skald hpke", header, content]`.
    pub signature: [u8; 64],
}

impl Codec for Envelope {
    fn encode(&self) -> Vec<u8> {
        cbor::array([self.header.encode(), cbor::bytes(&self.payload)])
    }

    fn decode(item: &Item<'_>) -> Result<Self, DecodeError> {
        let [header, payload] = item.tuple("envelope")?;
        Ok(Self {
            header: Header::decode(header)?,
            payload: payload.bytes("envelope.payload")?.to_vec(),
        })
    }
}

impl Codec for Header {
    fn encode(&self) -> Vec<u8> {
        MapOut::new(&self.unknown)
            .put(1, cbor::uint(self.v.get()))
            .put(2, cbor::id(&self.space))
            .put(3, cbor::id(&self.channel))
            .put(4, cbor::id(&self.msg_id))
            .put(5, cbor::uint(self.scheme as u64))
            .put_some(
                6,
                self.recipients
                    .as_ref()
                    .map(|recipients| cbor::array(recipients.iter().map(cbor::id))),
            )
            .put_some(7, self.ttl.map(cbor::uint))
            .finish()
    }

    fn decode(item: &Item<'_>) -> Result<Self, DecodeError> {
        let mut fields = item.map("header")?;
        Ok(Self {
            v: fields.required(1, "header.v")?.positive("header.v")?,
            space: fields.required(2, "header.space")?.id("header.space")?,
            channel: fields.required(3, "header.channel")?.id("header.channel")?,
            msg_id: fields.required(4, "header.msg_id")?.id("header.msg_id")?,
            scheme: Scheme::decode(fields.required(5, "header.scheme")?)?,
            recipients: fields
                .take(6)
                .map(|item| item.non_empty("header.recipients", |id| id.id("header.recipients")))
                .transpose()?,
            ttl: fields
                .take(7)
                .map(|item| item.uint("header.ttl"))
                .transpose()?,
            unknown: fields.unknown(),
        })
    }
}

impl Scheme {
    fn decode(item: &Item<'_>) -> Result<Self, DecodeError> {
        match item.uint("header.scheme")? {
            1 => Ok(Self::Mls),
            2 => Ok(Self::Hpke),
            other => Err(DecodeError::Field {
                field: "header.scheme",
                problem: FieldProblem::Value(other),
            }),
        }
    }
}

impl Codec for HpkePayload {
    fn encode(&self) -> Vec<u8> {
        cbor::array([cbor::bytes(&self.enc), cbor::bytes(&self.ct)])
    }

    fn decode(item: &Item<'_>) -> Result<Self, DecodeError> {
        let [enc, ct] = item.tuple("hpke-payload")?;
        Ok(Self {
            enc: enc.fixed("hpke-payload.enc")?,
            ct: ct.bytes("hpke-payload.ct")?.to_vec(),
        })
    }
}

impl Codec for HpkePlaintext {
    fn encode(&self) -> Vec<u8> {
        cbor::array([
            cbor::id(&self.sender),
            cbor::bytes(&self.content.encode()),
            cbor::bytes(&self.signature),
        ])
    }

    fn decode(item: &Item<'_>) -> Result<Self, DecodeError> {
        let [sender, content, signature] = item.tuple("hpke-plaintext")?;
        Ok(Self {
            sender: sender.id("hpke-plaintext.sender")?,
            content: content.embedded("hpke-plaintext.content")?,
            signature: signature.fixed("hpke-plaintext.signature")?,
        })
    }
}

cbor::cbor!(Envelope, Header, HpkePayload, HpkePlaintext);
