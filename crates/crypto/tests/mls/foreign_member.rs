//! A member of a channel's group on an MLS client outside the crate under test: it joins from the
//! Welcome of the tests' devices and commits whatever mls-rs lets it, so it makes the Commits a
//! member of another implementation, or a hostile one, could send.

use mls_rs::{
    Group, MlsMessage, client_builder::MlsConfig, crypto::SignatureSecretKey, group::CommitBuilder,
    identity::SigningIdentity, time::MlsTime,
};
use skald_crypto::SealedState;
use skald_proto::ChannelId;

use crate::fixtures::{NOW, device_id, signing_key};
use crate::foreign::{self, LIFETIME, SUITE};
use crate::steps::channel_with;

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
    let package = foreign::key_package_of(&client);
    let (states, welcome) = channel_with(channel, ours[0], &ours[1..], &[&package]);
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
    foreign::signing_identity(seed, device_id(device).as_bytes())
}
