//! Looking for keys in what a machine or a server keeps or sends, whatever their encoding: raw
//! bytes, hex, base64 or base64url with or without padding, or a list of byte values such as a JSON
//! array.

use base64::Engine as _;
use base64::alphabet::STANDARD;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
use curve25519_dalek::MontgomeryPoint;
use skald_crypto::SigningKey;

/// The Ed25519 public key of the private key `seed`.
pub fn ed25519(seed: &[u8; 32]) -> [u8; 32] {
    SigningKey::from_bytes(seed).public_key()
}

/// The X25519 public key of the private key `secret`.
pub fn x25519(secret: &[u8; 32]) -> [u8; 32] {
    MontgomeryPoint::mul_base_clamped(*secret).to_bytes()
}

/// Whether `bytes` hold `value` in any of the encodings above.
pub fn holds(bytes: &[u8], value: &[u8]) -> bool {
    decoded(bytes)
        .iter()
        .any(|found| found.windows(value.len()).any(|window| window == value))
}

/// The public keys, by `public_of`, of every private key `bytes` may hold in any of the encodings
/// above.
pub fn public_keys(bytes: &[u8], public_of: fn(&[u8; 32]) -> [u8; 32]) -> Vec<[u8; 32]> {
    candidates(bytes).iter().map(public_of).collect()
}

/// The private key `bytes` hold, in any of the encodings above, whose public key by `public_of` is
/// `public`.
pub fn private_key(
    bytes: &[u8],
    public: &[u8; 32],
    public_of: fn(&[u8; 32]) -> [u8; 32],
) -> Option<[u8; 32]> {
    candidates(bytes)
        .into_iter()
        .find(|candidate| public_of(candidate) == *public)
}

/// Every 32 bytes in `bytes`, in any of the encodings above, that look random, as a private key
/// does: none has four zero bytes in a row or only printable ASCII, which 32 random bytes have
/// with a chance below 10^-8. So raw text and the zeros of a database's free space are skipped.
fn candidates(bytes: &[u8]) -> Vec<[u8; 32]> {
    let mut found = Vec::new();
    for decoded in decoded(bytes) {
        for window in decoded.windows(32) {
            let zeros = window.windows(4).any(|four| four == [0; 4]);
            let text = window.iter().all(|byte| (b' '..=b'~').contains(byte));
            if !zeros && !text {
                found.push(window.try_into().expect("32 bytes"));
            }
        }
    }
    found
}

/// `bytes` themselves and every byte string encoded in their text.
fn decoded(bytes: &[u8]) -> Vec<Vec<u8>> {
    let text = String::from_utf8_lossy(bytes);
    let mut found = vec![bytes.to_vec()];
    // 32 bytes take 43 characters of base64 and 64 of hex.
    for run in runs(
        &text,
        |c| c.is_ascii_alphanumeric() || "+/_-".contains(c),
        43,
    ) {
        found.extend(base64(run));
    }
    for run in runs(&text, |c| c.is_ascii_hexdigit(), 64) {
        found.extend(hex(run));
    }
    found.extend(byte_lists(&text));
    found
}

/// The longest runs of at least `least` characters of `text` that `belongs` takes.
fn runs(text: &str, belongs: fn(char) -> bool, least: usize) -> Vec<&str> {
    let mut found = Vec::new();
    let mut start = None;
    for (at, c) in text.char_indices().chain([(text.len(), '\0')]) {
        match (belongs(c) && at < text.len(), start) {
            (true, None) => start = Some(at),
            (false, Some(from)) => {
                if at - from >= least {
                    found.push(&text[from..at]);
                }
                start = None;
            }
            _ => {}
        }
    }
    found
}

/// The bytes of `run` as base64 or base64url, padded or not; a run whose end is not part of the
/// encoding is cut until it decodes.
fn base64(run: &str) -> Option<Vec<u8>> {
    let engine = GeneralPurpose::new(
        &STANDARD,
        GeneralPurposeConfig::new()
            .with_decode_padding_mode(DecodePaddingMode::Indifferent)
            .with_decode_allow_trailing_bits(true),
    );
    let standard: String = run
        .chars()
        .map(|c| match c {
            '-' => '+',
            '_' => '/',
            c => c,
        })
        .collect();
    let trimmed = standard.trim_end_matches('=');
    (0..4).find_map(|cut| {
        let end = trimmed.len().checked_sub(cut)?;
        engine.decode(&trimmed[..end]).ok()
    })
}

/// The bytes of `run` as hex, read from its first character and from its second.
fn hex(run: &str) -> Vec<Vec<u8>> {
    let digits: Vec<u8> = run
        .chars()
        .map(|c| u8::try_from(c.to_digit(16).expect("a hex digit")).expect("below 16"))
        .collect();
    (0..2)
        .map(|from| {
            digits[from..]
                .as_chunks::<2>()
                .0
                .iter()
                .map(|[high, low]| high << 4 | low)
                .collect()
        })
        .collect()
}

/// Each list of at least 32 byte values in `text`, decimal and separated by commas, such as a JSON
/// array of bytes.
fn byte_lists(text: &str) -> Vec<Vec<u8>> {
    let mut found = Vec::new();
    for segment in text.split(|c: char| !(c.is_ascii_digit() || c == ',' || c.is_whitespace())) {
        let mut list = Vec::new();
        for part in segment.split(',').map(str::trim) {
            match part.parse::<u8>() {
                Ok(byte) => list.push(byte),
                Err(_) => {
                    if list.len() >= 32 {
                        found.push(std::mem::take(&mut list));
                    }
                    list.clear();
                }
            }
        }
        if list.len() >= 32 {
            found.push(list);
        }
    }
    found
}
