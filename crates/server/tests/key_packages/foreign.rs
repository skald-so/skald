//! KeyPackages that break the checks of `docs/protocol/membership.md`, "KeyPackages": made by an
//! MLS client on mls-rs of the tests' own, which signs any `basic` credential, in any ciphersuite
//! with Ed25519 signatures and with any lifetime.

use std::time::Duration;

use mls_rs::crypto::{SignaturePublicKey, SignatureSecretKey};
use mls_rs::identity::SigningIdentity;
use mls_rs::identity::basic::{BasicCredential, BasicIdentityProvider};
use mls_rs::time::MlsTime;
use mls_rs::{CipherSuite, Client, ExtensionList};
use mls_rs_crypto_rustcrypto::RustCryptoProvider;
use skald_crypto::SigningKey;
use skald_proto::DeviceId;

/// The profile's ciphersuite, `0x0001`.
pub const PROFILE: CipherSuite = CipherSuite::CURVE25519_AES128;
/// Another ciphersuite with Ed25519 signatures: `0x0003`.
pub const CHACHA: CipherSuite = CipherSuite::CURVE25519_CHACHA;

/// How the tests' client makes a KeyPackage.
pub struct Made<'a> {
    /// The Ed25519 seed it signs with.
    pub seed: &'a [u8; 32],
    /// The `identity` of its `basic` credential.
    pub identity: DeviceId,
    pub suite: CipherSuite,
    /// Its lifetime: from `start`, in seconds since the Unix epoch, for `span`.
    pub start: u64,
    pub span: Duration,
}

/// The KeyPackage `made` says, in an `MLSMessage`.
pub fn key_package(made: &Made<'_>) -> Vec<u8> {
    let public = SigningKey::from_bytes(made.seed).public_key();
    // mls-rs takes an Ed25519 secret key as the seed, then the public key.
    let secret = SignatureSecretKey::new([made.seed.as_slice(), public.as_slice()].concat());
    let credential = BasicCredential::new(made.identity.as_bytes().to_vec()).into_credential();
    let signer = SigningIdentity::new(credential, SignaturePublicKey::new(public.to_vec()));
    let provider = RustCryptoProvider::with_enabled_cipher_suites(vec![made.suite]);
    Client::builder()
        .crypto_provider(provider)
        .identity_provider(BasicIdentityProvider::new())
        .key_package_lifetime(made.span)
        .signing_identity(signer, secret, made.suite)
        .build()
        .generate_key_package_message(
            ExtensionList::new(),
            ExtensionList::new(),
            Some(MlsTime::from(made.start)),
        )
        .and_then(|message| message.to_bytes())
        .expect("a KeyPackage of the tests' client")
}
