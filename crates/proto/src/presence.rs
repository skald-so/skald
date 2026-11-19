//! Presence: `docs/protocol/agents.md`, "Presence".

use crate::cbor::{self, Codec, DecodeError, Item, MapOut, Unknown};
use crate::history::at_least;
use crate::id::{ChannelId, DeviceId};

/// A device's presence value in a channel, as it uploads it and the server keeps and gives it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Presence {
    /// The uploading device.
    pub device: DeviceId,
    pub channel: ChannelId,
    /// The epoch whose presence key encrypts `ct`; at least 1.
    pub epoch: u64,
    pub nonce: [u8; 12],
    /// The padded, encrypted `presence-plaintext`.
    pub ct: Vec<u8>,
    pub unknown: Unknown,
}

impl Codec for Presence {
    fn encode(&self) -> Vec<u8> {
        MapOut::new(&self.unknown)
            .put(1, cbor::id(&self.device))
            .put(2, cbor::id(&self.channel))
            .put(3, cbor::uint(self.epoch))
            .put(4, cbor::bytes(&self.nonce))
            .put(5, cbor::bytes(&self.ct))
            .finish()
    }

    fn decode(item: &Item<'_>) -> Result<Self, DecodeError> {
        let mut fields = item.map("presence")?;
        Ok(Self {
            device: fields
                .required(1, "presence.device")?
                .id("presence.device")?,
            channel: fields
                .required(2, "presence.channel")?
                .id("presence.channel")?,
            epoch: at_least(fields.required(3, "presence.epoch")?, "presence.epoch", 1)?,
            nonce: fields
                .required(4, "presence.nonce")?
                .fixed("presence.nonce")?,
            ct: fields
                .required(5, "presence.ct")?
                .bytes("presence.ct")?
                .to_vec(),
            unknown: fields.unknown(),
        })
    }
}

cbor::cbor!(Presence);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cbor::Cbor;

    fn value() -> Presence {
        Presence {
            device: DeviceId::from_signing_key(&[1; 32]),
            channel: ChannelId::generate(),
            epoch: 3,
            nonce: [2; 12],
            ct: vec![3; 4112],
            unknown: Unknown::default(),
        }
    }

    #[test]
    fn a_value_round_trips_byte_for_byte() {
        let value = value();
        let encoded = value.to_cbor();
        let decoded = Presence::from_cbor(&encoded).expect("a value decodes");
        assert_eq!(decoded, value);
        assert_eq!(decoded.to_cbor(), encoded);
    }

    #[test]
    fn a_value_of_epoch_0_or_with_a_short_nonce_does_not_decode() {
        let encoded = Presence {
            epoch: 0,
            ..value()
        }
        .to_cbor();
        assert!(Presence::from_cbor(&encoded).is_err(), "epoch 0");
        // The nonce, a byte string of 12 bytes, made one of 11.
        let mut encoded = value().to_cbor();
        let nonce = [&[0x4c][..], &[2; 12]].concat();
        let at = (encoded.windows(13).position(|window| window == nonce)).expect("the nonce");
        encoded.splice(at..at + 13, [&[0x4b][..], &[2; 11]].concat());
        assert!(Presence::from_cbor(&encoded).is_err(), "a short nonce");
    }
}
