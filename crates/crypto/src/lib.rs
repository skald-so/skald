//! Skald end-to-end encryption core.

mod cert;
mod hpke;
mod mls;
mod roster;
mod sealed;
mod sign;

pub use cert::{ChainProblem, DeviceCheckError, check_device, issue};
pub use hpke::{HpkeError, HpkeKey, HpkeMessage, open_message, seal_message};
pub use mls::{
    ChannelView, Decrypted, GroupChange, HistoryKey, Joining, KeyPackage, MemberDevice, MlsDevice,
    MlsError, Processed,
};
pub use roster::{RosterError, accept_next, check_roster, sign_roster};
pub use sealed::{SealedState, StateKey};
pub use sign::{SignatureError, SigningKey, verify};
