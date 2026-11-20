//! Presence: `docs/protocol/agents.md`, "Presence".

use crate::address::{Address, Name};
use crate::cbor::{self, Codec, DecodeError, FieldProblem, Item, MapOut, Unknown, field_error};
use crate::history::at_least;
use crate::id::{ChannelId, DeviceId, QueueId};
use crate::signed::{Signed, SignedBody};

/// The label of a presence plaintext's signature, and of its associated data.
pub const PRESENCE_LABEL: &str = "skald presence";

/// The longest `note` of an [`AgentStatus`], in bytes.
pub const NOTE_MAX: usize = 128;

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

impl Presence {
    /// The associated data of a value: the encoded `["skald presence", device, channel, epoch]`.
    pub fn associated_data(device: DeviceId, channel: ChannelId, epoch: u64) -> Vec<u8> {
        cbor::array([
            cbor::text(PRESENCE_LABEL),
            cbor::id(&device),
            cbor::id(&channel),
            cbor::uint(epoch),
        ])
    }
}

/// What a [`Presence`] encrypts: its body signed by `device` over `["skald presence", body]`.
pub type PresencePlaintext = Signed<PresenceBody>;

/// The body of a [`PresencePlaintext`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresenceBody {
    pub device: DeviceId,
    pub channel: ChannelId,
    /// At least 1.
    pub epoch: u64,
    /// One more than the device's previous value in the channel; at least 1.
    pub revision: u64,
    /// Seconds since the Unix epoch.
    pub expires: u64,
    /// The device's agents in the channel.
    pub agents: Vec<AgentStatus>,
    /// The channel's queues the device takes tasks from.
    pub serves: Vec<ServedQueue>,
    pub unknown: Unknown,
}

impl SignedBody for PresenceBody {
    const LABEL: &'static str = PRESENCE_LABEL;
    const FIELDS: [&'static str; 3] = [
        "presence-plaintext",
        "presence-plaintext.body",
        "presence-plaintext.signature",
    ];
}

/// An agent's state in a [`PresenceBody`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentStatus {
    pub agent: Address,
    pub state: AgentState,
    /// When the agent entered `state`, in seconds since the Unix epoch.
    pub since: u64,
    /// What it works on; 1 to [`NOTE_MAX`] bytes.
    pub note: Option<String>,
    /// The agent CLI, such as `claude-code`.
    pub runtime: Option<Name>,
    pub unknown: Unknown,
}

/// What an agent does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgentState {
    /// Takes messages.
    Available,
    /// Works on a turn or a task.
    Busy,
    /// Waits for its person.
    Waiting,
    /// Has no running session.
    Offline,
}

/// A queue a device takes tasks from, with its tier per sender trust level.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServedQueue {
    pub queue: QueueId,
    pub tiers: Tiers,
}

/// A queue's tier for senders of each trust level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Tiers {
    pub own: Tier,
    pub peer: Tier,
    pub service: Tier,
}

/// What a device does with a queue's tasks from senders of one trust level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tier {
    Off,
    ReadOnly,
    Approve,
    Full,
}

/// The fields a value and a body begin with, `device`, `channel` and `epoch`, and the `unknown`
/// ones.
fn head(unknown: &Unknown, device: &DeviceId, channel: &ChannelId, epoch: u64) -> MapOut {
    MapOut::new(unknown)
        .put(1, cbor::id(device))
        .put(2, cbor::id(channel))
        .put(3, cbor::uint(epoch))
}

impl Codec for Presence {
    fn encode(&self) -> Vec<u8> {
        head(&self.unknown, &self.device, &self.channel, self.epoch)
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

impl Codec for PresenceBody {
    fn encode(&self) -> Vec<u8> {
        head(&self.unknown, &self.device, &self.channel, self.epoch)
            .put(4, cbor::uint(self.revision))
            .put(5, cbor::uint(self.expires))
            .put(6, cbor::array(self.agents.iter().map(Codec::encode)))
            .put(7, cbor::array(self.serves.iter().map(Codec::encode)))
            .finish()
    }

    fn decode(item: &Item<'_>) -> Result<Self, DecodeError> {
        let mut fields = item.map("presence-body")?;
        Ok(Self {
            device: fields
                .required(1, "presence-body.device")?
                .id("presence-body.device")?,
            channel: fields
                .required(2, "presence-body.channel")?
                .id("presence-body.channel")?,
            epoch: at_least(
                fields.required(3, "presence-body.epoch")?,
                "presence-body.epoch",
                1,
            )?,
            revision: at_least(
                fields.required(4, "presence-body.revision")?,
                "presence-body.revision",
                1,
            )?,
            expires: fields
                .required(5, "presence-body.expires")?
                .uint("presence-body.expires")?,
            agents: fields
                .required(6, "presence-body.agents")?
                .array("presence-body.agents")?
                .iter()
                .map(AgentStatus::decode)
                .collect::<Result<_, _>>()?,
            serves: fields
                .required(7, "presence-body.serves")?
                .array("presence-body.serves")?
                .iter()
                .map(ServedQueue::decode)
                .collect::<Result<_, _>>()?,
            unknown: fields.unknown(),
        })
    }
}

impl Codec for AgentStatus {
    fn encode(&self) -> Vec<u8> {
        MapOut::new(&self.unknown)
            .put(1, cbor::text(&self.agent.to_string()))
            .put(2, self.state.encode())
            .put(3, cbor::uint(self.since))
            .put_some(4, self.note.as_deref().map(cbor::text))
            .put_some(
                5,
                self.runtime.as_ref().map(|name| cbor::text(name.as_str())),
            )
            .finish()
    }

    fn decode(item: &Item<'_>) -> Result<Self, DecodeError> {
        let mut fields = item.map("agent-status")?;
        Ok(Self {
            agent: fields
                .required(1, "agent-status.agent")?
                .address("agent-status.agent")?,
            state: AgentState::decode(fields.required(2, "agent-status.state")?)?,
            since: fields
                .required(3, "agent-status.since")?
                .uint("agent-status.since")?,
            note: fields.take(4).map(note).transpose()?,
            runtime: fields
                .take(5)
                .map(|item| item.name("agent-status.runtime"))
                .transpose()?,
            unknown: fields.unknown(),
        })
    }
}

/// An agent's `note`: `tstr .size (1..128)`.
fn note(item: &Item<'_>) -> Result<String, DecodeError> {
    const FIELD: &str = "agent-status.note";
    let note = item.text(FIELD)?;
    if (1..=NOTE_MAX).contains(&note.len()) {
        Ok(note.to_owned())
    } else {
        Err(field_error(
            FIELD,
            FieldProblem::Size {
                min: 1,
                max: NOTE_MAX,
                actual: note.len(),
            },
        ))
    }
}

impl Codec for AgentState {
    fn encode(&self) -> Vec<u8> {
        cbor::uint(match self {
            Self::Available => 1,
            Self::Busy => 2,
            Self::Waiting => 3,
            Self::Offline => 4,
        })
    }

    fn decode(item: &Item<'_>) -> Result<Self, DecodeError> {
        const FIELD: &str = "agent-status.state";
        match item.uint(FIELD)? {
            1 => Ok(Self::Available),
            2 => Ok(Self::Busy),
            3 => Ok(Self::Waiting),
            4 => Ok(Self::Offline),
            other => Err(field_error(FIELD, FieldProblem::Value(other))),
        }
    }
}

impl Codec for ServedQueue {
    fn encode(&self) -> Vec<u8> {
        let Tiers { own, peer, service } = self.tiers;
        cbor::array([
            cbor::id(&self.queue),
            cbor::array([own.encode(), peer.encode(), service.encode()]),
        ])
    }

    fn decode(item: &Item<'_>) -> Result<Self, DecodeError> {
        let [queue, tiers] = item.tuple("served-queue")?;
        let [own, peer, service] = tiers.tuple("served-queue.tiers")?;
        Ok(Self {
            queue: queue.id("served-queue.queue")?,
            tiers: Tiers {
                own: Tier::decode(own)?,
                peer: Tier::decode(peer)?,
                service: Tier::decode(service)?,
            },
        })
    }
}

impl Codec for Tier {
    fn encode(&self) -> Vec<u8> {
        cbor::uint(match self {
            Self::Off => 1,
            Self::ReadOnly => 2,
            Self::Approve => 3,
            Self::Full => 4,
        })
    }

    fn decode(item: &Item<'_>) -> Result<Self, DecodeError> {
        const FIELD: &str = "served-queue.tier";
        match item.uint(FIELD)? {
            1 => Ok(Self::Off),
            2 => Ok(Self::ReadOnly),
            3 => Ok(Self::Approve),
            4 => Ok(Self::Full),
            other => Err(field_error(FIELD, FieldProblem::Value(other))),
        }
    }
}

cbor::cbor!(Presence, PresenceBody, AgentStatus, ServedQueue);

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
