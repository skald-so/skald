//! Device revocations: `docs/protocol/membership.md`, "Revocation".

use crate::cbor::{self, Codec, DecodeError, Item, NonEmpty, Unknown};
use crate::cert::account_and_device;
use crate::id::{AccountId, DeviceId};
use crate::signed::{Signed, SignedBody};

/// The label of a revocation's signature.
pub const REVOCATION_LABEL: &str = "skald revocation";

/// A revocation, signed by its `signer` device or the account's master key over
/// `["skald revocation", body]`.
pub type Revocation = Signed<RevocationBody>;

/// What a [`Revocation`] signs. A body with both `signer` and `also` decodes; checks ignore it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevocationBody {
    pub account: AccountId,
    /// The revoked device.
    pub device: DeviceId,
    /// Certified devices that stay valid.
    pub keep: Option<NonEmpty<DeviceId>>,
    /// The signing device; `None`, the master key.
    pub signer: Option<DeviceId>,
    /// Seconds since the Unix epoch, for display.
    pub timestamp: u64,
    /// Devices that stay revoked; only with the master key.
    pub also: Option<NonEmpty<DeviceId>>,
    pub unknown: Unknown,
}

impl SignedBody for RevocationBody {
    const LABEL: &'static str = REVOCATION_LABEL;
    const FIELDS: [&'static str; 3] = ["revocation", "revocation.body", "revocation.signature"];
}

fn encode_ids(ids: &Option<NonEmpty<DeviceId>>) -> Option<Vec<u8>> {
    ids.as_ref()
        .map(|ids| cbor::array(ids.iter().map(cbor::id)))
}

fn decode_ids(
    item: Option<&Item<'_>>,
    field: &'static str,
) -> Result<Option<NonEmpty<DeviceId>>, DecodeError> {
    item.map(|item| item.non_empty(field, |id| id.id(field)))
        .transpose()
}

impl Codec for RevocationBody {
    fn encode(&self) -> Vec<u8> {
        account_and_device(&self.unknown, &self.account, &self.device)
            .put_some(3, encode_ids(&self.keep))
            .put_some(4, self.signer.as_ref().map(cbor::id))
            .put(5, cbor::uint(self.timestamp))
            .put_some(6, encode_ids(&self.also))
            .finish()
    }

    fn decode(item: &Item<'_>) -> Result<Self, DecodeError> {
        let mut fields = item.map("revocation-body")?;
        Ok(Self {
            account: fields
                .required(1, "revocation-body.account")?
                .id("revocation-body.account")?,
            device: fields
                .required(2, "revocation-body.device")?
                .id("revocation-body.device")?,
            keep: decode_ids(fields.take(3), "revocation-body.keep")?,
            signer: fields
                .take(4)
                .map(|item| item.id("revocation-body.signer"))
                .transpose()?,
            timestamp: fields
                .required(5, "revocation-body.timestamp")?
                .uint("revocation-body.timestamp")?,
            also: decode_ids(fields.take(6), "revocation-body.also")?,
            unknown: fields.unknown(),
        })
    }
}

cbor::cbor!(RevocationBody);
