//! A space's roster and policy: `docs/protocol/membership.md`, "Roster" and "Policy".

use sha2::{Digest, Sha256};

use crate::address::Name;
use crate::cbor::{self, Codec, DecodeError, FieldProblem, Item, MapOut, NonEmpty, Unknown};
use crate::id::{AccountId, ChannelId, DeviceId, InviteId, SpaceId};
use crate::signed::{Signed, SignedBody};

/// The label of a roster revision's signature.
pub const ROSTER_LABEL: &str = "skald roster";
/// The shortest `history` of a policy: a day, in seconds.
pub const HISTORY_MIN: u64 = 86_400;
/// The `history` of a new space's policy: 30 days, in seconds.
pub const NEW_SPACE_HISTORY: u64 = 30 * HISTORY_MIN;

/// A revision of a space's roster, signed by an admin's device over `["skald roster", body]`.
pub type Roster = Signed<RosterBody>;

impl Roster {
    /// The revision's hash: SHA-256 over the encoded roster, as `prev` of the next revision holds
    /// it.
    pub fn hash(&self) -> [u8; 32] {
        Sha256::digest(cbor::Cbor::to_cbor(self)).into()
    }
}

/// What a [`Roster`] signs: a full snapshot of the space's members, service agents, channels and
/// policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RosterBody {
    pub space: SpaceId,
    /// The space in addresses.
    pub name: Name,
    /// At least 1.
    pub revision: u64,
    /// SHA-256 of the previous revision's roster; `None` in revision 1.
    pub prev: Option<[u8; 32]>,
    /// An admin's device.
    pub signer: DeviceId,
    /// Seconds since the Unix epoch, for display.
    pub timestamp: u64,
    pub members: NonEmpty<Member>,
    pub service_agents: Vec<ServiceAgent>,
    pub channels: Vec<Channel>,
    pub policy: Policy,
    pub unknown: Unknown,
}

/// A member of a space.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    pub account: AccountId,
    /// The owner in its agents' addresses.
    pub handle: Name,
    pub role: Role,
    /// The account's master public key.
    pub master: [u8; 32],
    /// The invite it joined with.
    pub invite: Option<InviteId>,
    /// Its devices an admin barred from the space.
    pub excluded: Option<NonEmpty<DeviceId>>,
    pub unknown: Unknown,
}

/// A member's role in a space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    Admin = 1,
    Member = 2,
}

/// A `service` device of the space, sending as `space/owner/agent`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceAgent {
    pub device: DeviceId,
    /// A group of service agents, such as `ci`.
    pub owner: Name,
    /// Such as `deploy-bot`.
    pub agent: Name,
    pub unknown: Unknown,
}

/// A channel of a space.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Channel {
    pub id: ChannelId,
    pub members: Vec<ChannelMember>,
    /// The `service` devices in its MLS group.
    pub services: Vec<DeviceId>,
    pub unknown: Unknown,
}

/// A member's account in a channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelMember {
    pub account: AccountId,
    /// Whether it reads the history from before it joined.
    pub history: bool,
}

/// A space's policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    pub ceilings: Ceilings,
    /// The history key's longest age in seconds, at least [`HISTORY_MIN`].
    pub history: u64,
    pub unknown: Unknown,
}

/// The most rights a task gets without the approval of the claiming device's user, per trust
/// level of its sender.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ceilings {
    pub member: Mode,
    pub service: Mode,
}

/// The rights of a task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mode {
    /// Reads only the agent's folder and changes nothing, with no network.
    ReadOnly,
    /// No limits.
    Full,
    /// A mode of a later version, kept so that the roster encodes as it was decoded.
    Later(u64),
}

impl Mode {
    /// The mode this version applies: a mode of a later version is `ReadOnly`.
    pub fn effective(self) -> Self {
        match self {
            Self::Later(_) => Self::ReadOnly,
            known => known,
        }
    }

    fn value(self) -> u64 {
        match self {
            Self::ReadOnly => 1,
            Self::Full => 2,
            Self::Later(value) => value,
        }
    }

    fn from_value(value: u64) -> Self {
        match value {
            1 => Self::ReadOnly,
            2 => Self::Full,
            other => Self::Later(other),
        }
    }
}

impl Policy {
    /// A new space's policy: `read-only` for both ceilings and a history of 30 days.
    pub fn new_space() -> Self {
        Self {
            ceilings: Ceilings {
                member: Mode::ReadOnly,
                service: Mode::ReadOnly,
            },
            history: NEW_SPACE_HISTORY,
            unknown: Unknown::default(),
        }
    }
}

impl RosterBody {
    /// Revision 1 of a new space, which its creator signs: the creator's `account` the only member
    /// and admin, with its `handle` and `master` key, no service agents or channels, and a new
    /// space's policy.
    pub fn first(
        space: SpaceId,
        name: Name,
        account: AccountId,
        handle: Name,
        master: [u8; 32],
        signer: DeviceId,
        timestamp: u64,
    ) -> Self {
        Self {
            space,
            name,
            revision: 1,
            prev: None,
            signer,
            timestamp,
            members: NonEmpty::new(Member {
                account,
                handle,
                role: Role::Admin,
                master,
                invite: None,
                excluded: None,
                unknown: Unknown::default(),
            }),
            service_agents: Vec::new(),
            channels: Vec::new(),
            policy: Policy::new_space(),
            unknown: Unknown::default(),
        }
    }

    /// The member of `account`, the first if the revision lists it more than once.
    pub fn member(&self, account: AccountId) -> Option<&Member> {
        self.members.iter().find(|member| member.account == account)
    }
}

impl SignedBody for RosterBody {
    const LABEL: &'static str = ROSTER_LABEL;
    const FIELDS: [&'static str; 3] = ["roster", "roster.body", "roster.signature"];
}

/// The items of `item`, an array, each decoded by `decode`, for CDDL `[* T]`.
fn list<'b, T>(
    item: &Item<'b>,
    field: &'static str,
    decode: impl Fn(&Item<'b>) -> Result<T, DecodeError>,
) -> Result<Vec<T>, DecodeError> {
    item.array(field)?.iter().map(decode).collect()
}

fn encode_list<T>(items: &[T], encode: impl Fn(&T) -> Vec<u8>) -> Vec<u8> {
    cbor::array(items.iter().map(encode))
}

impl Codec for RosterBody {
    fn encode(&self) -> Vec<u8> {
        MapOut::new(&self.unknown)
            .put(1, cbor::id(&self.space))
            .put(2, cbor::text(self.name.as_str()))
            .put(3, cbor::uint(self.revision))
            .put_some(4, self.prev.as_ref().map(|prev| cbor::bytes(prev)))
            .put(5, cbor::id(&self.signer))
            .put(6, cbor::uint(self.timestamp))
            .put(7, encode_list(&self.members, Codec::encode))
            .put(8, encode_list(&self.service_agents, Codec::encode))
            .put(9, encode_list(&self.channels, Codec::encode))
            .put(10, self.policy.encode())
            .finish()
    }

    fn decode(item: &Item<'_>) -> Result<Self, DecodeError> {
        let mut fields = item.map("roster-body")?;
        Ok(Self {
            space: fields
                .required(1, "roster-body.space")?
                .id("roster-body.space")?,
            name: fields
                .required(2, "roster-body.name")?
                .name("roster-body.name")?,
            revision: fields
                .required(3, "roster-body.revision")?
                .positive("roster-body.revision")?
                .get(),
            prev: fields
                .take(4)
                .map(|item| item.fixed("roster-body.prev"))
                .transpose()?,
            signer: fields
                .required(5, "roster-body.signer")?
                .id("roster-body.signer")?,
            timestamp: fields
                .required(6, "roster-body.timestamp")?
                .uint("roster-body.timestamp")?,
            members: fields
                .required(7, "roster-body.members")?
                .non_empty("roster-body.members", Member::decode)?,
            service_agents: list(
                fields.required(8, "roster-body.service-agents")?,
                "roster-body.service-agents",
                ServiceAgent::decode,
            )?,
            channels: list(
                fields.required(9, "roster-body.channels")?,
                "roster-body.channels",
                Channel::decode,
            )?,
            policy: Policy::decode(fields.required(10, "roster-body.policy")?)?,
            unknown: fields.unknown(),
        })
    }
}

impl Codec for Member {
    fn encode(&self) -> Vec<u8> {
        MapOut::new(&self.unknown)
            .put(1, cbor::id(&self.account))
            .put(2, cbor::text(self.handle.as_str()))
            .put(3, cbor::uint(self.role as u64))
            .put(4, cbor::bytes(&self.master))
            .put_some(5, self.invite.as_ref().map(cbor::id))
            .put_some(
                6,
                self.excluded
                    .as_ref()
                    .map(|excluded| encode_list(excluded, cbor::id)),
            )
            .finish()
    }

    fn decode(item: &Item<'_>) -> Result<Self, DecodeError> {
        let mut fields = item.map("member")?;
        Ok(Self {
            account: fields.required(1, "member.account")?.id("member.account")?,
            handle: fields.required(2, "member.handle")?.name("member.handle")?,
            role: match fields.required(3, "member.role")?.uint("member.role")? {
                1 => Role::Admin,
                2 => Role::Member,
                other => {
                    return Err(cbor::field_error("member.role", FieldProblem::Value(other)));
                }
            },
            master: fields
                .required(4, "member.master")?
                .fixed("member.master")?,
            invite: fields
                .take(5)
                .map(|item| item.id("member.invite"))
                .transpose()?,
            excluded: fields
                .take(6)
                .map(|item| item.non_empty("member.excluded", |id| id.id("member.excluded")))
                .transpose()?,
            unknown: fields.unknown(),
        })
    }
}

impl Codec for ServiceAgent {
    fn encode(&self) -> Vec<u8> {
        MapOut::new(&self.unknown)
            .put(1, cbor::id(&self.device))
            .put(2, cbor::text(self.owner.as_str()))
            .put(3, cbor::text(self.agent.as_str()))
            .finish()
    }

    fn decode(item: &Item<'_>) -> Result<Self, DecodeError> {
        let mut fields = item.map("service-agent")?;
        Ok(Self {
            device: fields
                .required(1, "service-agent.device")?
                .id("service-agent.device")?,
            owner: fields
                .required(2, "service-agent.owner")?
                .name("service-agent.owner")?,
            agent: fields
                .required(3, "service-agent.agent")?
                .name("service-agent.agent")?,
            unknown: fields.unknown(),
        })
    }
}

impl Codec for Channel {
    fn encode(&self) -> Vec<u8> {
        MapOut::new(&self.unknown)
            .put(1, cbor::id(&self.id))
            .put(2, encode_list(&self.members, Codec::encode))
            .put(3, encode_list(&self.services, cbor::id))
            .finish()
    }

    fn decode(item: &Item<'_>) -> Result<Self, DecodeError> {
        let mut fields = item.map("channel")?;
        Ok(Self {
            id: fields
                .required(1, "channel.channel")?
                .id("channel.channel")?,
            members: list(
                fields.required(2, "channel.members")?,
                "channel.members",
                ChannelMember::decode,
            )?,
            services: list(
                fields.required(3, "channel.services")?,
                "channel.services",
                |id| id.id("channel.services"),
            )?,
            unknown: fields.unknown(),
        })
    }
}

impl Codec for ChannelMember {
    fn encode(&self) -> Vec<u8> {
        cbor::array([cbor::id(&self.account), cbor::boolean(self.history)])
    }

    fn decode(item: &Item<'_>) -> Result<Self, DecodeError> {
        let [account, history] = item.tuple("channel-member")?;
        Ok(Self {
            account: account.id("channel-member.account")?,
            history: history.boolean("channel-member.history")?,
        })
    }
}

impl Codec for Policy {
    fn encode(&self) -> Vec<u8> {
        let ceilings = [self.ceilings.member, self.ceilings.service];
        MapOut::new(&self.unknown)
            .put(
                1,
                cbor::array(ceilings.map(|mode| cbor::uint(mode.value()))),
            )
            .put(2, cbor::uint(self.history))
            .finish()
    }

    fn decode(item: &Item<'_>) -> Result<Self, DecodeError> {
        let mut fields = item.map("policy")?;
        let [member, service] = fields
            .required(1, "policy.ceilings")?
            .tuple("policy.ceilings")?;
        let mode = |item: &Item<'_>| item.uint("policy.ceilings").map(Mode::from_value);
        let history = fields
            .required(2, "policy.history")?
            .uint("policy.history")?;
        if history < HISTORY_MIN {
            return Err(cbor::field_error(
                "policy.history",
                FieldProblem::Below {
                    min: HISTORY_MIN,
                    actual: history,
                },
            ));
        }
        Ok(Self {
            ceilings: Ceilings {
                member: mode(member)?,
                service: mode(service)?,
            },
            history,
            unknown: fields.unknown(),
        })
    }
}

cbor::cbor!(RosterBody);
