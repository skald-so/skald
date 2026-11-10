//! The scenarios each prototype runs, as `docs/dev/mls-prototypes.md`, "Scenarios", lists them.
//! Each one panics on a failed check; [`crate::scenario_tests!`] makes them tests.

use crate::{
    Commit, Device, Fleet, Identity, Leaf, Received, Result,
    identity::device_id,
    wire::{self, Framing},
};

/// The channel ID, a UUIDv7, used as the group's `group_id`.
pub const CHANNEL: &[u8] = &[
    0x01, 0x9a, 0x6e, 0x2c, 0x5b, 0x10, 0x7d, 0x3e, 0x8f, 0x41, 0x92, 0x0c, 0x6d, 0x55, 0x3a, 0x07,
];

/// The leaf a device's KeyPackage gives it: its device ID and public signing key.
pub fn leaf(identity: &Identity) -> Leaf {
    Leaf {
        identity: identity.device_id().to_vec(),
        signature_key: identity.public_key().to_vec(),
    }
}

/// The leaves of the fleet's devices `devices`.
pub fn leaves<D: Device>(fleet: &Fleet<D>, devices: impl IntoIterator<Item = usize>) -> Vec<Leaf> {
    devices
        .into_iter()
        .map(|device| leaf(fleet.identity(device)))
        .collect()
}

/// A channel of `count` devices, made as `docs/protocol/membership.md`, "Creating a channel",
/// says: device 0 creates the group at epoch 0 and commits the Add of every other device from its
/// KeyPackage; the others join from the Welcome. Everyone is at epoch 1. Returns the Commit.
pub fn create_channel<D: Device>(fleet: &mut Fleet<D>, count: usize) -> Result<Commit> {
    let key_packages = (1..count)
        .map(|device| fleet.step(device, |d| d.key_package()))
        .collect::<Result<Vec<_>>>()?;
    fleet.step(0, |d| d.create_group(CHANNEL))?;
    let commit = fleet.step(0, |d| d.add(CHANNEL, &key_packages))?;
    fleet.step(0, |d| d.merge_pending_commit(CHANNEL))?;
    let welcome = commit
        .welcome
        .clone()
        .ok_or("an Add Commit without a Welcome")?;
    for device in 1..count {
        let group_id = fleet.step(device, |d| d.join(&welcome))?;
        if group_id != CHANNEL {
            return Err("joined another group".into());
        }
    }
    Ok(commit)
}

/// A new channel of `count` devices on a new fleet.
pub fn channel<D: Device>(count: usize) -> Fleet<D> {
    let mut fleet = Fleet::new(count).unwrap();
    create_channel(&mut fleet, count).unwrap();
    fleet
}

/// Every device in `devices` is at `epoch`, with the leaves of the devices `members` in leaf order.
#[track_caller]
pub fn assert_group<D: Device>(
    fleet: &mut Fleet<D>,
    devices: impl IntoIterator<Item = usize>,
    epoch: u64,
    members: impl IntoIterator<Item = usize>,
) {
    let members = leaves(fleet, members);
    for device in devices {
        let (got_epoch, got_members) = fleet
            .step(device, |d| Ok((d.epoch(CHANNEL)?, d.members(CHANNEL)?)))
            .unwrap();
        assert_eq!(got_epoch, epoch, "{}: device {device}'s epoch", D::LIBRARY);
        assert_eq!(
            got_members,
            members,
            "{}: device {device}'s members",
            D::LIBRARY
        );
    }
}

/// "Creating a channel" on 3 devices, with the profile's KeyPackages, Commit and Welcome.
pub fn creating_a_channel<D: Device>() {
    let lib = D::LIBRARY;
    let mut fleet = Fleet::<D>::new(3).unwrap();

    let key_packages: Vec<_> = (1..3)
        .map(|device| fleet.step(device, |d| d.key_package()).unwrap())
        .collect();
    for (device, key_package) in (1..3).zip(&key_packages) {
        let identity = fleet.identity(device);
        assert_eq!(
            wire::key_package(key_package).unwrap(),
            wire::KeyPackage {
                cipher_suite: wire::CIPHER_SUITE,
                signature_key: identity.public_key().to_vec(),
                credential_type: wire::BASIC,
                identity: identity.device_id().to_vec(),
            },
            "{lib}: device {device}'s KeyPackage"
        );
    }

    fleet.step(0, |d| d.create_group(CHANNEL)).unwrap();
    assert_group(&mut fleet, [0], 0, [0]);

    let commit = fleet.step(0, |d| d.add(CHANNEL, &key_packages)).unwrap();
    assert_eq!(
        wire::framing(&commit.commit).unwrap(),
        Framing {
            wire_format: wire::PUBLIC_MESSAGE,
            group_id: CHANNEL.to_vec(),
            epoch: 0,
            content_type: wire::COMMIT,
        },
        "{lib}: the Commit is a PublicMessage of epoch 0"
    );
    let welcome = commit.welcome.expect("the Add Commit has a Welcome");
    assert_eq!(
        wire::welcome_cipher_suite(&welcome).unwrap(),
        wire::CIPHER_SUITE,
        "{lib}: the Welcome's ciphersuite"
    );

    // The Commit stays pending in the saved state until the device merges it.
    assert_group(&mut fleet, [0], 0, [0]);
    fleet.step(0, |d| d.merge_pending_commit(CHANNEL)).unwrap();

    for device in 1..3 {
        let group_id = fleet.step(device, |d| d.join(&welcome)).unwrap();
        assert_eq!(
            group_id, CHANNEL,
            "{lib}: device {device} joined the channel"
        );
    }
    assert_group(&mut fleet, 0..3, 1, 0..3);
}

/// A member, not the channel's creator, adds a device; the other members process its Commit.
pub fn member_adds_a_device<D: Device>() {
    let lib = D::LIBRARY;
    let mut fleet = channel::<D>(3);
    let new = fleet.add_device().unwrap();
    let key_package = fleet.step(new, |d| d.key_package()).unwrap();

    let commit = fleet.step(1, |d| d.add(CHANNEL, &[key_package])).unwrap();
    let framing = wire::framing(&commit.commit).unwrap();
    assert_eq!(
        framing.wire_format,
        wire::PUBLIC_MESSAGE,
        "{lib}: the Commit is a PublicMessage"
    );
    assert_eq!(
        (framing.epoch, framing.content_type),
        (1, wire::COMMIT),
        "{lib}: the Commit"
    );
    fleet.step(1, |d| d.merge_pending_commit(CHANNEL)).unwrap();

    for device in [0, 2] {
        let received = fleet
            .step(device, |d| d.process(CHANNEL, &commit.commit))
            .unwrap();
        assert_eq!(
            received,
            Received::Commit {
                sender: leaf(fleet.identity(1))
            },
            "{lib}: device {device} processed device 1's Commit"
        );
    }
    let welcome = commit.welcome.expect("the Add Commit has a Welcome");
    fleet.step(new, |d| d.join(&welcome)).unwrap();
    assert_group(&mut fleet, 0..4, 2, 0..4);
}

/// Every member sends application messages as PrivateMessages; every other member decrypts each
/// one and gets the sender's device ID from its credential.
pub fn members_exchange_messages<D: Device>() {
    let lib = D::LIBRARY;
    let count = 4;
    let mut fleet = channel::<D>(count);

    for round in 0..2 {
        for sender in 0..count {
            let data = format!("message {round} from device {sender}").into_bytes();
            let message = fleet.step(sender, |d| d.encrypt(CHANNEL, &data)).unwrap();
            assert_eq!(
                wire::framing(&message).unwrap(),
                Framing {
                    wire_format: wire::PRIVATE_MESSAGE,
                    group_id: CHANNEL.to_vec(),
                    epoch: 1,
                    content_type: wire::APPLICATION,
                },
                "{lib}: an application message is a PrivateMessage"
            );
            assert!(
                !message.windows(data.len()).any(|window| window == data),
                "{lib}: the message's plaintext is not in it"
            );

            for receiver in (0..count).filter(|&receiver| receiver != sender) {
                let received = fleet
                    .step(receiver, |d| d.process(CHANNEL, &message))
                    .unwrap();
                let Received::Application {
                    sender: from,
                    data: got,
                } = received
                else {
                    panic!(
                        "{lib}: device {receiver} got {received:?}, want an application message"
                    );
                };
                assert_eq!(got, data, "{lib}: device {receiver} decrypted the message");
                assert_eq!(
                    from.identity,
                    fleet.identity(sender).device_id(),
                    "{lib}: device {receiver} got the sender's device ID"
                );
                let key: [u8; 32] = from.signature_key.as_slice().try_into().unwrap();
                assert_eq!(
                    device_id(&key).as_slice(),
                    from.identity,
                    "{lib}: the sender's identity is derived from its signature_key"
                );
            }
        }
    }
}

/// A member rejects an application message it already decrypted, and still decrypts the next one.
pub fn replayed_message_is_rejected<D: Device>() {
    let lib = D::LIBRARY;
    let mut fleet = channel::<D>(2);
    let first = fleet.step(0, |d| d.encrypt(CHANNEL, b"first")).unwrap();
    let second = fleet.step(0, |d| d.encrypt(CHANNEL, b"second")).unwrap();

    let decrypt = |fleet: &mut Fleet<D>, message: &[u8]| {
        fleet
            .step(1, |d| d.process(CHANNEL, message))
            .map(|received| match received {
                Received::Application { data, .. } => data,
                other => panic!("{lib}: got {other:?}, want an application message"),
            })
    };
    assert_eq!(decrypt(&mut fleet, &first).unwrap(), b"first");
    assert!(
        decrypt(&mut fleet, &first).is_err(),
        "{lib}: a replayed message is rejected"
    );
    assert_eq!(decrypt(&mut fleet, &second).unwrap(), b"second");
    assert!(
        decrypt(&mut fleet, &second).is_err(),
        "{lib}: a replayed message is rejected"
    );
}
