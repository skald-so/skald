//! Identifiers: `docs/protocol/README.md`, "Identifiers".

use std::fmt;
use std::hash::Hash;
use std::marker::PhantomData;
use std::str::FromStr;

use sha2::{Digest, Sha256};

/// What an [`Id`] identifies.
pub trait Kind: sealed::Sealed + Copy + Eq + Ord + Hash {
    /// The name of the ID type, such as `DeviceId`.
    const NAME: &'static str;
}

/// A kind whose IDs the issuer generates as UUIDv7: every kind but devices.
pub trait Issued: Kind {}

mod sealed {
    pub trait Sealed {}
}

/// The kinds of [`Id`].
pub mod kind {
    macro_rules! kinds {
        ($($kind:ident $name:literal: $doc:literal;)*) => {$(
            #[doc = $doc]
            #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
            pub enum $kind {}
            impl super::sealed::Sealed for $kind {}
            impl super::Kind for $kind {
                const NAME: &'static str = $name;
            }
        )*};
    }

    kinds! {
        Account "AccountId": "An account, issued by the server at sign-up.";
        Device "DeviceId": "A device, derived from its signing key.";
        Space "SpaceId": "A space, issued by the server at creation.";
        Channel "ChannelId": "A channel, issued by the server; also its MLS `group_id`.";
        Message "MessageId": "A message (`msg_id`), issued by the sending device; also its A2A `messageId`.";
        Queue "QueueId": "A queue, issued by the server at creation.";
        Task "TaskId": "A task (`task`), issued by the sending device; also the A2A `messageId` of its request.";
        Claim "ClaimId": "A claim of a task (`claim_id`), issued by the server at the claim.";
        Blob "BlobId": "An attachment's encrypted blob, issued by the server at upload.";
    }

    impl super::Issued for Account {}
    impl super::Issued for Space {}
    impl super::Issued for Channel {}
    impl super::Issued for Message {}
    impl super::Issued for Queue {}
    impl super::Issued for Task {}
    impl super::Issued for Claim {}
    impl super::Issued for Blob {}
}

/// A UUID that identifies one entity of kind `K`: 16 bytes on the wire, the lowercase hyphenated
/// form in text. Receivers treat it as opaque, so any 16 bytes are an ID.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Id<K: Kind> {
    bytes: [u8; 16],
    kind: PhantomData<K>,
}

pub type AccountId = Id<kind::Account>;
pub type DeviceId = Id<kind::Device>;
pub type SpaceId = Id<kind::Space>;
pub type ChannelId = Id<kind::Channel>;
pub type MessageId = Id<kind::Message>;
pub type QueueId = Id<kind::Queue>;
pub type TaskId = Id<kind::Task>;
pub type ClaimId = Id<kind::Claim>;
pub type BlobId = Id<kind::Blob>;

/// The length of an ID in text: 32 hex digits and 4 hyphens.
const TEXT_LEN: usize = 36;
/// Where the hyphens are in the text form.
const HYPHENS: [usize; 4] = [8, 13, 18, 23];
/// The deterministic CBOR of `["skald device", signing]` up to the key: the head of a 2-item array,
/// the 12-byte text string and the head of a 32-byte byte string.
const DEVICE_PREFIX: &[u8] = b"\x82\x6cskald device\x58\x20";

impl<K: Kind> Id<K> {
    /// The ID with these wire bytes.
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self {
            bytes,
            kind: PhantomData,
        }
    }

    /// The wire bytes.
    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.bytes
    }
}

impl<K: Issued> Id<K> {
    /// A new UUIDv7: the current Unix time in milliseconds and random bits from the
    /// cryptographically secure generator of the operating system, or in WASM of the browser or
    /// Node.js.
    ///
    /// # Panics
    /// If the random generator fails.
    pub fn generate() -> Self {
        let mut random = [0; 16];
        getrandom::fill(&mut random).expect("the random generator failed");
        Self::from_bytes(v7(unix_ms(), random))
    }
}

/// The current Unix time in milliseconds; 0 before the Unix epoch.
fn unix_ms() -> u64 {
    // `SystemTime` panics in WASM: there the time comes from JavaScript's clock, in whole
    // milliseconds well below 2^53; `as` saturates a time before the epoch to 0.
    #[cfg(target_arch = "wasm32")]
    let unix_ms = js_sys::Date::now() as u128;
    #[cfg(not(target_arch = "wasm32"))]
    let unix_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_millis());
    u64::try_from(unix_ms).unwrap_or(u64::MAX)
}

impl DeviceId {
    /// The ID of the device with this Ed25519 public signing key: a UUIDv8 of the first 16 bytes
    /// of SHA-256 over the deterministic CBOR of `["skald device", signing]`.
    pub fn from_signing_key(signing: &[u8; 32]) -> Self {
        let mut hash = Sha256::new();
        hash.update(DEVICE_PREFIX);
        hash.update(signing);
        let mut bytes = [0; 16];
        bytes.copy_from_slice(&hash.finalize()[..16]);
        Self::from_bytes(with_version(bytes, 8))
    }

    /// Whether this is the ID derived from `signing`: the check of a device ID against its
    /// certificate's signing key.
    pub fn is_derived_from(&self, signing: &[u8; 32]) -> bool {
        *self == Self::from_signing_key(signing)
    }
}

/// A UUIDv7 of a 48-bit Unix time in milliseconds and the random bits of `random`.
fn v7(unix_ms: u64, random: [u8; 16]) -> [u8; 16] {
    let mut bytes = random;
    bytes[..6].copy_from_slice(&unix_ms.to_be_bytes()[2..]);
    with_version(bytes, 7)
}

/// `bytes` with the UUID version field set to `version` and the variant field to `0b10`.
/// The fields are cleared first, so `+` sets them.
fn with_version(mut bytes: [u8; 16], version: u8) -> [u8; 16] {
    bytes[6] = (bytes[6] & 0x0f) + (version << 4);
    bytes[8] = (bytes[8] & 0x3f) + 0x80;
    bytes
}

/// Why bytes or text are not an ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdError {
    /// The wire form is not 16 bytes; holds the length.
    ByteLength(usize),
    /// The text form is not 36 characters; holds the length in bytes.
    TextLength(usize),
    /// The text form has no hyphen at this byte position, counting from 0.
    MissingHyphen(usize),
    /// The text form has an uppercase hex digit at this byte position.
    Uppercase(usize),
    /// The text form has a character other than a lowercase hex digit at this byte position.
    InvalidChar(usize),
}

impl fmt::Display for IdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ByteLength(len) => write!(f, "an ID is 16 bytes, not {len}"),
            Self::TextLength(len) => write!(
                f,
                "an ID is 36 characters, such as 01a1c4c4-94cb-7a3e-9c41-5d2e8f6b7a10, not {len}"
            ),
            Self::MissingHyphen(at) => write!(f, "an ID needs '-' at position {at}"),
            Self::Uppercase(at) => write!(
                f,
                "an ID has an uppercase letter at position {at}; IDs are lowercase"
            ),
            Self::InvalidChar(at) => write!(
                f,
                "an ID has a character other than 0-9 and a-f at position {at}"
            ),
        }
    }
}

impl std::error::Error for IdError {}

impl<K: Kind> FromStr for Id<K> {
    type Err = IdError;

    /// Parses the lowercase hyphenated form; any other form is invalid.
    fn from_str(text: &str) -> Result<Self, IdError> {
        let text = text.as_bytes();
        if text.len() != TEXT_LEN {
            return Err(IdError::TextLength(text.len()));
        }
        let mut digits = [0; 32];
        let mut count = 0;
        for (at, &char) in text.iter().enumerate() {
            if HYPHENS.contains(&at) {
                if char != b'-' {
                    return Err(IdError::MissingHyphen(at));
                }
                continue;
            }
            digits[count] = match char {
                b'0'..=b'9' => char - b'0',
                b'a'..=b'f' => char - b'a' + 10,
                b'A'..=b'F' => return Err(IdError::Uppercase(at)),
                _ => return Err(IdError::InvalidChar(at)),
            };
            count += 1;
        }
        let mut bytes = [0; 16];
        for (byte, pair) in bytes.iter_mut().zip(digits.as_chunks::<2>().0) {
            *byte = pair[0] * 16 + pair[1];
        }
        Ok(Self::from_bytes(bytes))
    }
}

impl<K: Kind> TryFrom<&[u8]> for Id<K> {
    type Error = IdError;

    /// The ID in its wire form: exactly 16 bytes.
    fn try_from(bytes: &[u8]) -> Result<Self, IdError> {
        <[u8; 16]>::try_from(bytes)
            .map(Self::from_bytes)
            .map_err(|_| IdError::ByteLength(bytes.len()))
    }
}

impl<K: Kind> fmt::Display for Id<K> {
    /// The lowercase hyphenated form.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (at, byte) in self.bytes.iter().enumerate() {
            if matches!(at, 4 | 6 | 8 | 10) {
                f.write_str("-")?;
            }
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl<K: Kind> fmt::Debug for Id<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}({self})", K::NAME)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v7_puts_the_time_first_and_keeps_the_random_bits() {
        let random = [0xff; 16];
        assert_eq!(
            v7(0x0123_4567_89ab, random),
            [
                0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0x7f, 0xff, 0xbf, 0xff, 0xff, 0xff, 0xff, 0xff,
                0xff, 0xff
            ]
        );
        let random = [0; 16];
        assert_eq!(
            v7(u64::MAX, random),
            [
                0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x70, 0, 0x80, 0, 0, 0, 0, 0, 0, 0
            ]
        );
    }
}
