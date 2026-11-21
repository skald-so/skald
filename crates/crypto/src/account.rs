//! The account record, the recovery key and the master backup: `docs/protocol/membership.md`,
//! "Account".

use std::fmt;

use hkdf::Hkdf;
use hpke::aead::AesGcm128;
use hpke::kdf::HkdfSha256;
use hpke::kem::X25519HkdfSha256;
use hpke::{Deserializable as _, Kem, OpModeR, OpModeS, PskBundle, Serializable as _};
use sha2::Sha256;
use skald_proto::{
    ACCOUNT_LABEL, AccountBody, AccountId, AccountRecord, Cbor, DecodeError, MasterBackup,
};
use zeroize::Zeroizing;

use crate::hpke::Fixed;
use crate::sign::{SigningKey, secret_bytes, verify};

/// `info` and `psk_id` of the master backup's seal.
const BACKUP_INFO: &[u8] = b"skald master backup";
/// `info` of the derivation of the master backup's PSK.
const PSK_INFO: &[u8] = b"skald master backup psk";
/// The base32 alphabet of RFC 4648, §6.
const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
/// The characters of a recovery key's shown form, without `-`.
const DIGITS: usize = 52;

/// The KEM of the suite.
type Dhkem = X25519HkdfSha256;

/// An account's recovery key: 32 bytes, zeroed on drop and never shown but by [`RecoveryKey::shown`].
pub struct RecoveryKey(Zeroizing<[u8; 32]>);

secret_bytes!(RecoveryKey);

impl RecoveryKey {
    /// The text a person writes down: base32 of RFC 4648, §6, without padding, in groups of four
    /// characters joined by `-`.
    pub fn shown(&self) -> Zeroizing<String> {
        let mut digits = Zeroizing::new(Vec::with_capacity(DIGITS));
        let (mut bits, mut count) = (0u16, 0);
        for byte in self.0.iter() {
            bits = (bits << 8) | u16::from(*byte);
            count += 8;
            while count >= 5 {
                count -= 5;
                digits.push(ALPHABET[usize::from((bits >> count) & 0x1f)]);
            }
            bits &= (1 << count) - 1;
        }
        if count > 0 {
            digits.push(ALPHABET[usize::from((bits << (5 - count)) & 0x1f)]);
        }
        let mut shown = Zeroizing::new(String::with_capacity(DIGITS + DIGITS / 4));
        for (at, digit) in digits.iter().enumerate() {
            if at > 0 && at % 4 == 0 {
                shown.push('-');
            }
            shown.push(char::from(*digit));
        }
        shown
    }

    /// The key a person typed: its shown form in any case, with or without spaces and `-`.
    ///
    /// # Errors
    /// [`RecoveryKeyError`] if, without spaces and `-`, it is not 52 characters of the alphabet that
    /// encode 32 bytes.
    pub fn parse(input: &str) -> Result<Self, RecoveryKeyError> {
        let mut key = Zeroizing::new([0; 32]);
        let (mut bits, mut count, mut read, mut at) = (0u16, 0, 0, 0);
        for c in input.chars().filter(|c| !matches!(c, ' ' | '-')) {
            let value = u8::try_from(c.to_ascii_uppercase())
                .ok()
                .and_then(|c| ALPHABET.iter().position(|digit| *digit == c))
                .ok_or(RecoveryKeyError)?;
            read += 1;
            if read > DIGITS {
                return Err(RecoveryKeyError);
            }
            bits = (bits << 5) | u16::try_from(value).expect("a base32 digit");
            count += 5;
            if count >= 8 {
                count -= 8;
                key[at] = u8::try_from(bits >> count).expect("a byte");
                at += 1;
                bits &= (1 << count) - 1;
            }
        }
        // 52 digits hold 32 bytes and 4 bits more, which are zero.
        if read != DIGITS || bits != 0 {
            return Err(RecoveryKeyError);
        }
        Ok(Self(key))
    }

    /// The backup's PSK: `HKDF-SHA256(salt = "", IKM = recovery key, info = "skald master backup
    /// psk", L = 32)`.
    fn psk(&self) -> Zeroizing<[u8; 32]> {
        let mut psk = Zeroizing::new([0; 32]);
        Hkdf::<Sha256>::new(Some(b""), self.0.as_ref())
            .expand(PSK_INFO, psk.as_mut())
            .expect("32 bytes of HKDF-SHA256");
        psk
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
        f.write_str("not a recovery key: 13 groups of four letters A to Z and digits 2 to 7")
    }
}

impl std::error::Error for RecoveryKeyError {}

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
    /// The backup opens to a key other than the master key.
    Key,
}

impl fmt::Display for AccountError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Master => "the account record names another master key",
            Self::Signature => "the account record's signature does not verify with the master key",
            Self::Open => "the master backup does not open with the recovery key",
            Self::Key => "the master backup holds another key than the master key",
        })
    }
}

impl std::error::Error for AccountError {}

/// `master`, the account's master key, sealed to `recovery` on `account`: HPKE in PSK mode,
/// single-shot, to `DeriveKeyPair(recovery)`, as "Account" says. Each seal takes a new ephemeral
/// key.
///
/// # Panics
/// If the random generator fails.
pub fn seal_master_backup(
    master: &SigningKey,
    recovery: &RecoveryKey,
    account: AccountId,
) -> MasterBackup {
    seal(
        master,
        recovery,
        account,
        |mode, recipient, plaintext, aad| {
            hpke::single_shot_seal::<AesGcm128, HkdfSha256, Dhkem>(
                mode,
                recipient,
                BACKUP_INFO,
                plaintext,
                aad,
            )
        },
    )
}

/// [`seal_master_backup`] with the ephemeral key pair `DeriveKeyPair(ephemeral)`, for a test
/// vector.
pub fn seal_master_backup_with_ephemeral(
    master: &SigningKey,
    recovery: &RecoveryKey,
    account: AccountId,
    ephemeral: &[u8; 32],
) -> MasterBackup {
    seal(
        master,
        recovery,
        account,
        |mode, recipient, plaintext, aad| {
            hpke::single_shot_seal_with_rng::<AesGcm128, HkdfSha256, Dhkem>(
                mode,
                recipient,
                BACKUP_INFO,
                plaintext,
                aad,
                &mut Fixed(Some(*ephemeral)),
            )
        },
    )
}

type Sealed = Result<(<Dhkem as Kem>::EncappedKey, Vec<u8>), hpke::HpkeError>;

fn seal(
    master: &SigningKey,
    recovery: &RecoveryKey,
    account: AccountId,
    single_shot: impl FnOnce(&OpModeS<'_, Dhkem>, &<Dhkem as Kem>::PublicKey, &[u8], &[u8]) -> Sealed,
) -> MasterBackup {
    let (_, recipient) = Dhkem::derive_keypair(recovery.0.as_ref());
    let psk = recovery.psk();
    let mode = OpModeS::Psk(PskBundle::new(psk.as_ref(), BACKUP_INFO).expect("a PSK and its ID"));
    let (enc, ct) = single_shot(
        &mode,
        &recipient,
        master.to_bytes().as_ref(),
        account.as_bytes(),
    )
    .expect("a seal to a derived key");
    MasterBackup {
        enc: enc.to_bytes().into(),
        ct,
    }
}

/// The master key `backup` holds for `account`, opened with `recovery`: it must be `master`, the
/// master public key.
///
/// # Errors
/// [`AccountError::Open`] if it does not open in PSK mode with `recovery` on `account`;
/// [`AccountError::Key`] if it opens to something other than `master`'s private key.
pub fn open_master_backup(
    backup: &MasterBackup,
    recovery: &RecoveryKey,
    account: AccountId,
    master: &[u8; 32],
) -> Result<SigningKey, AccountError> {
    let (key, _) = Dhkem::derive_keypair(recovery.0.as_ref());
    let psk = recovery.psk();
    let mode = OpModeR::Psk(PskBundle::new(psk.as_ref(), BACKUP_INFO).expect("a PSK and its ID"));
    let enc =
        <Dhkem as Kem>::EncappedKey::from_bytes(&backup.enc).map_err(|_| AccountError::Open)?;
    let opened = Zeroizing::new(
        hpke::single_shot_open::<AesGcm128, HkdfSha256, Dhkem>(
            &mode,
            &key,
            &enc,
            BACKUP_INFO,
            &backup.ct,
            account.as_bytes(),
        )
        .map_err(|_| AccountError::Open)?,
    );
    let secret: Zeroizing<[u8; 32]> = Zeroizing::new(
        opened
            .as_slice()
            .try_into()
            .map_err(|_| AccountError::Key)?,
    );
    let opened = SigningKey::from_bytes(&secret);
    if opened.public_key() != *master {
        return Err(AccountError::Key);
    }
    Ok(opened)
}
