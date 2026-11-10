//! Content: `docs/protocol/README.md`, "Content", "Service grant" and "Custodian device";
//! attachments: `docs/protocol/delivery.md`, "Attachments".

use std::num::NonZeroU64;

use crate::address::Address;
use crate::cbor::{self, Codec, DecodeError, FieldProblem, Item, MapOut, NonEmpty, Unknown};
use crate::id::{BlobId, ChannelId, DeviceId, MessageId};
use crate::receipt::Receipt;

/// What an application message carries inside its payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Content {
    /// The sending agent.
    pub from: Address,
    /// The recipient agent; `None`, every agent in the channel.
    pub to: Option<Address>,
    /// The per-sender counter, for gap detection; `None` only from a stateless service device.
    pub counter: Option<NonZeroU64>,
    /// The hop counter, for loop protection.
    pub hops: u64,
    /// `reply_to`: the `msg_id` of the message this one answers.
    pub reply_to: Option<MessageId>,
    /// `None` only in decoded content whose body this version does not know; it is in `unknown`.
    pub body: Option<Body>,
    /// Required in content from a custodian device.
    pub delegation: Option<Delegation>,
    /// Required in content from a service device.
    pub grant: Option<Grant>,
    /// The encrypted files the A2A message refers to.
    pub attachments: Option<NonEmpty<Attachment>>,
    pub unknown: Unknown,
}

/// The one body of a [`Content`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Body {
    /// `a2a`: one A2A Message in its JSON form, as the sender wrote it.
    A2a(String),
    /// `receipt`: `held`, `refused` or `read` for messages of one sending device.
    Receipt(Receipt),
}

/// A space admin's permission for a service device to send in some channels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grant {
    pub body: GrantBody,
    /// By `body.signer` over `["skald grant", body]`.
    pub signature: [u8; 64],
}

/// What a [`Grant`] signs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantBody {
    /// The service device.
    pub service: DeviceId,
    /// The channels it may send in.
    pub channels: NonEmpty<ChannelId>,
    /// The admin's device.
    pub signer: DeviceId,
    /// Seconds since the Unix epoch.
    pub expires: u64,
    pub unknown: Unknown,
}

/// A member's permission for a custodian device to send as one of their agents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delegation {
    pub body: DelegationBody,
    /// By `body.signer` over `["skald delegation", body]`.
    pub signature: [u8; 64],
}

/// What a [`Delegation`] signs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DelegationBody {
    /// The delegated agent.
    pub agent: Address,
    /// The custodian device.
    pub custodian: DeviceId,
    /// The member's device.
    pub signer: DeviceId,
    /// Seconds since the Unix epoch.
    pub expires: u64,
    pub unknown: Unknown,
}

/// An encrypted file uploaded apart from the message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attachment {
    /// The blob's ID, assigned by the server.
    pub blob: BlobId,
    /// The blob's AES-128-GCM key.
    pub key: [u8; 16],
    /// SHA-256 of the blob.
    pub digest: [u8; 32],
    /// The file's length in bytes.
    pub size: u64,
    pub unknown: Unknown,
}

/// The content keys of the bodies this version knows.
const A2A: u64 = 6;
const RECEIPT: u64 = 9;

impl Codec for Content {
    fn encode(&self) -> Vec<u8> {
        let (a2a, receipt) = match &self.body {
            Some(Body::A2a(json)) => (Some(cbor::text(json)), None),
            Some(Body::Receipt(receipt)) => (None, Some(receipt.encode())),
            None => (None, None),
        };
        MapOut::new(&self.unknown)
            .put(1, cbor::text(&self.from.to_string()))
            .put_some(2, self.to.as_ref().map(|to| cbor::text(&to.to_string())))
            .put_some(3, self.counter.map(|counter| cbor::uint(counter.get())))
            .put(4, cbor::uint(self.hops))
            .put_some(5, self.reply_to.as_ref().map(cbor::id))
            .put_some(A2A, a2a)
            .put_some(7, self.delegation.as_ref().map(Codec::encode))
            .put_some(8, self.grant.as_ref().map(Codec::encode))
            .put_some(RECEIPT, receipt)
            .put_some(
                10,
                self.attachments
                    .as_ref()
                    .map(|attachments| cbor::array(attachments.iter().map(Codec::encode))),
            )
            .finish()
    }

    fn decode(item: &Item<'_>) -> Result<Self, DecodeError> {
        let mut fields = item.map("content")?;
        let from = fields
            .required(1, "content.from")?
            .address("content.from")?;
        let to = fields
            .take(2)
            .map(|item| item.address("content.to"))
            .transpose()?;
        let counter = fields
            .take(3)
            .map(|item| item.positive("content.counter"))
            .transpose()?;
        let hops = fields.required(4, "content.hops")?.uint("content.hops")?;
        let reply_to = fields
            .take(5)
            .map(|item| item.id("content.reply_to"))
            .transpose()?;
        let a2a = fields
            .take(A2A)
            .map(|item| {
                item.text("content.a2a")
                    .map(|json| Body::A2a(json.to_owned()))
            })
            .transpose()?;
        let receipt = fields
            .take(RECEIPT)
            .map(|item| Receipt::decode(item).map(Body::Receipt))
            .transpose()?;
        let delegation = fields.take(7).map(Delegation::decode).transpose()?;
        let grant = fields.take(8).map(Grant::decode).transpose()?;
        let attachments = fields
            .take(10)
            .map(|item| item.non_empty("content.attachments", Attachment::decode))
            .transpose()?;
        let body = match (a2a, receipt) {
            (Some(_), Some(_)) => return Err(body_error(FieldProblem::TwoBodies)),
            // Any field left may be the body of a later version.
            (None, None) if fields.is_empty() => return Err(body_error(FieldProblem::NoBody)),
            (a2a, receipt) => a2a.or(receipt),
        };
        if matches!(body, Some(Body::Receipt(_))) {
            check_receipt(&from, to.as_ref(), hops, attachments.is_some())?;
        }
        Ok(Self {
            from,
            to,
            counter,
            hops,
            reply_to,
            body,
            delegation,
            grant,
            attachments,
            unknown: fields.unknown(),
        })
    }
}

/// The fields `docs/protocol/delivery.md`, "Receipts" fixes for content with a `receipt` body.
fn check_receipt(
    from: &Address,
    to: Option<&Address>,
    hops: u64,
    attachments: bool,
) -> Result<(), DecodeError> {
    let (field, problem) = if from.is_person() {
        ("content.from", FieldProblem::NotAllowed)
    } else if to.is_none() {
        ("content.to", FieldProblem::Missing)
    } else if hops != 0 {
        ("content.hops", FieldProblem::Value(hops))
    } else if attachments {
        ("content.attachments", FieldProblem::NotAllowed)
    } else {
        return Ok(());
    };
    Err(DecodeError::Field { field, problem })
}

fn body_error(problem: FieldProblem) -> DecodeError {
    DecodeError::Field {
        field: "content",
        problem,
    }
}

impl Codec for Grant {
    fn encode(&self) -> Vec<u8> {
        cbor::array([
            cbor::bytes(&self.body.encode()),
            cbor::bytes(&self.signature),
        ])
    }

    fn decode(item: &Item<'_>) -> Result<Self, DecodeError> {
        let [body, signature] = item.tuple("grant")?;
        Ok(Self {
            body: body.embedded("grant.body")?,
            signature: signature.fixed("grant.signature")?,
        })
    }
}

impl Codec for GrantBody {
    fn encode(&self) -> Vec<u8> {
        MapOut::new(&self.unknown)
            .put(1, cbor::id(&self.service))
            .put(2, cbor::array(self.channels.iter().map(cbor::id)))
            .put(3, cbor::id(&self.signer))
            .put(4, cbor::uint(self.expires))
            .finish()
    }

    fn decode(item: &Item<'_>) -> Result<Self, DecodeError> {
        let mut fields = item.map("grant-body")?;
        Ok(Self {
            service: fields
                .required(1, "grant-body.service")?
                .id("grant-body.service")?,
            channels: fields
                .required(2, "grant-body.channels")?
                .non_empty("grant-body.channels", |id| id.id("grant-body.channels"))?,
            signer: fields
                .required(3, "grant-body.signer")?
                .id("grant-body.signer")?,
            expires: fields
                .required(4, "grant-body.expires")?
                .uint("grant-body.expires")?,
            unknown: fields.unknown(),
        })
    }
}

impl Codec for Delegation {
    fn encode(&self) -> Vec<u8> {
        cbor::array([
            cbor::bytes(&self.body.encode()),
            cbor::bytes(&self.signature),
        ])
    }

    fn decode(item: &Item<'_>) -> Result<Self, DecodeError> {
        let [body, signature] = item.tuple("delegation")?;
        Ok(Self {
            body: body.embedded("delegation.body")?,
            signature: signature.fixed("delegation.signature")?,
        })
    }
}

impl Codec for DelegationBody {
    fn encode(&self) -> Vec<u8> {
        MapOut::new(&self.unknown)
            .put(1, cbor::text(&self.agent.to_string()))
            .put(2, cbor::id(&self.custodian))
            .put(3, cbor::id(&self.signer))
            .put(4, cbor::uint(self.expires))
            .finish()
    }

    fn decode(item: &Item<'_>) -> Result<Self, DecodeError> {
        let mut fields = item.map("delegation-body")?;
        Ok(Self {
            agent: fields
                .required(1, "delegation-body.agent")?
                .address("delegation-body.agent")?,
            custodian: fields
                .required(2, "delegation-body.custodian")?
                .id("delegation-body.custodian")?,
            signer: fields
                .required(3, "delegation-body.signer")?
                .id("delegation-body.signer")?,
            expires: fields
                .required(4, "delegation-body.expires")?
                .uint("delegation-body.expires")?,
            unknown: fields.unknown(),
        })
    }
}

impl Codec for Attachment {
    fn encode(&self) -> Vec<u8> {
        MapOut::new(&self.unknown)
            .put(1, cbor::id(&self.blob))
            .put(2, cbor::bytes(&self.key))
            .put(3, cbor::bytes(&self.digest))
            .put(4, cbor::uint(self.size))
            .finish()
    }

    fn decode(item: &Item<'_>) -> Result<Self, DecodeError> {
        let mut fields = item.map("attachment")?;
        Ok(Self {
            blob: fields
                .required(1, "attachment.blob")?
                .id("attachment.blob")?,
            key: fields
                .required(2, "attachment.key")?
                .fixed("attachment.key")?,
            digest: fields
                .required(3, "attachment.digest")?
                .fixed("attachment.digest")?,
            size: fields
                .required(4, "attachment.size")?
                .uint("attachment.size")?,
            unknown: fields.unknown(),
        })
    }
}

cbor::cbor!(
    Content,
    Grant,
    GrantBody,
    Delegation,
    DelegationBody,
    Attachment
);
