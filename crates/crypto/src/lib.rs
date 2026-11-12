//! Skald end-to-end encryption core.

mod cert;
mod sign;

pub use cert::{ChainProblem, DeviceCheckError, check_device, issue};
pub use sign::{SignatureError, SigningKey, verify};
