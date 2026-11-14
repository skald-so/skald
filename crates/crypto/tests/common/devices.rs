//! The tests' devices by seed, their keys as they come from key storage, and the caller's time.

use skald_crypto::{SigningKey, StateKey};
use skald_proto::DeviceId;

/// The caller's time, in seconds since the Unix epoch, far from the machine's clock.
pub const NOW: u64 = 1_800_000_000;

/// The signing key of device `seed`.
pub fn signing_key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

/// The ID of device `seed`, derived from its signing key.
pub fn device_id(seed: u8) -> DeviceId {
    DeviceId::from_signing_key(&signing_key(seed).public_key())
}

/// The state key of device `seed`.
pub fn state_key(seed: u8) -> StateKey {
    StateKey::from_bytes(&[seed; 32])
}
