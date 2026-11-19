//! A `service` device's key secret: `docs/protocol/membership.md`, "Service device".

use std::fmt;

use zeroize::Zeroize as _;

use crate::cbor::{self, Codec, DecodeError, Item, MapOut, Unknown};
use crate::id::{AccountId, SpaceId};

/// What a `service` device that keeps no state between runs starts each run from: its private
/// keys and what it pins. Its private keys are zeroed on drop.
#[derive(Clone, PartialEq, Eq)]
pub struct ServiceKey {
    /// The Ed25519 private key, its seed.
    pub signing: [u8; 32],
    /// The X25519 private key.
    pub hpke: [u8; 32],
    /// The account the device is certified on.
    pub account: AccountId,
    /// The account's master public key.
    pub master: [u8; 32],
    pub space: SpaceId,
    /// The hash of the roster revision with the device's entry: its anchor.
    pub anchor: [u8; 32],
    /// The server's public address, such as `https://api.skald.so`.
    pub server: String,
    pub unknown: Unknown,
}

impl fmt::Debug for ServiceKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServiceKey")
            .field("account", &self.account)
            .field("space", &self.space)
            .field("server", &self.server)
            .finish_non_exhaustive()
    }
}

impl Drop for ServiceKey {
    fn drop(&mut self) {
        self.signing.zeroize();
        self.hpke.zeroize();
    }
}

impl Codec for ServiceKey {
    fn encode(&self) -> Vec<u8> {
        MapOut::new(&self.unknown)
            .put(1, cbor::bytes(&self.signing))
            .put(2, cbor::bytes(&self.hpke))
            .put(3, cbor::id(&self.account))
            .put(4, cbor::bytes(&self.master))
            .put(5, cbor::id(&self.space))
            .put(6, cbor::bytes(&self.anchor))
            .put(7, cbor::text(&self.server))
            .finish()
    }

    fn decode(item: &Item<'_>) -> Result<Self, DecodeError> {
        let mut fields = item.map("service-key")?;
        Ok(Self {
            signing: fields
                .required(1, "service-key.signing")?
                .fixed("service-key.signing")?,
            hpke: fields
                .required(2, "service-key.hpke")?
                .fixed("service-key.hpke")?,
            account: fields
                .required(3, "service-key.account")?
                .id("service-key.account")?,
            master: fields
                .required(4, "service-key.master")?
                .fixed("service-key.master")?,
            space: fields
                .required(5, "service-key.space")?
                .id("service-key.space")?,
            anchor: fields
                .required(6, "service-key.anchor")?
                .fixed("service-key.anchor")?,
            server: fields
                .required(7, "service-key.server")?
                .text("service-key.server")?
                .to_owned(),
            unknown: fields.unknown(),
        })
    }
}

cbor::cbor!(ServiceKey);
