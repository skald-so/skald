//! Skald end-to-end encryption core.

mod cert;
mod hpke;
mod mls;
mod roster;
mod sealed;
mod sign;

pub use cert::{ChainProblem, DeviceCheckError, check_device, issue};
pub use hpke::HpkeKey;
pub use mls::{Decrypted, HistoryKey, Joining, KeyPackage, MemberDevice, MlsDevice, MlsError};
pub use roster::{RosterError, accept_next, check_roster, sign_roster};
pub use sealed::{SealedState, StateKey};
pub use sign::{SignatureError, SigningKey, verify};
