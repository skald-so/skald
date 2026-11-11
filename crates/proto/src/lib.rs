//! Skald wire protocol types, as the protocol spec in `docs/protocol/` defines them.

mod address;
mod cbor;
mod content;
mod envelope;
mod id;
mod receipt;
mod version;

pub use address::{
    Address, AddressError, Name, NameError, PERSON, Segment, ServerError, ServerName,
};
pub use cbor::{Cbor, DecodeError, EncodingProblem, FieldProblem, MAX_DEPTH, NonEmpty, Unknown};
pub use content::{Attachment, Body, Content, Grant, GrantBody};
pub use envelope::{Envelope, Header, HpkePayload, HpkePlaintext, Scheme};
pub use id::{
    AccountId, BlobId, ChannelId, ClaimId, DeviceId, Id, IdError, Issued, Kind, MessageId, QueueId,
    SpaceId, TaskId, kind,
};
pub use receipt::{DeliveryStatus, Receipt, ReceiptStatus, SendKey, ServerStatus, Stored};
pub use version::{
    LATEST_HEADER, PROTOCOL_VERSION, SUBPROTOCOL_PREFIX, VERSION_HEADER, VersionProblem,
    VersionRange, describe_versions, parse_version, subprotocol,
};
