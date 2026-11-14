//! A member of a channel's group on an MLS client outside the crate under test: it joins from the
//! Welcome of the tests' devices and commits whatever mls-rs lets it, so it makes the Commits a
//! member of another implementation, or a hostile one, could send.

use std::time::Duration;

use mls_rs::{
    CipherSuite, ExtensionList, Group, MlsMessage,
    client_builder::MlsConfig,
    crypto::{SignaturePublicKey, SignatureSecretKey},
    group::CommitBuilder,
    identity::{SigningIdentity, basic::BasicCredential},
    time::MlsTime,
};
use skald_crypto::SealedState;
use skald_proto::ChannelId;

use crate::fixtures::{NOW, device_id, signing_key};
use crate::foreign;
use crate::steps::{add, creator, join, stored, with_key_packages};

/// The profile's ciphersuite, `0x0001`.
pub const SUITE: CipherSuite = CipherSuite::CURVE25519_AES128;
/// The longest lifetime of a KeyPackage a device adds: 60 days.
pub const LIFETIME: Duration = Duration::from_secs(60 * 24 * 3_600);

/// `channel`'s group of the tests' devices `ours`, the first its creator, and of a member on mls-rs
/// with device `foreign`'s key and device ID, all in epoch 1: the creator adds the others from
/// their KeyPackages and commits, and each joins from the Welcome. Gives the tests' devices' states,
/// in the order of `ours`, and the foreign member's group.
pub fn with_foreign_member(
    channel: ChannelId,
    ours: &[u8],
    foreign: u8,
) -> (Vec<SealedState>, Group<impl MlsConfig + use<>>) {
    let identity = device_id(foreign);
    let client = foreign::client(foreign, identity.as_bytes(), SUITE, LIFETIME);
    let package = client
        .generate_key_package_message(
            ExtensionList::new(),
            ExtensionList::new(),
            Some(MlsTime::from(NOW)),
        )
        .and_then(|message| message.to_bytes())
        .expect("a KeyPackage of mls-rs");
    let mut states = Vec::new();
    let mut packages = Vec::new();
    for &seed in &ours[1..] {
        let (state, mut made) = with_key_packages(seed, 1);
        states.push(state);
        packages.push(made.remove(0));
    }
    packages.push(package);
    let mut state = creator(ours[0], channel);
    let added: Vec<&[u8]> = packages.iter().map(Vec::as_slice).collect();
    add(ours[0], &mut state, channel, &added);
    let welcome = stored(ours[0], &mut state, channel);
    for (&seed, joiner) in ours[1..].iter().zip(&mut states) {
        assert_eq!(join(seed, joiner, &welcome), channel, "the channel joined");
    }
    states.insert(0, state);
    let welcome = MlsMessage::from_bytes(&welcome).expect("a Welcome message");
    let (group, _) = client
        .join_group(None, &welcome, Some(MlsTime::from(NOW)))
        .expect("the foreign member joins");
    (states, group)
}

/// A Commit of the foreign member's `group` at the tests' time, as `build` makes it; the member
/// drops it, so its next Commit is of the same epoch.
pub fn commit<C: MlsConfig>(
    group: &mut Group<C>,
    build: impl for<'a> FnOnce(CommitBuilder<'a, C>) -> CommitBuilder<'a, C>,
) -> Vec<u8> {
    let output = build(group.commit_builder().commit_time(MlsTime::from(NOW)))
        .build()
        .expect("a Commit of mls-rs");
    group.clear_pending_commit();
    output
        .commit_message
        .to_bytes()
        .expect("the Commit's bytes")
}

/// The leaf index of device `seed` in the foreign member's `group`.
pub fn index_of<C: MlsConfig>(group: &Group<C>, seed: u8) -> u32 {
    let key = signing_key(seed).public_key();
    group
        .roster()
        .members()
        .into_iter()
        .find(|member| member.signing_identity.signature_key.as_bytes() == key)
        .unwrap_or_else(|| panic!("device {seed} is a member"))
        .index
}

/// A new signing identity of device `device` that signs with key `seed`, as an UpdatePath that
/// changes the leaf's key would carry: the same `identity`, another `signature_key`.
pub fn other_key(device: u8, seed: u8) -> (SignatureSecretKey, SigningIdentity) {
    let key = signing_key(seed);
    let public = key.public_key();
    // mls-rs takes an Ed25519 secret key as its seed followed by its public key.
    let mut secret = key.to_bytes().to_vec();
    secret.extend_from_slice(&public);
    let credential = BasicCredential::new(device_id(device).as_bytes().to_vec()).into_credential();
    (
        SignatureSecretKey::new(secret),
        SigningIdentity::new(credential, SignaturePublicKey::new(public.to_vec())),
    )
}
