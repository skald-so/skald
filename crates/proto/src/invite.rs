//! Invites and acceptances: `docs/protocol/membership.md`, "Invite".

use crate::address::Name;
use crate::cbor::{self, Codec, DecodeError, Item, MapOut, Unknown};
use crate::id::{AccountId, ChannelId, DeviceId, InviteId, SpaceId};
use crate::roster::Role;
use crate::signed::{Signed, SignedBody};

/// The label of an invite's signature.
pub const INVITE_LABEL: &str = "skald invite";
/// The label of an acceptance's signature.
pub const ACCEPTANCE_LABEL: &str = "skald acceptance";

/// An invite, signed by its `signer` device over `["skald invite", body]`.
pub type Invite = Signed<InviteBody>;

/// What an [`Invite`] signs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InviteBody {
    /// Assigned by the server.
    pub invite: InviteId,
    pub space: SpaceId,
    /// The invite key: the Ed25519 public key derived from the secret.
    pub key: [u8; 32],
    pub role: Role,
    /// Each channel to add the newcomer to and whether it reads the channel's earlier history, in
    /// the body's order.
    pub channels: Vec<(ChannelId, bool)>,
    /// An admin's device.
    pub signer: DeviceId,
    /// Seconds since the Unix epoch.
    pub expires: u64,
    pub unknown: Unknown,
}

impl SignedBody for InviteBody {
    const LABEL: &'static str = INVITE_LABEL;
    const FIELDS: [&'static str; 3] = ["invite", "invite.body", "invite.signature"];
}

impl Codec for InviteBody {
    fn encode(&self) -> Vec<u8> {
        let channels = self
            .channels
            .iter()
            .map(|(channel, history)| cbor::array([cbor::id(channel), cbor::boolean(*history)]));
        MapOut::new(&self.unknown)
            .put(1, cbor::id(&self.invite))
            .put(2, cbor::id(&self.space))
            .put(3, cbor::bytes(&self.key))
            .put(4, cbor::uint(self.role as u64))
            .put(5, cbor::array(channels))
            .put(6, cbor::id(&self.signer))
            .put(7, cbor::uint(self.expires))
            .finish()
    }

    fn decode(item: &Item<'_>) -> Result<Self, DecodeError> {
        let mut fields = item.map("invite-body")?;
        let invite = fields
            .required(1, "invite-body.invite")?
            .id("invite-body.invite")?;
        let space = fields
            .required(2, "invite-body.space")?
            .id("invite-body.space")?;
        let key = fields
            .required(3, "invite-body.key")?
            .fixed("invite-body.key")?;
        let role = Role::decode(fields.required(4, "invite-body.role")?, "invite-body.role")?;
        let channels = fields
            .required(5, "invite-body.channels")?
            .array("invite-body.channels")?
            .iter()
            .map(|entry| {
                let [channel, history] = entry.tuple("invite-body.channels")?;
                Ok((
                    channel.id("invite-body.channels")?,
                    history.boolean("invite-body.channels")?,
                ))
            })
            .collect::<Result<_, DecodeError>>()?;
        let signer = fields
            .required(6, "invite-body.signer")?
            .id("invite-body.signer")?;
        let expires = fields
            .required(7, "invite-body.expires")?
            .uint("invite-body.expires")?;
        Ok(Self {
            invite,
            space,
            key,
            role,
            channels,
            signer,
            expires,
            unknown: fields.unknown(),
        })
    }
}

cbor::cbor!(InviteBody);

/// An acceptance, signed by the invite key over `["skald acceptance", body]`.
pub type Acceptance = Signed<AcceptanceBody>;

/// What an [`Acceptance`] signs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptanceBody {
    pub invite: InviteId,
    /// The newcomer's account.
    pub account: AccountId,
    /// The newcomer's master public key.
    pub master: [u8; 32],
    /// The handle the newcomer asks for.
    pub handle: Name,
    pub unknown: Unknown,
}

impl SignedBody for AcceptanceBody {
    const LABEL: &'static str = ACCEPTANCE_LABEL;
    const FIELDS: [&'static str; 3] = ["acceptance", "acceptance.body", "acceptance.signature"];
}

impl Codec for AcceptanceBody {
    fn encode(&self) -> Vec<u8> {
        MapOut::new(&self.unknown)
            .put(1, cbor::id(&self.invite))
            .put(2, cbor::id(&self.account))
            .put(3, cbor::bytes(&self.master))
            .put(4, cbor::text(self.handle.as_str()))
            .finish()
    }

    fn decode(item: &Item<'_>) -> Result<Self, DecodeError> {
        let mut fields = item.map("acceptance-body")?;
        Ok(Self {
            invite: fields
                .required(1, "acceptance-body.invite")?
                .id("acceptance-body.invite")?,
            account: fields
                .required(2, "acceptance-body.account")?
                .id("acceptance-body.account")?,
            master: fields
                .required(3, "acceptance-body.master")?
                .fixed("acceptance-body.master")?,
            handle: fields
                .required(4, "acceptance-body.handle")?
                .name("acceptance-body.handle")?,
            unknown: fields.unknown(),
        })
    }
}

cbor::cbor!(AcceptanceBody);
