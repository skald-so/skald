//! The master backup as `docs/protocol/membership.md`, "Account", seals it, with the `hpke` crate
//! apart from the code under test, and the recovery key's text form: for the tests of `skald-crypto`
//! and of the CLI.

use hkdf::Hkdf;
use hpke::aead::AesGcm128;
use hpke::kdf::HkdfSha256;
use hpke::kem::X25519HkdfSha256;
use hpke::{Deserializable as _, Kem as _, OpModeR, OpModeS, PskBundle, Serializable as _};
use sha2::Sha256;

/// `info` and `psk_id` of the master backup's seal, and `info` of its PSK's derivation.
pub const BACKUP_INFO: &[u8] = b"skald master backup";
const PSK_ID: &[u8] = b"skald master backup";
const PSK_INFO: &[u8] = b"skald master backup psk";
/// The base32 alphabet of RFC 4648, §6.
const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// The backup's PSK: `HKDF-SHA256(salt = "", IKM = recovery key, info = "skald master backup psk",
/// L = 32)`.
fn psk(recovery: &[u8; 32]) -> [u8; 32] {
    let mut psk = [0; 32];
    Hkdf::<Sha256>::new(Some(b""), recovery)
        .expand(PSK_INFO, &mut psk)
        .expect("32 bytes of HKDF-SHA256");
    psk
}

/// `plaintext` sealed as a master backup to `recovery` on the account `account`, its 16 bytes: HPKE
/// in PSK mode, single-shot, with DHKEM(X25519, HKDF-SHA256), HKDF-SHA256 and AES-128-GCM, to
/// `DeriveKeyPair(recovery)`; gives `enc` and `ct`.
pub fn seal(plaintext: &[u8], recovery: &[u8; 32], account: &[u8; 16]) -> ([u8; 32], Vec<u8>) {
    let psk = psk(recovery);
    let (_, recipient) = X25519HkdfSha256::derive_keypair(recovery);
    let mode = OpModeS::Psk(PskBundle::new(&psk, PSK_ID).expect("a PSK and its ID"));
    let (enc, ct) = hpke::single_shot_seal::<AesGcm128, HkdfSha256, X25519HkdfSha256>(
        &mode,
        &recipient,
        BACKUP_INFO,
        plaintext,
        account,
    )
    .expect("a seal to a derived key");
    (enc.to_bytes().into(), ct)
}

/// What `enc` and `ct` open to as a master backup of `recovery` on the account `account`, if they
/// open.
pub fn open(enc: &[u8], ct: &[u8], recovery: &[u8; 32], account: &[u8; 16]) -> Option<Vec<u8>> {
    let psk = psk(recovery);
    let (key, _) = X25519HkdfSha256::derive_keypair(recovery);
    let mode = OpModeR::Psk(PskBundle::new(&psk, PSK_ID).expect("a PSK and its ID"));
    let enc = <X25519HkdfSha256 as hpke::Kem>::EncappedKey::from_bytes(enc).ok()?;
    hpke::single_shot_open::<AesGcm128, HkdfSha256, X25519HkdfSha256>(
        &mode,
        &key,
        &enc,
        BACKUP_INFO,
        ct,
        account,
    )
    .ok()
}

/// The bytes of `text` in base32 of RFC 4648, §6, without padding; `None` for any other character,
/// a length no byte string has, or bits set past the last byte.
pub fn base32(text: &str) -> Option<Vec<u8>> {
    if matches!(text.len() % 8, 1 | 3 | 6) {
        return None;
    }
    let mut bits: u64 = 0;
    let mut count = 0;
    let mut out = Vec::new();
    for c in text.bytes() {
        let value = ALPHABET.iter().position(|a| *a == c)?;
        bits = (bits << 5) | value as u64;
        count += 5;
        if count >= 8 {
            count -= 8;
            out.push((bits >> count) as u8);
            bits &= (1 << count) - 1;
        }
    }
    (bits == 0).then_some(out)
}

/// The shown form of the recovery key `key`: base32 of RFC 4648, §6, without padding, in groups of
/// four joined by `-`.
pub fn shown_form(key: &[u8; 32]) -> String {
    let mut digits = Vec::new();
    let (mut bits, mut count) = (0u64, 0);
    for byte in key {
        bits = (bits << 8) | u64::from(*byte);
        count += 8;
        while count >= 5 {
            count -= 5;
            digits.push(ALPHABET[((bits >> count) & 0x1f) as usize]);
        }
    }
    if count > 0 {
        digits.push(ALPHABET[((bits << (5 - count)) & 0x1f) as usize]);
    }
    let groups: Vec<&str> = digits
        .chunks(4)
        .map(|group| std::str::from_utf8(group).expect("base32 is ASCII"))
        .collect();
    groups.join("-")
}
