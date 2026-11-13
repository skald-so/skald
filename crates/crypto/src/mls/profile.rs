//! The MLS profile of `docs/protocol/membership.md`, "MLS profile", as mls-rs takes it: the one
//! ciphersuite, and leaves whose `basic` credential is the device ID derived from their signature
//! key.

use std::fmt;

use mls_rs::{
    CipherSuite, Client, ExtensionList, IdentityProvider,
    client_builder::MlsConfig,
    crypto::{SignaturePublicKey, SignatureSecretKey},
    error::IntoAnyError,
    identity::{CredentialType, SigningIdentity, basic::BasicCredential},
    time::MlsTime,
};
use mls_rs_core::identity::MemberValidationContext;
use mls_rs_crypto_rustcrypto::RustCryptoProvider;
use skald_proto::DeviceId;

use super::store::Store;
use crate::SigningKey;

/// `MLS_128_DHKEMX25519_AES128GCM_SHA256_Ed25519`, `0x0001`.
const SUITE: CipherSuite = CipherSuite::CURVE25519_AES128;

/// A client of device `id`, which signs with `signing` and keeps its state in `store`. mls-rs
/// takes the Ed25519 key as the 64 bytes of the seed and the public key.
pub(super) fn client(
    signing: &SigningKey,
    id: DeviceId,
    store: &Store,
) -> Client<impl MlsConfig + use<>> {
    let public = signing.public_key();
    let identity = SigningIdentity::new(
        BasicCredential::new(id.as_bytes().to_vec()).into_credential(),
        SignaturePublicKey::new(public.to_vec()),
    );
    let secret = SignatureSecretKey::new([signing.to_bytes().as_slice(), &public].concat());
    Client::builder()
        .crypto_provider(RustCryptoProvider::with_enabled_cipher_suites(vec![SUITE]))
        .identity_provider(Profile)
        .group_state_storage(store.clone())
        .key_package_repo(store.clone())
        .signing_identity(identity, secret, SUITE)
        .build()
}

/// The device ID and the public signing key of a leaf that keeps to the profile.
pub(super) fn device_of(leaf: &SigningIdentity) -> Result<(DeviceId, [u8; 32]), ProfileError> {
    let identity = leaf
        .credential
        .as_basic()
        .ok_or(ProfileError::Credential)?
        .identifier();
    let signing: [u8; 32] = leaf
        .signature_key
        .as_ref()
        .try_into()
        .map_err(|_| ProfileError::SignatureKey)?;
    let device = DeviceId::from_bytes(identity.try_into().map_err(|_| ProfileError::Identity)?);
    if device.is_derived_from(&signing) {
        Ok((device, signing))
    } else {
        Err(ProfileError::Identity)
    }
}

/// The identity provider of the profile. mls-rs asks it of every leaf it validates: a KeyPackage's,
/// each leaf of a tree it joins or a GroupInfo gives, and a Commit's UpdatePath leaf.
#[derive(Debug, Clone, Copy)]
struct Profile;

impl IdentityProvider for Profile {
    type Error = ProfileError;

    fn validate_member(
        &self,
        leaf: &SigningIdentity,
        _: Option<MlsTime>,
        _: MemberValidationContext<'_>,
    ) -> Result<(), ProfileError> {
        device_of(leaf).map(|_| ())
    }

    /// The profile has no external senders.
    fn validate_external_sender(
        &self,
        _: &SigningIdentity,
        _: Option<MlsTime>,
        _: Option<&ExtensionList>,
    ) -> Result<(), ProfileError> {
        Err(ProfileError::ExternalSender)
    }

    fn identity(&self, leaf: &SigningIdentity, _: &ExtensionList) -> Result<Vec<u8>, ProfileError> {
        device_of(leaf).map(|(device, _)| device.as_bytes().to_vec())
    }

    /// A leaf succeeds another only with the same device ID and signature key: so no UpdatePath
    /// changes a member's key, which mls-rs's `BasicIdentityProvider` lets through.
    fn valid_successor(
        &self,
        predecessor: &SigningIdentity,
        successor: &SigningIdentity,
        _: &ExtensionList,
    ) -> Result<bool, ProfileError> {
        Ok(device_of(successor)? == device_of(predecessor)?)
    }

    fn supported_types(&self) -> Vec<CredentialType> {
        vec![CredentialType::BASIC]
    }
}

/// How a leaf breaks the profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProfileError {
    /// The credential is not `basic`.
    Credential,
    /// The signature key is not an Ed25519 public key's 32 bytes.
    SignatureKey,
    /// The `identity` is not the device ID derived from the signature key.
    Identity,
    /// The profile has no external senders.
    ExternalSender,
}

impl fmt::Display for ProfileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Credential => "the credential is not basic",
            Self::SignatureKey => "the signature key is not 32 bytes",
            Self::Identity => "the identity is not the device ID of the signature key",
            Self::ExternalSender => "the profile has no external senders",
        })
    }
}

impl std::error::Error for ProfileError {}

impl IntoAnyError for ProfileError {
    fn into_dyn_error(self) -> Result<Box<dyn std::error::Error + Send + Sync>, Self> {
        Ok(Box::new(self))
    }
}

#[cfg(test)]
mod tests {
    use mls_rs::identity::basic::BasicCredential;

    use super::*;

    fn leaf(identity: &[u8], signing: &[u8]) -> SigningIdentity {
        SigningIdentity::new(
            BasicCredential::new(identity.to_vec()).into_credential(),
            SignaturePublicKey::new(signing.to_vec()),
        )
    }

    fn device(seed: u8) -> (DeviceId, [u8; 32]) {
        let signing = SigningKey::from_bytes(&[seed; 32]).public_key();
        (DeviceId::from_signing_key(&signing), signing)
    }

    fn validate(leaf: &SigningIdentity) -> Result<(), ProfileError> {
        Profile.validate_member(leaf, None, MemberValidationContext::None)
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn a_leaf_of_the_device_id_of_its_key_keeps_to_the_profile() {
        let (id, signing) = device(1);
        let own = leaf(id.as_bytes(), &signing);
        assert_eq!(validate(&own), Ok(()));
        assert_eq!(device_of(&own), Ok((id, signing)));
        assert_eq!(
            Profile.identity(&own, &ExtensionList::new()),
            Ok(id.as_bytes().to_vec())
        );
        assert_eq!(Profile.supported_types(), [CredentialType::BASIC]);
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn a_leaf_that_breaks_the_profile_is_refused() {
        let (id, signing) = device(2);
        let (other, _) = device(3);
        for (leaf, error) in [
            (leaf(other.as_bytes(), &signing), ProfileError::Identity),
            (leaf(&id.as_bytes()[..15], &signing), ProfileError::Identity),
            (
                leaf(&[id.as_bytes().as_slice(), &[0]].concat(), &signing),
                ProfileError::Identity,
            ),
            (
                leaf(id.as_bytes(), &signing[..31]),
                ProfileError::SignatureKey,
            ),
            (
                SigningIdentity::new(
                    mls_rs::identity::Credential::Custom(mls_rs::identity::CustomCredential::new(
                        CredentialType::new(0xf000),
                        id.as_bytes().to_vec(),
                    )),
                    SignaturePublicKey::new(signing.to_vec()),
                ),
                ProfileError::Credential,
            ),
        ] {
            assert_eq!(validate(&leaf), Err(error), "{leaf:?}");
            assert!(Profile.identity(&leaf, &ExtensionList::new()).is_err());
        }
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn a_leaf_succeeds_only_one_of_the_same_device_and_key() {
        let extensions = ExtensionList::new();
        let (id, signing) = device(4);
        let (other_id, other_signing) = device(5);
        let own = leaf(id.as_bytes(), &signing);
        assert_eq!(Profile.valid_successor(&own, &own, &extensions), Ok(true));
        let other = leaf(other_id.as_bytes(), &other_signing);
        assert_eq!(
            Profile.valid_successor(&own, &other, &extensions),
            Ok(false)
        );
        let changed_key = leaf(id.as_bytes(), &other_signing);
        assert!(
            Profile
                .valid_successor(&own, &changed_key, &extensions)
                .is_err()
        );
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn the_profile_has_no_external_senders() {
        let (id, signing) = device(6);
        assert_eq!(
            Profile.validate_external_sender(&leaf(id.as_bytes(), &signing), None, None),
            Err(ProfileError::ExternalSender)
        );
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn each_profile_error_says_how_the_leaf_breaks_it() {
        let errors = [
            ProfileError::Credential,
            ProfileError::SignatureKey,
            ProfileError::Identity,
            ProfileError::ExternalSender,
        ];
        let messages: std::collections::BTreeSet<String> =
            errors.iter().map(ToString::to_string).collect();
        assert_eq!(messages.len(), errors.len(), "{messages:?}");
        assert!(messages.iter().all(|message| !message.is_empty()));
    }
}
