//! IDs generated on every target `proto` builds for, natively and in WASM: UUIDv7 of the current
//! time, read in WASM from JavaScript's clock, with random bits from the cryptographically secure
//! generator.

use std::collections::HashSet;

use skald_proto::{ChannelId, MessageId};

/// The current Unix time in milliseconds: the system clock natively, JavaScript's clock in WASM.
#[cfg(not(target_arch = "wasm32"))]
fn now_ms() -> u64 {
    let since = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap();
    u64::try_from(since.as_millis()).unwrap()
}

/// The current Unix time in milliseconds: the system clock natively, JavaScript's clock in WASM.
#[cfg(target_arch = "wasm32")]
fn now_ms() -> u64 {
    // Whole milliseconds since the Unix epoch, well below 2^53.
    js_sys::Date::now() as u64
}

/// The 48-bit Unix time in milliseconds of a UUIDv7.
fn time_of(bytes: &[u8; 16]) -> u64 {
    let mut time = [0; 8];
    time[2..].copy_from_slice(&bytes[..6]);
    u64::from_be_bytes(time)
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn generated_ids_are_uuidv7_of_the_current_time() {
    let before = now_ms();
    let messages: Vec<MessageId> = (0..32).map(|_| MessageId::generate()).collect();
    let channel = ChannelId::generate();
    let after = now_ms();
    // A time after 2025-01-01: the clock is read, not left at the Unix epoch.
    assert!(before > 1_735_689_600_000, "{before}");
    let ids = messages
        .iter()
        .map(MessageId::as_bytes)
        .chain([channel.as_bytes()]);
    for bytes in ids {
        assert_eq!(bytes[6] >> 4, 7, "version of {bytes:02x?}");
        assert_eq!(bytes[8] >> 6, 0b10, "variant of {bytes:02x?}");
        let time = time_of(bytes);
        assert!(
            before <= time && time <= after,
            "{bytes:02x?}: {time} not in {before}..={after}"
        );
    }
    // Random bits: the IDs differ, and so does each random byte across them.
    assert_eq!(
        messages.iter().collect::<HashSet<_>>().len(),
        messages.len()
    );
    for at in 6..16 {
        let values: HashSet<u8> = messages.iter().map(|id| id.as_bytes()[at]).collect();
        assert!(values.len() > 1, "byte {at} is the same in every ID");
    }
}
