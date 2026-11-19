//! Skald wire protocol types, as the protocol spec in `docs/protocol/` defines them.

mod account;
mod address;
mod anchors;
mod cbor;
mod cert;
mod content;
mod envelope;
mod frame;
mod history;
mod id;
mod link;
mod receipt;
mod revocation;
mod roster;
mod service_key;
mod signed;
mod trust;
mod version;

pub use account::{ACCOUNT_LABEL, AccountBody, AccountRecord, MasterBackup};
pub use address::{
    Address, AddressError, Name, NameError, PERSON, Segment, ServerError, ServerName,
};
pub use anchors::{ANCHORS_LABEL, Anchors, AnchorsBody};
pub use cbor::{
    Cbor, DecodeError, EncodingProblem, FieldProblem, MAX_DEPTH, NonEmpty, Unknown, signed_data,
};
pub use cert::{CERT_LABEL, CERT_NAME_MAX, Cert, CertBody, DeviceKind, PROOF_LABEL};
pub use content::{
    Attachment, Body, Content, Delegation, DelegationBody, GRANT_LABEL, Grant, GrantBody,
};
pub use envelope::{Envelope, Header, HpkePayload, HpkePlaintext, Scheme};
pub use frame::{
    AUTH_LABEL, Frame, HistoryItem, PROOF_HEADER, Push, REQUEST_LABEL, RawEnvelope, RequestProof,
    UnknownPush,
};
pub use history::{
    HISTORY_LABEL, HistoryBody, HistoryEntry, HistoryLink, HistoryPlaintext, HistoryWrap,
    HistoryWrapBody, WrapRecipient,
};
pub use id::{
    AccountId, BlobId, ChannelId, ClaimId, DeviceId, Id, IdError, InviteId, Issued, Kind,
    MessageId, QueueId, SpaceId, TaskId, kind,
};
pub use link::LinkMessage;
pub use receipt::{DeliveryStatus, Receipt, ReceiptStatus, SendKey, ServerStatus, Stored};
pub use revocation::{REVOCATION_LABEL, Revocation, RevocationBody, Revocations};
pub use roster::{
    Ceilings, Channel, ChannelMember, HISTORY_MIN, Member, Mode, NEW_SPACE_HISTORY, Policy,
    ROSTER_LABEL, Role, Roster, RosterBody, ServiceAgent,
};
pub use service_key::ServiceKey;
pub use signed::{Signed, SignedBody};
pub use trust::{TrustLevel, TrustLevelError};
pub use version::{
    LATEST_HEADER, PROTOCOL_VERSION, SUBPROTOCOL_PREFIX, VERSION_HEADER, VersionProblem,
    VersionRange, describe_versions, parse_version, subprotocol,
};
