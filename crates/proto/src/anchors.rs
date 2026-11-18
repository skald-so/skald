//! Anchors objects: `docs/protocol/membership.md`, "Anchors".

use crate::cbor::{self, Codec, DecodeError, Fields, Item, MapOut, Unknown};
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
        let map = account_and_device(&self.unknown, &self.account, &self.device);
        finish_anchors(map, &self.anchors, self.personal.as_ref())
    }

    fn decode(item: &Item<'_>) -> Result<Self, DecodeError> {
        let mut fields = item.map("anchors-body")?;
        let account = (fields.required(1, "anchors-body.account")?).id("anchors-body.account")?;
        let device = (fields.required(2, "anchors-body.device")?).id("anchors-body.device")?;
        let names = ["anchors-body.anchors", "anchors-body.personal"];
        let (anchors, personal) = take_anchors(&mut fields, names)?;
        Ok(Self {
            account,
            device,
            anchors,
            personal,
            unknown: fields.unknown(),
        })
    }
}

/// An anchors or link body's anchors and personal space.
type Starts = (Vec<(SpaceId, [u8; 32])>, Option<SpaceId>);

/// Ends `map`, an anchors or link body, with its `anchors` and `personal` space, keys 3 and 4.
pub(crate) fn finish_anchors(
    map: MapOut,
    anchors: &[(SpaceId, [u8; 32])],
    personal: Option<&SpaceId>,
) -> Vec<u8> {
    map.put(3, encode_anchors(anchors))
        .put_some(4, personal.map(cbor::id))
        .finish()
}

/// The anchors and personal space, keys 3 and 4, of an anchors or link body's `fields`, by their
/// `names` as fields.
pub(crate) fn take_anchors(
    fields: &mut Fields<'_, '_>,
    [anchors, personal]: [&'static str; 2],
) -> Result<Starts, DecodeError> {
    let starts = decode_anchors(fields.required(3, anchors)?, anchors)?;
    let space = (fields.take(4).map(|item| item.id(personal))).transpose()?;
    Ok((starts, space))
}

/// The anchors of an anchors or link body: `[space, revision]` each, in order.
fn encode_anchors(anchors: &[(SpaceId, [u8; 32])]) -> Vec<u8> {
    cbor::array(
        anchors
            .iter()
            .map(|(space, revision)| cbor::array([cbor::id(space), cbor::bytes(revision)])),
    )
}

/// The anchors `item` holds as `field` of an anchors or link body.
fn decode_anchors(
    item: &Item<'_>,
    field: &'static str,
) -> Result<Vec<(SpaceId, [u8; 32])>, DecodeError> {
    item.array(field)?
        .iter()
        .map(|anchor| {
            let [space, revision] = anchor.tuple(field)?;
            Ok((space.id(field)?, revision.fixed(field)?))
        })
        .collect()
}

cbor::cbor!(AnchorsBody);
