//! Skald end-to-end encryption core.

mod cert;
mod hpke;
mod roster;
mod sign;

pub use cert::{ChainProblem, DeviceCheckError, check_device, issue};
pub use hpke::HpkeKey;
pub use roster::{RosterError, accept_next, check_roster, sign_roster};
pub use sign::{SignatureError, SigningKey, verify};
