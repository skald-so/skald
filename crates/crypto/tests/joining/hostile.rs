//! A joiner on mls-rs outside the crate under test whose identity provider takes any leaf as the
//! successor of any other, so it makes the external Commits that remove another device's leaf, as a
//! hostile holder of the latest GroupInfo could (RFC 9420, §12.2).

use mls_rs::{
    Client, ExtensionList, IdentityProvider, MlsMessage,
    identity::{CredentialType, SigningIdentity, basic::BasicIdentityProvider},
    time::MlsTime,
};
use mls_rs_core::identity::MemberValidationContext;
use mls_rs_crypto_rustcrypto::RustCryptoProvider;

use crate::fixtures::{NOW, device_id};
use crate::foreign::{LIFETIME, SUITE, signing_identity};

/// mls-rs's `basic` provider, but any leaf succeeds any other.
#[derive(Clone)]
struct AnySuccessor(BasicIdentityProvider);

impl IdentityProvider for AnySuccessor {
    type Error = <BasicIdentityProvider as IdentityProvider>::Error;

    fn validate_member(
        &self,
        leaf: &SigningIdentity,
        at: Option<MlsTime>,
        context: MemberValidationContext<'_>,
    ) -> Result<(), Self::Error> {
        self.0.validate_member(leaf, at, context)
    }

    fn validate_external_sender(
        &self,
        sender: &SigningIdentity,
        at: Option<MlsTime>,
        extensions: Option<&ExtensionList>,
    ) -> Result<(), Self::Error> {
        self.0.validate_external_sender(sender, at, extensions)
    }

    fn identity(
        &self,
        leaf: &SigningIdentity,
        extensions: &ExtensionList,
    ) -> Result<Vec<u8>, Self::Error> {
        self.0.identity(leaf, extensions)
    }

    fn valid_successor(
        &self,
        _: &SigningIdentity,
        _: &SigningIdentity,
        _: &ExtensionList,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }

    fn supported_types(&self) -> Vec<CredentialType> {
        self.0.supported_types()
    }
}

/// The external Commit with which device `seed`, with its own key and device ID, joins the group of
/// `group_info` at the tests' time and removes the leaf at `index`, whoever holds it.
pub fn joins_removing(seed: u8, group_info: &[u8], index: u32) -> Vec<u8> {
    let (secret, leaf) = signing_identity(seed, device_id(seed).as_bytes());
    let joiner = Client::builder()
        .identity_provider(AnySuccessor(BasicIdentityProvider::new()))
        .crypto_provider(RustCryptoProvider::with_enabled_cipher_suites(vec![SUITE]))
        .signing_identity(leaf, secret, SUITE)
        .key_package_lifetime(LIFETIME)
        .build();
    let group_info = MlsMessage::from_bytes(group_info).expect("a GroupInfo message");
    let (_, commit) = (joiner.external_commit_builder())
        .and_then(|builder| {
            builder
                .with_removal(index)
                .commit_time(MlsTime::from(NOW))
                .build(group_info)
        })
        .expect("mls-rs makes the external Commit");
    commit.to_bytes().expect("the Commit's bytes")
}
