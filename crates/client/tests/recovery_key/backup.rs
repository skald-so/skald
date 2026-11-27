//! The account record and the master backup the server holds, read as
//! `docs/protocol/membership.md`, "Account", says, apart from the code under test: a CBOR reader
//! for what they hold, the recovery key's shown form, its derivation, and the backup opened.

use skald_crypto::SigningKey;
use skald_proto::AccountId;

pub use crate::master_backup::shown_form;

/// The label of an account record's signature.
const LABEL: &str = "skald account";

/// A CBOR item as far as the record and the backup need: byte strings, unsigned integers, arrays
/// and maps with unsigned keys; anything else is kept as its kind only.
#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    Uint(u64),
    Bytes(Vec<u8>),
    Array(Vec<Item>),
    Map(Vec<(u64, Item)>),
    Other,
}

/// The one CBOR item `bytes` hold; `None` if they hold another number of items or one this
/// reader cannot read.
pub fn item(bytes: &[u8]) -> Option<Item> {
    let (found, rest) = next(bytes)?;
    rest.is_empty().then_some(found)
}

fn next(bytes: &[u8]) -> Option<(Item, &[u8])> {
    let (&first, rest) = bytes.split_first()?;
    let (major, info) = (first >> 5, first & 0x1f);
    let (arg, mut rest) = match info {
        0..=23 => (u64::from(info), rest),
        24..=27 => {
            let len = 1 << (info - 24);
            let (arg, rest) = (rest.get(..len)?, &rest[len..]);
            (arg.iter().fold(0, |n, b| (n << 8) | u64::from(*b)), rest)
        }
        _ => return None,
    };
    let len = usize::try_from(arg).ok();
    let found = match major {
        0 => Item::Uint(arg),
        2 | 3 => {
            let len = len?;
            let value = rest.get(..len)?.to_vec();
            rest = &rest[len..];
            if major == 2 {
                Item::Bytes(value)
            } else {
                Item::Other
            }
        }
        4 => {
            let mut items = Vec::new();
            for _ in 0..arg {
                let (found, after) = next(rest)?;
                items.push(found);
                rest = after;
            }
            Item::Array(items)
        }
        5 => {
            let mut fields = Vec::new();
            for _ in 0..arg {
                let (key, after) = next(rest)?;
                let (value, after) = next(after)?;
                let Item::Uint(key) = key else { return None };
                fields.push((key, value));
                rest = after;
            }
            Item::Map(fields)
        }
        _ => Item::Other,
    };
    Some((found, rest))
}

/// What an account record holds.
#[derive(Debug)]
pub struct Record {
    pub account: Vec<u8>,
    pub master: Vec<u8>,
    pub recovery: Vec<u8>,
    pub revision: u64,
    body: Vec<u8>,
    signature: Vec<u8>,
}

impl Record {
    /// The record `bytes` encode; fails if they encode none.
    #[track_caller]
    pub fn of(bytes: &[u8]) -> Self {
        let Some(Item::Array(parts)) = item(bytes) else {
            panic!("an account record is a CBOR array: {bytes:?}");
        };
        let [Item::Bytes(body), Item::Bytes(signature)] = &parts[..] else {
            panic!("an account record is [body, signature]: {parts:?}");
        };
        let Some(Item::Map(fields)) = item(body) else {
            panic!("an account body is a CBOR map: {body:?}");
        };
        let field = |key| {
            fields
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v.clone())
        };
        let bytes_of = |key| match field(key) {
            Some(Item::Bytes(bytes)) => bytes,
            other => panic!("field {key} of an account body: {other:?}"),
        };
        let Some(Item::Uint(revision)) = field(4) else {
            panic!("an account body's revision: {fields:?}");
        };
        Record {
            account: bytes_of(1),
            master: bytes_of(2),
            recovery: bytes_of(3),
            revision,
            body: body.clone(),
            signature: signature.clone(),
        }
    }

    /// Whether `master` signs the record over its label.
    pub fn signed_by(&self, master: &[u8; 32]) -> bool {
        let Ok(signature) = <[u8; 64]>::try_from(&self.signature[..]) else {
            return false;
        };
        skald_crypto::verify(master, LABEL, &[&self.body], &signature).is_ok()
    }
}

/// The characters of the recovery key `shown`, in its shown form: 3 groups of 4 characters of
/// Crockford's base32 joined by `-`.
#[track_caller]
pub fn key_digits(shown: &str) -> [u8; 12] {
    crate::master_backup::digits(shown)
        .unwrap_or_else(|| panic!("{shown:?} is not a recovery key in its shown form"))
}

/// The recovery HPKE public key the recovery key of the characters `key` derives on `account`, at
/// the cost of this test build.
pub fn recovery_public(key: &[u8; 12], account: AccountId) -> [u8; 32] {
    let (_, ikm) = crate::master_backup::derive_as_built(key, account.as_bytes());
    crate::master_backup::recovery_public(&ikm)
}

/// The master private key the backup `bytes` of `account` opens to with the recovery key of the
/// characters `key`, as "Account" seals it at the cost of this test build; `None` if it does not
/// open.
pub fn open(bytes: &[u8], key: &[u8; 12], account: AccountId) -> Option<[u8; 32]> {
    let Some(Item::Array(parts)) = item(bytes) else {
        panic!("a master backup is a CBOR array: {bytes:?}");
    };
    let [Item::Bytes(nonce), Item::Bytes(ct)] = &parts[..] else {
        panic!("a master backup is [nonce, ct]: {parts:?}");
    };
    let nonce: [u8; 12] = nonce.as_slice().try_into().expect("a 12-byte nonce");
    let (aead, _) = crate::master_backup::derive_as_built(key, account.as_bytes());
    let opened = crate::master_backup::open(ct, &aead, &nonce, account.as_bytes())?;
    opened.try_into().ok()
}

/// The master public key of the private key `seed`.
pub fn master_public(seed: &[u8; 32]) -> [u8; 32] {
    SigningKey::from_bytes(seed).public_key()
}
