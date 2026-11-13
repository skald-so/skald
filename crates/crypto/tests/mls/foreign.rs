//! An MLS client on mls-rs of the tests' own, outside the crate under test: it takes any `basic`
//! credential, so it makes the KeyPackages and Welcomes a device of another implementation, or a
//! hostile one, could send.

use std::time::Duration;

use mls_rs::{
    CipherSuite, Client, ExtensionList, MlsMessage,
    client_builder::MlsConfig,
    crypto::{SignaturePublicKey, SignatureSecretKey},
    identity::{
        SigningIdentity,
        basic::{BasicCredential, BasicIdentityProvider},
    },
};
use mls_rs_crypto_rustcrypto::RustCryptoProvider;
use skald_proto::ChannelId;

use crate::fixtures::signing_key;

/// The profile's ciphersuite, `0x0001`.
const SUITE: CipherSuite = CipherSuite::CURVE25519_AES128;
/// The lifetime of mls-rs's KeyPackages unless its client is given another.
const DEFAULT_LIFETIME: Duration = Duration::from_secs(365 * 24 * 3_600);

/// A client that signs with key `seed` and has a `basic` credential of `identity`, whatever it is,
/// in ciphersuite `suite`, one with Ed25519 signatures; its KeyPackages' lifetime runs `lifetime`
/// from the time they are made at.
pub fn client(
    seed: u8,
    identity: &[u8],
    suite: CipherSuite,
    lifetime: Duration,
) -> Client<impl MlsConfig> {
    let key = signing_key(seed);
    let public = key.public_key();
    // mls-rs takes an Ed25519 secret key as its seed followed by its public key.
    let mut secret = key.to_bytes().to_vec();
    secret.extend_from_slice(&public);
    let credential = BasicCredential::new(identity.to_vec()).into_credential();
    Client::builder()
        .identity_provider(BasicIdentityProvider::new())
        .crypto_provider(RustCryptoProvider::with_enabled_cipher_suites(vec![suite]))
        .signing_identity(
            SigningIdentity::new(credential, SignaturePublicKey::new(public.to_vec())),
            SignatureSecretKey::new(secret),
            suite,
        )
        .key_package_lifetime(lifetime)
        .build()
}

/// A KeyPackage, in an `MLSMessage`, of a client with key `seed` and credential `identity`.
pub fn key_package(seed: u8, identity: &[u8]) -> Vec<u8> {
    client(seed, identity, SUITE, DEFAULT_LIFETIME)
        .generate_key_package_message(ExtensionList::new(), ExtensionList::new(), None)
        .and_then(|message| message.to_bytes())
        .expect("a KeyPackage of mls-rs")
}

/// The Welcome of `channel`'s group, which a client with key `seed` and credential `identity`
/// creates, adds `key_packages` to and commits.
pub fn welcome(seed: u8, identity: &[u8], channel: ChannelId, key_packages: &[&[u8]]) -> Vec<u8> {
    let mut group = client(seed, identity, SUITE, DEFAULT_LIFETIME)
        .create_group_with_id(
            channel.as_bytes().to_vec(),
            ExtensionList::new(),
            ExtensionList::new(),
            None,
        )
        .expect("a group of mls-rs");
    let mut commit = group.commit_builder();
    for package in key_packages {
        let package = MlsMessage::from_bytes(package).expect("a KeyPackage message");
        commit = commit
            .add_member(package)
            .expect("mls-rs adds the KeyPackage");
    }
    let output = commit.build().expect("a Commit of mls-rs");
    group.apply_pending_commit().expect("the Commit merges");
    let [welcome] = output.welcome_messages.as_slice() else {
        panic!("one Welcome");
    };
    welcome.to_bytes().expect("the Welcome's bytes")
}
