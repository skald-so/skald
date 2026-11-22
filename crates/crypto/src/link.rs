//! Link codes: `docs/protocol/membership.md`, "Link code". The code, its shown form and its entry,
//! with a run's pairing for a new device without a session; the run of CPace it keys between the
//! new device, A, and the issuing device, B; and the request and grant keys of the run's ISK, each
//! sealing one plaintext.

use std::fmt;

use aes_gcm::aead::{Aead as _, Payload};
use aes_gcm::{Aes128Gcm, KeyInit as _, Nonce};
use hkdf::Hkdf;
use sha2::Sha256;
use skald_proto::{AccountId, signed_data};
use zeroize::Zeroizing;

use crate::cpace::{CpaceError, CpaceInitiator, CpaceResponder, Isk};

/// The base32 alphabet of RFC 4648, §6.
const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
/// The characters of a code, and of a pairing.
const LEN: usize = 6;
/// The label of the CPace channel identifier.
const CI_LABEL: &str = "skald link code";
/// The labels of the request and grant keys.
const REQUEST_LABEL: &[u8] = b"skald link request";
const GRANT_LABEL: &[u8] = b"skald link grant";

/// A link code: its 6 characters of the base32 alphabet, in uppercase ASCII, which are its PRS.
#[derive(Clone, PartialEq, Eq)]
pub struct LinkCode([u8; LEN]);

/// A run's pairing: 6 characters of the base32 alphabet, in uppercase ASCII, which the server picks
/// for a run of a new device without a session and the run's CI binds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkPairing([u8; LEN]);

/// What the person enters on the issuing device: a code, or a run's pairing and its code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkEntry {
    /// 6 characters: the code of a run of the account, its latest hello's.
    Code(LinkCode),
    /// 12 characters: the pairing of a run without a session, then its code.
    Paired {
        pairing: LinkPairing,
        code: LinkCode,
    },
}

/// An entry that is not 6 or 12 characters of the alphabet, read as "Link code" says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkEntryError;

impl fmt::Display for LinkEntryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a link code is 6 or 12 characters of A to Z and 2 to 7")
    }
}

impl std::error::Error for LinkEntryError {}

/// The characters of the alphabet in `entry`, read as "Link code" says: case, spaces and `-`
/// ignored, `0`, `1` and `8` read as `O`, `I` and `B`. `None` for any other character.
fn read(entry: &str) -> Option<Vec<u8>> {
    entry
        .chars()
        .filter(|c| !matches!(c, ' ' | '-'))
        .map(|c| {
            let c = match c.to_ascii_uppercase() {
                '0' => 'O',
                '1' => 'I',
                '8' => 'B',
                c => c,
            };
            u8::try_from(c).ok().filter(|byte| ALPHABET.contains(byte))
        })
        .collect()
}

/// 6 characters of the alphabet from the cryptographically secure generator.
fn generate() -> [u8; LEN] {
    let mut random = [0; 4];
    getrandom::fill(&mut random).expect("the random generator failed");
    let bits = u32::from_le_bytes(random);
    std::array::from_fn(|at| ALPHABET[((bits >> (5 * at)) & 0x1f) as usize])
}

impl LinkEntry {
    /// What `entry` holds, read as "Link code" says.
    ///
    /// # Errors
    /// [`LinkEntryError`] unless the rest is 6 or 12 characters of the alphabet.
    pub fn from_entry(entry: &str) -> Result<Self, LinkEntryError> {
        let read = read(entry).ok_or(LinkEntryError)?;
        match read.len() {
            LEN => Ok(Self::Code(LinkCode(read.try_into().expect("6 characters")))),
            12 => {
                let (pairing, code) = read.split_at(LEN);
                Ok(Self::Paired {
                    pairing: LinkPairing(pairing.try_into().expect("6 characters")),
                    code: LinkCode(code.try_into().expect("6 characters")),
                })
            }
            _ => Err(LinkEntryError),
        }
    }
}

impl LinkPairing {
    /// A fresh pairing: 30 bits from the cryptographically secure generator of the operating
    /// system, or in WASM of the browser or Node.js.
    ///
    /// # Panics
    /// If the random generator fails.
    pub fn generate() -> Self {
        Self(generate())
    }

    /// The pairing `text` holds, read as an entry of a code is.
    ///
    /// # Errors
    /// [`LinkEntryError`] unless the rest is 6 characters of the alphabet.
    pub fn from_entry(text: &str) -> Result<Self, LinkEntryError> {
        let read = read(text).ok_or(LinkEntryError)?;
        Ok(Self(read.try_into().map_err(|_| LinkEntryError)?))
    }

    /// Its 6 characters, in uppercase ASCII.
    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.0).expect("ASCII")
    }
}

/// Its 6 characters.
impl fmt::Display for LinkPairing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An entry that is not 6 characters of the alphabet, read as "Link code" says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkCodeError;

impl fmt::Display for LinkCodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a link code is 6 characters of A to Z and 2 to 7")
    }
}

impl std::error::Error for LinkCodeError {}

impl LinkCode {
    /// A fresh code: 30 bits from the cryptographically secure generator of the operating system,
    /// or in WASM of the browser or Node.js.
    ///
    /// # Panics
    /// If the random generator fails.
    pub fn generate() -> Self {
        Self(generate())
    }

    /// The code `entry` holds: case, spaces and `-` ignored, `0`, `1` and `8` read as `O`, `I` and
    /// `B`.
    ///
    /// # Errors
    /// [`LinkCodeError`] unless the rest is 6 characters of the alphabet.
    pub fn from_entry(entry: &str) -> Result<Self, LinkCodeError> {
        let read = read(entry).ok_or(LinkCodeError)?;
        Ok(Self(read.try_into().map_err(|_| LinkCodeError)?))
    }

    /// The new device's side, A, of a run with this code on `account`'s link with the `session`
    /// it chose: A's state, which takes B's reply, and its share Ya for the hello.
    ///
    /// # Panics
    /// If the random generator fails.
    pub fn initiate(&self, account: AccountId, session: &[u8; 16]) -> (CpaceInitiator, [u8; 32]) {
        CpaceInitiator::start(&self.0, &ci(account), session, &[])
    }

    /// The issuing device's side, B, of a run with this code on `account`'s link, given the
    /// hello's `session` and share Ya: its share Yb for the reply, and the ISK.
    ///
    /// # Errors
    /// [`CpaceError`] if Ya does not decode or K is the identity.
    ///
    /// # Panics
    /// If the random generator fails.
    pub fn respond(
        &self,
        account: AccountId,
        session: &[u8; 16],
        share: &[u8; 32],
    ) -> Result<([u8; 32], Isk), CpaceError> {
        CpaceResponder::respond(&self.0, &ci(account), session, &[], share, &[])
    }

    /// The new device's side, A, of a run with this code started without a session, with the
    /// run's `pairing` and the `session` it chose: A's state and its share Ya, as
    /// [`LinkCode::initiate`] gives them.
    ///
    /// # Panics
    /// If the random generator fails.
    pub fn initiate_paired(
        &self,
        pairing: &LinkPairing,
        session: &[u8; 16],
    ) -> (CpaceInitiator, [u8; 32]) {
        CpaceInitiator::start(&self.0, &paired_ci(pairing), session, &[])
    }

    /// The issuing device's side, B, of a run with this code started without a session, with the
    /// `pairing` its user entered, as [`LinkCode::respond`] gives it.
    ///
    /// # Errors
    /// [`CpaceError`] if Ya does not decode or K is the identity.
    ///
    /// # Panics
    /// If the random generator fails.
    pub fn respond_paired(
        &self,
        pairing: &LinkPairing,
        session: &[u8; 16],
        share: &[u8; 32],
    ) -> Result<([u8; 32], Isk), CpaceError> {
        CpaceResponder::respond(&self.0, &paired_ci(pairing), session, &[], share, &[])
    }

    /// The shown form of this code with the run's `pairing`: its 12 characters, the pairing first,
    /// in three groups of four joined by `-`.
    pub fn shown_with(&self, pairing: &LinkPairing) -> String {
        let all = [pairing.0, self.0].concat();
        let text = |group: &[u8]| std::str::from_utf8(group).expect("ASCII").to_owned();
        all.chunks(4).map(text).collect::<Vec<_>>().join("-")
    }
}

/// The shown form: two groups of three joined by `-`.
impl fmt::Display for LinkCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (first, second) = self.0.split_at(LEN / 2);
        let text = |half| std::str::from_utf8(half).expect("ASCII");
        write!(f, "{}-{}", text(first), text(second))
    }
}

/// Never shows the code.
impl fmt::Debug for LinkCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("LinkCode(..)")
    }
}

/// A fresh `session` for a run: 16 bytes from the cryptographically secure generator.
///
/// # Panics
/// If the random generator fails.
pub fn link_session() -> [u8; 16] {
    let mut session = [0; 16];
    getrandom::fill(&mut session).expect("the random generator failed");
    session
}

/// The CI of a run on `account`: the encoded `["skald link code", account]`.
fn ci(account: AccountId) -> Vec<u8> {
    signed_data(CI_LABEL, &[account.as_bytes()])
}

/// The CI of a run started without a session: the encoded `["skald link code", pairing]`.
fn paired_ci(pairing: &LinkPairing) -> Vec<u8> {
    signed_data(CI_LABEL, &[&pairing.0])
}

impl Isk {
    /// The key that seals the new device's request.
    pub fn request_key(&self) -> LinkKey {
        LinkKey::derive(self, REQUEST_LABEL)
    }

    /// The key that seals the issuing device's grant.
    pub fn grant_key(&self) -> LinkKey {
        LinkKey::derive(self, GRANT_LABEL)
    }
}

/// A request or grant key: AEAD_AES_128_GCM with a nonce of 12 zero bytes and empty associated
/// data, so it seals one plaintext only. Zeroed on drop and never shown.
pub struct LinkKey(Zeroizing<[u8; 16]>);

/// Bytes the key did not seal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkOpenError;

impl fmt::Display for LinkOpenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("it does not decrypt with the key")
    }
}

impl std::error::Error for LinkOpenError {}

impl LinkKey {
    /// `HKDF-SHA256(salt = "", IKM = ISK, info = label, L = 16)`.
    fn derive(isk: &Isk, label: &[u8]) -> Self {
        let mut key = Zeroizing::new([0; 16]);
        Hkdf::<Sha256>::new(Some(&[]), isk.bytes())
            .expand(label, key.as_mut())
            .expect("16 bytes are within HKDF-SHA256's output bound");
        Self(key)
    }

    /// Its 16 bytes.
    pub fn to_bytes(&self) -> Zeroizing<[u8; 16]> {
        self.0.clone()
    }

    fn cipher(&self) -> Aes128Gcm {
        Aes128Gcm::new((&*self.0).into())
    }

    /// `plaintext` sealed, the key spent: the ciphertext followed by its 16-byte tag.
    pub fn seal(self, plaintext: &[u8]) -> Vec<u8> {
        self.cipher()
            .encrypt(
                &Nonce::from([0; 12]),
                Payload {
                    msg: plaintext,
                    aad: &[],
                },
            )
            .expect("a plaintext within AES-GCM's length bound")
    }

    /// The plaintext of `sealed`, as [`LinkKey::seal`] sealed it with this key.
    ///
    /// # Errors
    /// [`LinkOpenError`] for bytes the key did not seal.
    pub fn open(&self, sealed: &[u8]) -> Result<Zeroizing<Vec<u8>>, LinkOpenError> {
        self.cipher()
            .decrypt(
                &Nonce::from([0; 12]),
                Payload {
                    msg: sealed,
                    aad: &[],
                },
            )
            .map(Zeroizing::new)
            .map_err(|_| LinkOpenError)
    }
}

impl fmt::Debug for LinkKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("LinkKey(..)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn each_session_is_fresh() {
        assert_ne!(link_session(), link_session());
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn codes_and_keys_are_never_shown_and_errors_say_why() {
        let code = LinkCode::from_entry("K7Q-M2X").expect("a code");
        assert_eq!(format!("{code:?}"), "LinkCode(..)");
        let key = Isk::from_bytes(&[7; 64]).request_key();
        assert_eq!(format!("{key:?}"), "LinkKey(..)");
        assert_eq!(
            LinkCodeError.to_string(),
            "a link code is 6 characters of A to Z and 2 to 7"
        );
        assert_eq!(
            LinkEntryError.to_string(),
            "a link code is 6 or 12 characters of A to Z and 2 to 7"
        );
        assert_eq!(
            LinkOpenError.to_string(),
            "it does not decrypt with the key"
        );
    }
}
