//! What the tests of joining channels share: a channel whose group Bob's device created without
//! Ada's, a device the server lets in that the members do not admit and the external Commit with
//! which it joins, a GroupInfo of a fork, Commits that never reach a device, and what the members
//! read and see of Ada's device joining.

use mls_rs::{
    CipherSuite, Client, MlsMessage,
    crypto::{SignaturePublicKey, SignatureSecretKey},
    identity::{
        SigningIdentity,
        basic::{BasicCredential, BasicIdentityProvider},
    },
};
use mls_rs_crypto_rustcrypto::RustCryptoProvider;
use serde_json::json;
use skald_crypto::{MlsDevice, StateKey};
use skald_github_stand_in::User;
use skald_proto::{Cert, ChannelId, DeviceId, RawEnvelope, Role, SpaceId};

use crate::cloud::Cloud;
use crate::fixtures::{Issuer, Keyed, add_device, hex};
use crate::kit::{
    Change, b64, group_info_on_server, group_info_request, issue_channel, latest, revise,
    warnings_naming,
};
use crate::machine::{Finished, Machine};
use crate::played::{Played, mls_envelope, without_key_packages};
use crate::talk::Talk;
use crate::world::{Person, now};

/// What the CLI's warning says when a Commit removed the device from a channel's group while it
/// was admitted.
pub const REMOVED: &str = "removed";

/// The lifetime of the leaf of a device that joins on mls-rs, within the profile's 60 days.
const LEAF_LIFETIME: std::time::Duration = std::time::Duration::from_secs(30 * 24 * 3_600);

/// The talk of Ada and Bob, an admin, in the space `name`, Ada GitHub user `ids[0]` with
/// `logins[0]` and Bob the next; and another channel, which a revision Bob's device signs adds with
/// Bob and Ada and whose group Bob's device creates alone, as no KeyPackage of Ada's device is left
/// on the server. Ada's machine has not run since: its device is admitted to that channel but not
/// in its group.
pub fn outside(name: &str, ids: &[u64], logins: &[&str]) -> (Talk, ChannelId) {
    let mut talk = Talk::with(name, ids, logins, false, (Role::Admin, |_| {}));
    let space = talk.space.id;
    let channel = bob_adds_channel(&talk.world.cloud, &mut talk.played[0], space, &talk.ada);
    (talk, channel)
}

/// A revision `bob`'s device signs adds a channel of `space` with Bob and Ada, and Bob's device
/// creates its group alone, uploads its GroupInfo and adds no one, as no KeyPackage of Ada's device
/// is left on the server; gives the channel.
pub fn bob_adds_channel(
    cloud: &Cloud,
    bob: &mut Played,
    space: SpaceId,
    ada: &Person,
) -> ChannelId {
    let channel = issue_channel(cloud, &bob.person.token, space);
    let previous = latest(cloud, &bob.person.token, space);
    let change = Change {
        channels: vec![(channel, vec![&bob.person, ada])],
        ..Change::default()
    };
    revise(cloud, &previous, &bob.person, &change);
    without_key_packages(cloud, ada.device);
    bob.creates_alone(cloud, (space, channel));
    bob.uploads_group_info(cloud, (space, channel));
    channel
}

/// A `client` device the server's database holds on `of`'s account, its certificate chained to
/// another master key than the account's, as a server would set up a device of its own: the server
/// lets it in, and the members do not admit it. It connects on `of`'s machine's session.
pub struct Intruder {
    pub keyed: Keyed,
    pub cert: Cert,
    /// The person as the device: `of` with the device's ID and key.
    pub person: Person,
}

impl Intruder {
    pub fn id(&self) -> DeviceId {
        self.keyed.id
    }
}

/// Writes an [`Intruder`] named `name` to the server's database, on `of`'s account.
pub fn intruder(cloud: &Cloud, of: &Person, name: &str) -> Intruder {
    let keyed = add_device(cloud, of.account, name, &Issuer::Server);
    let cert = (cloud.devices(&of.token).into_iter())
        .find(|device| device.id == keyed.id)
        .and_then(|device| device.cert)
        .unwrap_or_else(|| panic!("the certificate of {name}"));
    let person = Person {
        user: User::new(0, name),
        machine: Machine::new(),
        token: of.token.clone(),
        account: of.account,
        master: of.master,
        handle: of.handle.clone(),
        device: keyed.id,
        host: "intruder",
        key: skald_crypto::SigningKey::from_bytes(&keyed.signing.to_bytes()),
    };
    Intruder {
        keyed,
        cert,
        person,
    }
}

/// The external Commit with which `device` joins the group of `group_info`, a GroupInfo in an
/// `MLSMessage`, made on mls-rs as a client of the device would.
pub fn external_commit(device: &Keyed, group_info: &[u8]) -> Vec<u8> {
    let suite = CipherSuite::CURVE25519_AES128;
    let public = device.signing.public_key();
    // mls-rs takes an Ed25519 secret key as the seed, then the public key.
    let key = [device.signing.to_bytes().as_slice(), public.as_slice()].concat();
    let credential = BasicCredential::new(device.id.as_bytes().to_vec());
    let joiner = Client::builder()
        .key_package_lifetime(LEAF_LIFETIME)
        .signing_identity(
            SigningIdentity::new(
                credential.into_credential(),
                SignaturePublicKey::new(public.to_vec()),
            ),
            SignatureSecretKey::new(key),
            suite,
        )
        .identity_provider(BasicIdentityProvider::new())
        .crypto_provider(RustCryptoProvider::with_enabled_cipher_suites(vec![suite]))
        .build();
    let message = MlsMessage::from_bytes(group_info).expect("a GroupInfo message");
    let (_, commit) = joiner
        .commit_external(message)
        .expect("an external Commit of mls-rs");
    commit.to_bytes().expect("the external Commit's bytes")
}

/// The intruder joins `at`'s channel by itself from the GroupInfo the server holds, which it gets
/// as `of`'s session, and sends its external Commit through the server; fails unless the server
/// stores it.
#[track_caller]
pub fn intruder_joins(cloud: &Cloud, intruder: &Intruder, at: (SpaceId, ChannelId)) {
    let group_info = group_info_bytes(cloud, &intruder.person.token, at);
    let commit = external_commit(&intruder.keyed, &group_info);
    let envelope = mls_envelope(at, &commit, &[]);
    crate::peer::Peer::connect(cloud, &intruder.person, &intruder.person.key).stored(&envelope);
}

/// The bytes of the GroupInfo of `at`'s channel the server holds of its current epoch, for the
/// session `token`; fails unless it holds one.
#[track_caller]
pub fn group_info_bytes(cloud: &Cloud, token: &str, at: (SpaceId, ChannelId)) -> Vec<u8> {
    match group_info_on_server(cloud, token, at) {
        (_, Some(group_info)) => group_info,
        other => panic!("no GroupInfo of {}: {other:?}", at.1),
    }
}

/// The GroupInfo of a fork of `channel`'s group, signed by `signer`'s device: a group of its own
/// with the channel's ID that adds the device `listed`, which the members do not admit.
pub fn forked_group_info(signer: &Person, channel: ChannelId, listed: &Intruder) -> Vec<u8> {
    let mut fork = MlsDevice::new(&signer.key, &StateKey::generate());
    let mut device = MlsDevice::new(&listed.person.key, &StateKey::generate());
    let package = device.key_package(now()).expect("a KeyPackage");
    fork.create_group(channel, now())
        .expect("a group of the fork");
    (fork.add_devices(
        channel,
        &[package],
        std::slice::from_ref(&listed.cert),
        now(),
    ))
    .expect("a Commit");
    fork.commit_stored(channel)
        .expect("the fork's Commit merges");
    fork.group_info(channel).expect("the fork's GroupInfo")
}

/// From now on, the relay gives `group_info` as `at`'s channel's GroupInfo to every request for
/// it, as a compromised server would; until [`crate::kit::tell_groups`].
pub fn serve_group_info(cloud: &Cloud, (space, channel): (SpaceId, ChannelId), group_info: &[u8]) {
    let answer = json!({
        "group": true,
        "group_info": b64(group_info),
    })
    .to_string();
    (cloud.relay).rewrite(&group_info_request(space, channel), move |_| answer.clone());
}

/// `played` commits `commit`, which it made in `at`'s channel's group; once the server stores it,
/// the device merges it and uploads the new epoch's GroupInfo. Gives the envelope sent.
#[track_caller]
pub fn kept(
    cloud: &Cloud,
    played: &mut Played,
    at: (SpaceId, ChannelId),
    commit: &[u8],
) -> RawEnvelope {
    let envelope = mls_envelope(at, commit, &[]);
    played.connect(cloud).stored(&envelope);
    (played.mls.commit_stored(at.1)).expect("the stored Commit merges");
    played.uploads_group_info(cloud, at);
    envelope
}

/// The server drops `device`'s mailbox entry of `envelope`, as a proxy that keeps it from the
/// device's catch-up would.
pub fn drop_entry(cloud: &Cloud, device: DeviceId, envelope: &RawEnvelope) {
    cloud.execute(&format!(
        "DELETE FROM entries WHERE device_id = '{device}'::uuid AND envelope_id IN
           (SELECT id FROM envelopes WHERE envelope = '\\x{}'::bytea)",
        hex(envelope.bytes())
    ));
}

/// The warning lines of `finished` that say a Commit removed the device from `channel`'s group.
pub fn removal_warnings(finished: &Finished, channel: ChannelId) -> Vec<String> {
    (warnings_naming(finished, channel).into_iter())
        .filter(|line| line.contains(REMOVED))
        .collect()
}

/// Fails unless the last change of `channel`'s group `played` entered is the external Commit of
/// `joiner`: its sender and the one device it adds, which also removes the joiner's old leaf if
/// `rejoined`, and no other.
#[track_caller]
pub fn assert_joined(played: &Played, channel: ChannelId, joiner: DeviceId, rejoined: bool) {
    let changes = played.changes_of(channel);
    let last = changes.last().unwrap_or_else(|| {
        panic!("{} entered no change of {channel}", played.device());
    });
    let devices = |members: &[skald_crypto::MemberDevice]| -> Vec<DeviceId> {
        members.iter().map(|member| member.device).collect()
    };
    assert_eq!(last.sender, joiner, "the Commit's sender: {last:?}");
    assert_eq!(
        devices(&last.added),
        [joiner],
        "the devices added: {last:?}"
    );
    let old: &[DeviceId] = if rejoined { &[joiner] } else { &[] };
    assert_eq!(devices(&last.removed), old, "the leaves removed: {last:?}");
}

/// The sending devices of the application messages of `channel` that `played` decrypted holding
/// `text`, in order.
pub fn read_from(played: &Played, channel: ChannelId, text: &str) -> Vec<DeviceId> {
    played
        .read_in(channel)
        .into_iter()
        .filter(|(_, read)| String::from_utf8_lossy(&read.plaintext).contains(text))
        .map(|(_, read)| read.sender)
        .collect()
}
