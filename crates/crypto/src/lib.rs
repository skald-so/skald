//! Skald end-to-end encryption core.

mod cert;
mod history;
mod hpke;
mod mls;
mod roster;
mod sealed;
mod sign;
mod wrap;

pub use cert::{ChainProblem, DeviceCheckError, check_device, issue};
pub use history::{HistoryError, HistoryKey, HistoryMessage, follow_links};
pub use hpke::{HpkeError, HpkeKey, HpkeMessage, open_message, seal_message};
pub use mls::{
    ChannelView, Decrypted, GroupChange, Joining, KeyPackage, MemberDevice, MlsDevice, MlsError,
    Processed,
};
pub use roster::{RosterError, accept_next, check_roster, sign_roster};
pub use sealed::{SealedState, StateKey};
pub use sign::{SignatureError, SigningKey, verify};
pub use wrap::{OpenedWrap, open_wrap};
