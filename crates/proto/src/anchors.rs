//! Anchors objects: `docs/protocol/membership.md`, "Anchors".

use crate::cbor::{self, Codec, DecodeError, Item, Unknown};
use crate::cert::account_and_device;
use crate::id::{AccountId, DeviceId, SpaceId};
use crate::signed::{Signed, SignedBody};

/// The label of an anchors object's signature.
pub const ANCHORS_LABEL: &str = "skald anchors";

/// An anchors object, signed by its `device` over `["skald anchors", body]`.
pub type Anchors = Signed<AnchorsBody>;

/// What an [`Anchors`] object signs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnchorsBody {
    pub account: AccountId,
    /// A `client` device of the account: the signer.
    pub device: DeviceId,
    /// Each space and the hash of the latest revision of it the device accepted, in the body's
    /// order.
    pub anchors: Vec<(SpaceId, [u8; 32])>,
    /// The account's personal space the device pinned, if any.
    pub personal: Option<SpaceId>,
    pub unknown: Unknown,
}

impl SignedBody for AnchorsBody {
    const LABEL: &'static str = ANCHORS_LABEL;
    const FIELDS: [&'static str; 3] = ["anchors", "anchors.body", "anchors.signature"];
}

impl Codec for AnchorsBody {
    fn encode(&self) -> Vec<u8> {
        let anchors = self
            .anchors
            .iter()
            .map(|(space, revision)| cbor::array([cbor::id(space), cbor::bytes(revision)]));
        account_and_device(&self.unknown, &self.account, &self.device)
            .put(3, cbor::array(anchors))
            .put_some(4, self.personal.as_ref().map(cbor::id))
            .finish()
    }

    fn decode(item: &Item<'_>) -> Result<Self, DecodeError> {
        let mut fields = item.map("anchors-body")?;
        let field = "anchors-body.anchors";
        Ok(Self {
            account: fields
                .required(1, "anchors-body.account")?
                .id("anchors-body.account")?,
            device: fields
                .required(2, "anchors-body.device")?
                .id("anchors-body.device")?,
            anchors: fields
                .required(3, field)?
                .array(field)?
                .iter()
                .map(|anchor| {
                    let [space, revision] = anchor.tuple(field)?;
                    Ok((space.id(field)?, revision.fixed(field)?))
                })
                .collect::<Result<_, DecodeError>>()?,
            personal: fields
                .take(4)
                .map(|item| item.id("anchors-body.personal"))
                .transpose()?,
            unknown: fields.unknown(),
        })
    }
}

cbor::cbor!(AnchorsBody);
