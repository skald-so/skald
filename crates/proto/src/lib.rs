//! Skald wire protocol types, as the protocol spec in `docs/protocol/` defines them.

mod address;
mod id;

pub use address::{
    Address, AddressError, Name, NameError, PERSON, Segment, ServerError, ServerName,
};
pub use id::{
    AccountId, ChannelId, ClaimId, DeviceId, Id, IdError, Issued, Kind, MessageId, QueueId,
    SpaceId, TaskId, kind,
};
