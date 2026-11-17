//! The clock by which this device counts a channel epoch's age (`docs/protocol/membership.md`,
//! "History key"): the system's, moved on by an offset that tests set. Signed timestamps, proofs
//! and whatever the server checks keep the system's clock ([`crate::spaces::now`]).

use std::sync::atomic::{AtomicU64, Ordering};

/// Seconds added to the system's clock; set at the start of each operation.
static OFFSET: AtomicU64 = AtomicU64::new(0);

/// Moves the clock of epochs' ages on by `seconds` for the rest of the operation.
pub fn set_offset(seconds: u64) {
    OFFSET.store(seconds, Ordering::Relaxed);
}

/// Seconds since the Unix epoch, by which the device counts an epoch's age.
pub fn epoch_now() -> u64 {
    crate::spaces::now().saturating_add(OFFSET.load(Ordering::Relaxed))
}
