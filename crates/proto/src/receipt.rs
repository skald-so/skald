//! Receipts: `docs/protocol/delivery.md`, "Receipts" and "Frames".

use crate::cbor::{self, Codec, DecodeError, FieldProblem, Item, MapOut, NonEmpty, Unknown};
use crate::id::{DeviceId, MessageId};

/// The `receipt` body: a status the `to` agent's device issues, signed and encrypted like any
/// content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Receipt {
    pub status: ReceiptStatus,
    /// The `msg_id` of each message it reports on, all from one sending device.
    pub messages: NonEmpty<MessageId>,
    /// Why a message was refused, for its sender.
    pub reason: Option<String>,
    pub unknown: Unknown,
}

/// The status a [`Receipt`] reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReceiptStatus {
    /// The `to` agent's inbound mode holds the message for a human.
    Held = 1,
    /// The `to` agent's inbound mode or a human refused the message.
    Refused = 2,
    /// The message was handed to the `to` agent.
    Read = 3,
}

/// The `stored` frame: the server has committed a sent envelope. Unsigned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stored {
    pub send_key: SendKey,
    /// When the server committed it, in seconds since the Unix epoch.
    pub timestamp: u64,
}

/// Which sent envelope a server answer is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendKey {
    pub msg_id: MessageId,
    /// Under `hpke`, the recipient device.
    pub recipient: Option<DeviceId>,
}

/// The frame type of [`Stored`].
pub(crate) const STORED: u64 = 2;

/// A server status of one message for one recipient device, unsigned: the payload of a
/// `status` frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryStatus {
    /// The message it reports on.
    pub msg_id: MessageId,
    /// The recipient device.
    pub device: DeviceId,
    pub status: ServerStatus,
    /// When it happened, in seconds since the Unix epoch.
    pub timestamp: u64,
    pub unknown: Unknown,
}

/// The status a [`DeliveryStatus`] reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ServerStatus {
    /// The recipient device has written the envelope.
    Delivered = 1,
    /// The recipient device's entry expired before its ack.
    Expired = 2,
}

fn value_error(field: &'static str, value: u64) -> DecodeError {
    DecodeError::Field {
        field,
        problem: FieldProblem::Value(value),
    }
}

impl Codec for Receipt {
    fn encode(&self) -> Vec<u8> {
        MapOut::new(&self.unknown)
            .put(1, cbor::uint(self.status as u64))
            .put(2, cbor::array(self.messages.iter().map(cbor::id)))
            .put_some(3, self.reason.as_deref().map(cbor::text))
            .finish()
    }

    fn decode(item: &Item<'_>) -> Result<Self, DecodeError> {
        let mut fields = item.map("receipt")?;
        let status = match fields
            .required(1, "receipt.status")?
            .uint("receipt.status")?
        {
            1 => ReceiptStatus::Held,
            2 => ReceiptStatus::Refused,
            3 => ReceiptStatus::Read,
            other => return Err(value_error("receipt.status", other)),
        };
        Ok(Self {
            status,
            messages: fields
                .required(2, "receipt.messages")?
                .non_empty("receipt.messages", |id| id.id("receipt.messages"))?,
            reason: fields
                .take(3)
                .map(|item| item.text("receipt.reason").map(str::to_owned))
                .transpose()?,
            unknown: fields.unknown(),
        })
    }
}

impl Codec for Stored {
    fn encode(&self) -> Vec<u8> {
        cbor::array([
            cbor::uint(STORED),
            self.send_key.encode(),
            cbor::uint(self.timestamp),
        ])
    }

    fn decode(item: &Item<'_>) -> Result<Self, DecodeError> {
        let [frame_type, send_key, timestamp] = item.tuple("stored")?;
        match frame_type.uint("stored.type")? {
            STORED => Ok(Self {
                send_key: SendKey::decode(send_key)?,
                timestamp: timestamp.uint("stored.timestamp")?,
            }),
            other => Err(value_error("stored.type", other)),
        }
    }
}

impl Codec for SendKey {
    fn encode(&self) -> Vec<u8> {
        cbor::array(
            std::iter::once(cbor::id(&self.msg_id)).chain(self.recipient.as_ref().map(cbor::id)),
        )
    }

    fn decode(item: &Item<'_>) -> Result<Self, DecodeError> {
        match item.array("send-key")? {
            [] => Err(DecodeError::Field {
                field: "send-key.msg_id",
                problem: FieldProblem::Missing,
            }),
            [msg_id, recipient @ ..] if recipient.len() <= 1 => Ok(Self {
                msg_id: msg_id.id("send-key.msg_id")?,
                recipient: recipient
                    .first()
                    .map(|id| id.id("send-key.recipient"))
                    .transpose()?,
            }),
            items => Err(DecodeError::Field {
                field: "send-key",
                problem: FieldProblem::Items {
                    expected: 2,
                    actual: items.len(),
                },
            }),
        }
    }
}

impl Codec for DeliveryStatus {
    fn encode(&self) -> Vec<u8> {
        MapOut::new(&self.unknown)
            .put(1, cbor::id(&self.msg_id))
            .put(2, cbor::id(&self.device))
            .put(3, cbor::uint(self.status as u64))
            .put(4, cbor::uint(self.timestamp))
            .finish()
    }

    fn decode(item: &Item<'_>) -> Result<Self, DecodeError> {
        let mut fields = item.map("delivery-status")?;
        Ok(Self {
            msg_id: fields
                .required(1, "delivery-status.msg_id")?
                .id("delivery-status.msg_id")?,
            device: fields
                .required(2, "delivery-status.device")?
                .id("delivery-status.device")?,
            status: match fields
                .required(3, "delivery-status.status")?
                .uint("delivery-status.status")?
            {
                1 => ServerStatus::Delivered,
                2 => ServerStatus::Expired,
                other => return Err(value_error("delivery-status.status", other)),
            },
            timestamp: fields
                .required(4, "delivery-status.timestamp")?
                .uint("delivery-status.timestamp")?,
            unknown: fields.unknown(),
        })
    }
}

cbor::cbor!(Receipt, Stored, SendKey, DeliveryStatus);
