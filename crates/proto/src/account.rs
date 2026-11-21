//! The account record and the master backup: `docs/protocol/membership.md`, "Account".

use crate::cbor::{self, Codec, DecodeError, Item, MapOut, Unknown};
use crate::id::AccountId;
use crate::signed::{Signed, SignedBody};

/// The label of an account record's signature.
pub const ACCOUNT_LABEL: &str = "skald account";

/// An account record, signed by the account's master key over `["skald account", body]`.
pub type AccountRecord = Signed<AccountBody>;

/// What an [`AccountRecord`] signs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountBody {
    pub account: AccountId,
    /// The master public key.
    pub master: [u8; 32],
    /// The recovery HPKE public key.
    pub recovery: [u8; 32],
    /// At least 1.
    pub revision: u64,
    pub unknown: Unknown,
}

impl SignedBody for AccountBody {
    const LABEL: &'static str = ACCOUNT_LABEL;
    const FIELDS: [&'static str; 3] = ["account", "account.body", "account.signature"];
}

impl Codec for AccountBody {
    fn encode(&self) -> Vec<u8> {
        MapOut::new(&self.unknown)
            .put(1, cbor::id(&self.account))
            .put(2, cbor::bytes(&self.master))
            .put(3, cbor::bytes(&self.recovery))
            .put(4, cbor::uint(self.revision))
            .finish()
    }

    fn decode(item: &Item<'_>) -> Result<Self, DecodeError> {
        let mut fields = item.map("account-body")?;
        Ok(Self {
            account: fields
                .required(1, "account-body.account")?
                .id("account-body.account")?,
            master: fields
                .required(2, "account-body.master")?
                .fixed("account-body.master")?,
            recovery: fields
                .required(3, "account-body.recovery")?
                .fixed("account-body.recovery")?,
            revision: fields
                .required(4, "account-body.revision")?
                .positive("account-body.revision")?
                .get(),
            unknown: fields.unknown(),
        })
    }
}

cbor::cbor!(AccountBody);

/// The master private key sealed with the AEAD key of the recovery key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MasterBackup {
    /// The AES-256-GCM nonce.
    pub nonce: [u8; 12],
    /// The 32-byte master private key, sealed: the key and the 16-byte tag.
    pub ct: [u8; 48],
}

impl Codec for MasterBackup {
    fn encode(&self) -> Vec<u8> {
        cbor::array([cbor::bytes(&self.nonce), cbor::bytes(&self.ct)])
    }

    fn decode(item: &Item<'_>) -> Result<Self, DecodeError> {
        let [nonce, ct] = item.tuple("master-backup")?;
        Ok(Self {
            nonce: nonce.fixed("master-backup.nonce")?,
            ct: ct.fixed("master-backup.ct")?,
        })
    }
}

cbor::cbor!(MasterBackup);
