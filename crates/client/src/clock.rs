//! The clock by which this device counts a channel epoch's age (`docs/protocol/membership.md`,
//! "History key"): the system's, moved on by an offset that tests set. Signed timestamps, proofs
//! and whatever the server checks keep the system's clock ([`crate::spaces::now`]).

use std::sync::OnceLock;

/// Seconds added to the system's clock; set once, at the start of a run.
static OFFSET: OnceLock<u64> = OnceLock::new();

/// Moves the clock of epochs' ages on by `seconds` for the rest of the run.
pub fn set_offset(seconds: u64) {
    let _ = OFFSET.set(seconds);
}

/// Seconds since the Unix epoch, by which the device counts an epoch's age.
pub fn epoch_now() -> u64 {
    crate::spaces::now().saturating_add(OFFSET.get().copied().unwrap_or(0))
}
