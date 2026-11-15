//! An MLS client on mls-rs of the tests' own, apart from `skald-crypto`, for a test device: it
//! joins a channel's group by an external Commit, proposes and gives GroupInfo without the
//! extensions a member's carries, as a device of another implementation, or a hostile one, could.

use mls_rs::{
    CipherSuite, Client, Group, MlsMessage,
    client_builder::MlsConfig,
    crypto::{SignaturePublicKey, SignatureSecretKey},
    identity::{
        SigningIdentity,
        basic::{BasicCredential, BasicIdentityProvider},
    },
};
use mls_rs_crypto_rustcrypto::RustCryptoProvider;

use crate::common::devices::Device;

/// `MLS_128_DHKEMX25519_AES128GCM_SHA256_Ed25519`, the profile's.
const CIPHERSUITE: CipherSuite = CipherSuite::CURVE25519_AES128;

/// A client of `device`: a `basic` credential of its ID, its signing key.
fn client(device: &Device) -> Client<impl MlsConfig + use<>> {
    let public = device.signing.public();
    // mls-rs takes an Ed25519 secret key as the seed, then the public key.
    let seed_and_public = [device.signing.secret.as_slice(), public.as_slice()].concat();
    let leaf = SigningIdentity::new(
        BasicCredential::new(device.id.as_bytes().to_vec()).into_credential(),
        SignaturePublicKey::new(public.to_vec()),
    );
    Client::builder()
        .crypto_provider(RustCryptoProvider::with_enabled_cipher_suites(vec![
            CIPHERSUITE,
        ]))
        .identity_provider(BasicIdentityProvider::new())
        .signing_identity(leaf, SignatureSecretKey::new(seed_and_public), CIPHERSUITE)
        .build()
}

/// `device` joins the group of `group_info`, a GroupInfo in an `MLSMessage`, by an external
/// Commit; gives its group and the Commit.
pub fn joined(device: &Device, group_info: &[u8]) -> (Group<impl MlsConfig + use<>>, Vec<u8>) {
    let group_info = MlsMessage::from_bytes(group_info).expect("a GroupInfo message");
    let (group, commit) = (client(device).external_commit_builder())
        .and_then(|builder| builder.build(group_info))
        .expect("an external Commit of mls-rs");
    (group, commit.to_bytes().expect("the Commit's bytes"))
}

/// A proposal to update the device's leaf, a PublicMessage of the group's epoch.
pub fn proposal<C: MlsConfig>(group: &mut Group<C>) -> Vec<u8> {
    (group.propose_update(Vec::new()))
        .and_then(|message| message.to_bytes())
        .expect("an Update proposal of mls-rs")
}

/// The group's GroupInfo of its epoch, signed by the device, with `ratchet_tree` if `tree` and
/// `external_pub` if `external_pub`.
pub fn group_info<C: MlsConfig>(group: &Group<C>, tree: bool, external_pub: bool) -> Vec<u8> {
    let message = if external_pub {
        group.group_info_message_allowing_ext_commit(tree)
    } else {
        group.group_info_message(tree)
    };
    (message.and_then(|message| message.to_bytes())).expect("a GroupInfo of mls-rs")
}
