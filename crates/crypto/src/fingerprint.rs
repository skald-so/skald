//! Account fingerprints: `docs/protocol/membership.md`, "Account fingerprint".

use sha2::{Digest as _, Sha256};
use skald_proto::AccountId;

/// The deterministic CBOR of `["skald fingerprint", account, master]` up to the account ID: the
/// head of a 3-item array, the 17-byte text string and the head of a 16-byte byte string.
const PREFIX: &[u8] = b"\x83\x71skald fingerprint\x50";

/// The head of the 32-byte byte string of the master key.
const MASTER_HEAD: &[u8] = b"\x58\x20";

/// The fingerprint of `account` with the master public key `master`, as a device shows it: SHA-256
/// over the deterministic CBOR of `["skald fingerprint", account, master]`, each of its eight
/// 4-byte words as a big-endian number modulo 100000, in five digits, the groups joined by one
/// space.
pub fn account_fingerprint(account: AccountId, master: &[u8; 32]) -> String {
    let mut hash = Sha256::new();
    hash.update(PREFIX);
    hash.update(account.as_bytes());
    hash.update(MASTER_HEAD);
    hash.update(master);
    let digest: [u8; 32] = hash.finalize().into();
    let (words, _) = digest.as_chunks::<4>();
    words
        .iter()
        .map(|word| format!("{:05}", u32::from_be_bytes(*word) % 100_000))
        .collect::<Vec<_>>()
        .join(" ")
}
