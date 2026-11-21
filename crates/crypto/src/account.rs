//! The account record, the recovery key and the master backup: `docs/protocol/membership.md`,
//! "Account".

use std::fmt;

use aes_gcm::aead::{Aead as _, Payload};
use aes_gcm::{Aes256Gcm, KeyInit as _, Nonce};
use argon2::{Algorithm, Argon2, Params, Version};
use skald_proto::{
    ACCOUNT_LABEL, AccountBody, AccountId, AccountRecord, Cbor, DecodeError, MasterBackup, Unknown,
};
use zeroize::Zeroizing;

use crate::hpke::HpkeKey;
use crate::sign::{SigningKey, random_secret, verify};

/// Crockford's base32 alphabet, in the order of its values.
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
/// The characters of a recovery key, and of each group of its shown form.
const DIGITS: usize = 12;
const GROUP: usize = 4;
/// A terminal's bracketed-paste markers, before and after what is pasted; removed whole from an
/// input.
pub const PASTE_START: &str = "\x1b[200~";
pub const PASTE_END: &str = "\x1b[201~";
const PASTE_MARKERS: [&str; 2] = [PASTE_START, PASTE_END];
/// What comes before the account ID in the salt of the recovery key's derivation.
const SALT_PREFIX: &[u8] = b"skald recovery";
/// Argon2id's memory in KiB, passes and lanes: "Account".
const ARGON2_MEMORY: u32 = 262_144;
const ARGON2_PASSES: u32 = 8;
const ARGON2_LANES: u32 = 1;

/// An account's recovery key: 12 characters of Crockford's base32, zeroed on drop and never shown
/// but by [`RecoveryKey::shown`].
pub struct RecoveryKey(Zeroizing<[u8; DIGITS]>);

impl RecoveryKey {
    /// A new key of 60 bits from the cryptographically secure generator of the operating system,
    /// or in WASM of the browser or Node.js.
    ///
    /// # Panics
    /// If the random generator fails.
    pub fn generate() -> Self {
        let random = random_secret();
        let bits = Zeroizing::new(u64::from_le_bytes(random[..8].try_into().expect("8 bytes")));
        Self::from_bits(*bits)
    }

    /// The key of the low 60 bits of `bits`: its first character the lowest 5, each next one the
    /// next 5.
    fn from_bits(bits: u64) -> Self {
        let mut bits = Zeroizing::new(bits);
        let mut key = Zeroizing::new([0; DIGITS]);
        for digit in key.iter_mut() {
            *digit = ALPHABET[usize::try_from(*bits & 0x1f).expect("5 bits")];
            *bits >>= 5;
        }
        Self(key)
    }

    /// The text a person writes down: its 12 characters in 3 groups of 4 joined by `-`.
    pub fn shown(&self) -> Zeroizing<String> {
        let mut shown = Zeroizing::new(String::with_capacity(DIGITS + DIGITS / GROUP));
        for (at, digit) in self.0.iter().enumerate() {
            if at > 0 && at % GROUP == 0 {
                shown.push('-');
            }
            shown.push(char::from(*digit));
        }
        shown
    }

    /// The key a person typed or pasted: in any case, O read as 0 and I and L as 1; a terminal's
    /// bracketed-paste markers removed whole and every other character outside the alphabet
    /// skipped.
    ///
    /// # Errors
    /// [`RecoveryKeyError`] unless exactly 12 characters of the alphabet are left.
    pub fn parse(input: &str) -> Result<Self, RecoveryKeyError> {
        let mut unmarked = Zeroizing::new(input.to_owned());
        for marker in PASTE_MARKERS {
            if unmarked.contains(marker) {
                unmarked = Zeroizing::new(unmarked.replace(marker, ""));
            }
        }
        let mut key = Zeroizing::new([0; DIGITS]);
        let mut read = 0;
        for c in unmarked.chars() {
            let digit = match c.to_ascii_uppercase() {
                'O' => b'0',
                'I' | 'L' => b'1',
                c => match u8::try_from(c) {
                    Ok(c) if ALPHABET.contains(&c) => c,
                    _ => continue,
                },
            };
            *key.get_mut(read).ok_or(RecoveryKeyError)? = digit;
            read += 1;
        }
        if read != DIGITS {
            return Err(RecoveryKeyError);
        }
        Ok(Self(key))
    }

    /// Argon2id of the key on `account`, as "Account" says: the AEAD key of the master backup and
    /// the recovery HPKE key pair.
    fn derive(&self, account: AccountId) -> Derived {
        let params = Params::new(ARGON2_MEMORY, ARGON2_PASSES, ARGON2_LANES, Some(64))
            .expect("the spec's Argon2id parameters");
        let mut salt = Vec::with_capacity(SALT_PREFIX.len() + 16);
        salt.extend_from_slice(SALT_PREFIX);
        salt.extend_from_slice(account.as_bytes());
        let mut out = Zeroizing::new([0; 64]);
        Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
            .hash_password_into(self.0.as_ref(), &salt, out.as_mut())
            .expect("Argon2id of the spec's lengths");
        let (aead, ikm) = out.split_at(32);
        Derived {
            aead: Zeroizing::new(aead.try_into().expect("32 bytes")),
            hpke: HpkeKey::from_ikm(ikm.try_into().expect("32 bytes")),
        }
    }
}

impl fmt::Debug for RecoveryKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RecoveryKey(..)")
    }
}

/// A typed recovery key is not one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecoveryKeyError;

impl fmt::Display for RecoveryKeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("not a recovery key: 12 letters and digits")
    }
}

impl std::error::Error for RecoveryKeyError {}

/// What the recovery key derives on an account.
struct Derived {
    /// The AES-256-GCM key of the master backup.
    aead: Zeroizing<[u8; 32]>,
    /// The recovery HPKE key pair.
    hpke: HpkeKey,
}

impl Derived {
    fn cipher(&self) -> Aes256Gcm {
        Aes256Gcm::new((&*self.aead).into())
    }
}

/// What a master backup opened with the recovery key gives: the master key and the recovery HPKE
/// key pair.
pub struct Recovered {
    pub master: SigningKey,
    pub hpke: HpkeKey,
}

impl fmt::Debug for Recovered {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Recovered(..)")
    }
}

/// The body of the account record of `master` and `recovery` on `account`, revision 1, and the
/// master backup that seals `master` with `recovery`, as "Account" says: one derivation, a random
/// nonce.
///
/// # Panics
/// If the random generator fails.
pub fn new_recovery(
    master: &SigningKey,
    recovery: &RecoveryKey,
    account: AccountId,
) -> (AccountBody, MasterBackup) {
    let mut nonce = [0; 12];
    getrandom::fill(&mut nonce).expect("the random generator failed");
    new_recovery_with_nonce(master, recovery, account, nonce)
}

/// [`new_recovery`] with the backup's `nonce`, for a test vector.
pub fn new_recovery_with_nonce(
    master: &SigningKey,
    recovery: &RecoveryKey,
    account: AccountId,
    nonce: [u8; 12],
) -> (AccountBody, MasterBackup) {
    let derived = recovery.derive(account);
    let sealed = derived
        .cipher()
        .encrypt(
            &Nonce::from(nonce),
            Payload {
                msg: master.to_bytes().as_ref(),
                aad: account.as_bytes(),
            },
        )
        .expect("32 bytes within AES-GCM's length bound");
    let body = AccountBody {
        account,
        master: master.public_key(),
        recovery: derived.hpke.public_key(),
        revision: 1,
        unknown: Unknown::default(),
    };
    let ct = sealed.try_into().expect("the key and the tag");
    (body, MasterBackup { nonce, ct })
}

/// The master key and the recovery HPKE key pair `backup` holds for the account of `record`,
/// opened with `recovery`: one derivation. The caller checks `record` itself.
///
/// # Errors
/// [`AccountError::Open`] if `backup` does not open with `recovery` on the record's account;
/// [`AccountError::Key`] if it opens to something other than the private key of the record's
/// `master`, or the derived recovery HPKE key is not the record's `recovery`.
pub fn open_recovery(
    record: &AccountRecord,
    backup: &MasterBackup,
    recovery: &RecoveryKey,
) -> Result<Recovered, AccountError> {
    let body = record.body();
    let derived = recovery.derive(body.account);
    let opened = Zeroizing::new(
        derived
            .cipher()
            .decrypt(
                &Nonce::from(backup.nonce),
                Payload {
                    msg: &backup.ct,
                    aad: body.account.as_bytes(),
                },
            )
            .map_err(|_| AccountError::Open)?,
    );
    let secret: Zeroizing<[u8; 32]> = Zeroizing::new(
        opened
            .as_slice()
            .try_into()
            .map_err(|_| AccountError::Key)?,
    );
    let master = SigningKey::from_bytes(&secret);
    if master.public_key() != body.master || derived.hpke.public_key() != body.recovery {
        return Err(AccountError::Key);
    }
    Ok(Recovered {
        master,
        hpke: derived.hpke,
    })
}

/// The account record of `body`, signed by `master`, the account's master key.
///
/// # Errors
/// Why `body` breaks the spec's CDDL.
pub fn sign_account(body: &AccountBody, master: &SigningKey) -> Result<AccountRecord, DecodeError> {
    let bytes = body.to_cbor();
    let signature = master.sign(ACCOUNT_LABEL, &[&bytes]);
    AccountRecord::from_parts(bytes, signature)
}

/// Checks `record` against the pinned `master` key: it names it, and it signs it.
///
/// # Errors
/// [`AccountError::Master`] if it names another master key; [`AccountError::Signature`] if its
/// signature does not verify with `master`.
pub fn check_account(record: &AccountRecord, master: &[u8; 32]) -> Result<(), AccountError> {
    if record.body().master != *master {
        return Err(AccountError::Master);
    }
    verify(
        master,
        ACCOUNT_LABEL,
        &[record.body_bytes()],
        record.signature(),
    )
    .map_err(|_| AccountError::Signature)
}

/// Why an account record or a master backup is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountError {
    /// The record names another master key than the pinned one.
    Master,
    /// The record's signature does not verify with the master key.
    Signature,
    /// The backup does not open with the recovery key on the account.
    Open,
    /// The backup opens to a key other than the master key, or the recovery key derives another
    /// recovery HPKE key than the record's.
    Key,
}

impl fmt::Display for AccountError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Master => "the account record names another master key",
            Self::Signature => "the account record's signature does not verify with the master key",
            Self::Open => "the master backup does not open with the recovery key",
            Self::Key => {
                "the master backup holds another key than the master key, or the account record \
                 names another recovery key"
            }
        })
    }
}

impl std::error::Error for AccountError {}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each character is the value of its own 5 bits, the first the lowest: each of the 60 bits
    /// sets only its character's bit.
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn each_character_takes_its_own_5_of_the_60_bits() {
        assert_eq!(
            RecoveryKey::from_bits(0x0fed_cba9_8765_4321)
                .shown()
                .as_str(),
            "1SGA-P36N-BEVZ"
        );
        for bit in 0..60 {
            let mut digits = [b'0'; DIGITS];
            digits[bit / 5] = ALPHABET[1 << (bit % 5)];
            let key = RecoveryKey::from_bits(1 << bit);
            assert_eq!(*key.0, digits, "bit {bit}");
        }
        assert_eq!(*RecoveryKey::from_bits(0xf << 60).0, [b'0'; DIGITS]);
    }
}
