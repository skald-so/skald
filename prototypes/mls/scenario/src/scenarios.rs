//! The scenarios each prototype runs, as `docs/dev/mls-prototypes.md`, "Scenarios", lists them.
//! Each one panics on a failed check; [`crate::scenario_tests!`] makes them tests.

use crate::{
    Commit, Device, DeviceId, Fleet, Identity, Leaf, Received, Result, Staged,
    identity::device_id,
    revocation::{self, AccountId},
    wire::{self, Framing, PublicCommit},
};

/// The channel ID, a UUIDv7, used as the group's `group_id`.
pub const CHANNEL: &[u8] = &[
    0x01, 0x9a, 0x6e, 0x2c, 0x5b, 0x10, 0x7d, 0x3e, 0x8f, 0x41, 0x92, 0x0c, 0x6d, 0x55, 0x3a, 0x07,
];

/// The account of every device of a fleet.
pub const ACCOUNT: AccountId = [
    0x01, 0x9a, 0x6e, 0x2c, 0x5a, 0x00, 0x7a, 0x11, 0x8b, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88,
];

/// The `timestamp` of the scenarios' revocations.
pub const REVOKED_AT: u64 = 1_780_000_000;

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

/// The `authenticated_data` of a Commit by `signer` for the revoked devices: their revocations by
/// `signer`, as "Group changes" says.
pub fn revocations<D: Device>(fleet: &Fleet<D>, signer: usize, revoked: &[DeviceId]) -> Vec<u8> {
    let signer = fleet.identity(signer);
    let revocations: Vec<_> = revoked
        .iter()
        .map(|device| revocation::revoke(signer, &ACCOUNT, device, REVOKED_AT))
        .collect();
    revocation::list(&revocations).expect("at least one revocation")
}

/// What a member read from another member's Commit before it merged the Commit: the Commit and the
/// devices the counting revocations in its `authenticated_data` revoke.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Read {
    pub staged: Staged,
    pub revoked: Vec<DeviceId>,
}

/// Device `device` processes another member's Commit, as "Group changes" says: before it merges the
/// Commit, it reads the revocations in the Commit's `authenticated_data` against the leaves of its
/// current epoch. A Commit without `authenticated_data` carries none.
pub fn process_commit<D: Device>(
    fleet: &mut Fleet<D>,
    device: usize,
    commit: &[u8],
) -> Result<Read> {
    fleet.step(device, |d| {
        let members = d.members(CHANNEL)?;
        let mut read = None;
        d.process_with(CHANNEL, commit, |staged| {
            let revoked = if staged.authenticated_data.is_empty() {
                Vec::new()
            } else {
                revocation::revoked(&staged.authenticated_data, &ACCOUNT, &members)?
            };
            read = Some(Read {
                staged: staged.clone(),
                revoked,
            });
            Ok(())
        })?;
        read.ok_or_else(|| "a Commit merged without its check".into())
    })
}

/// Member `committer` commits the Remove of device `removed`, revoked, with its revocation in the
/// Commit's `authenticated_data`, and merges it. Every device in `members` processes the Commit and
/// reads the revocation before it merges. Returns the Commit.
pub fn remove_device<D: Device>(
    fleet: &mut Fleet<D>,
    committer: usize,
    removed: usize,
    members: impl IntoIterator<Item = usize>,
) -> Result<Commit> {
    let lib = D::LIBRARY;
    let id = fleet.identity(removed).device_id();
    let authenticated_data = revocations(fleet, committer, &[id]);
    let commit = fleet.step(committer, |d| {
        d.remove(CHANNEL, &[id.to_vec()], &authenticated_data)
    })?;
    fleet.step(committer, |d| d.merge_pending_commit(CHANNEL))?;
    let want = Read {
        staged: Staged {
            sender: leaf(fleet.identity(committer)),
            removed: vec![leaf(fleet.identity(removed))],
            authenticated_data,
        },
        revoked: vec![id],
    };
    for member in members {
        let read = process_commit(fleet, member, &commit.commit)?;
        if read != want {
            return Err(format!("{lib}: device {member} read {read:?}, want {want:?}").into());
        }
    }
    Ok(commit)
}

/// Device `device`'s history key of the channel's current epoch.
#[track_caller]
pub fn history_key<D: Device>(fleet: &mut Fleet<D>, device: usize) -> crate::HistoryKey {
    fleet.step(device, |d| d.history_key(CHANNEL)).unwrap()
}

/// What a device `identity` gets from `message` when it loads `state` and processes the message.
pub fn process_from<D: Device>(
    identity: &Identity,
    state: &[u8],
    message: &[u8],
) -> Result<Received> {
    D::load(identity, state)?.process(CHANNEL, message)
}

/// A member removes a revoked device: the Remove Commit carries its revocation and starts a new
/// epoch; the removed device, from a full copy of its saved state, reads no later message.
pub fn removed_device_reads_nothing_later<D: Device>() {
    let lib = D::LIBRARY;
    let mut fleet = channel::<D>(4);
    let removed = 3;
    let identity = fleet.identity(removed).clone();
    let copy = fleet.state(removed).to_vec();

    let commit = remove_device(&mut fleet, 1, removed, [0, 2]).unwrap();
    assert!(commit.welcome.is_none(), "{lib}: a Remove has no Welcome");
    let framing = wire::framing(&commit.commit).unwrap();
    assert_eq!(
        (framing.wire_format, framing.epoch, framing.content_type),
        (wire::PUBLIC_MESSAGE, 1, wire::COMMIT),
        "{lib}: the Remove is a PublicMessage Commit of epoch 1"
    );
    assert_eq!(
        wire::public_commit(&commit.commit).unwrap(),
        PublicCommit {
            sender: 1,
            authenticated_data: revocations(&fleet, 1, &[identity.device_id()]),
            removed: vec![3],
            other_proposals: 0,
            path: true,
        },
        "{lib}: the Remove Commit"
    );
    assert_group(&mut fleet, 0..3, 2, 0..3);

    // The removed device learns of its Remove from the Commit; what it saves then is a copy too.
    let mut device = D::load(&identity, &copy).unwrap();
    let mut read = None;
    let received = device
        .process_with(CHANNEL, &commit.commit, |staged| {
            read = Some(staged.removed.clone());
            Ok(())
        })
        .unwrap();
    assert_eq!(
        (received, read),
        (
            Received::Commit {
                sender: leaf(fleet.identity(1))
            },
            Some(vec![leaf(&identity)])
        ),
        "{lib}: the removed device processes its Remove"
    );
    let copies = [copy, device.save().unwrap()];

    // What it sends from its copy, in epoch 1, no member reads.
    let sent = D::load(&identity, &copies[0])
        .unwrap()
        .encrypt(CHANNEL, b"from the copy")
        .unwrap();
    for member in 0..3 {
        assert!(
            fleet.step(member, |d| d.process(CHANNEL, &sent)).is_err(),
            "{lib}: device {member} read the removed device's message"
        );
    }

    // Every later message: application messages of epoch 2, an empty Commit, and one of epoch 3.
    let mut later = Vec::new();
    for sender in 0..3 {
        let data = format!("after the Remove, from device {sender}").into_bytes();
        later.push(fleet.step(sender, |d| d.encrypt(CHANNEL, &data)).unwrap());
    }
    let empty = fleet.step(0, |d| d.empty_commit(CHANNEL, &[])).unwrap();
    fleet.step(0, |d| d.merge_pending_commit(CHANNEL)).unwrap();
    for member in [1, 2] {
        process_commit(&mut fleet, member, &empty.commit).unwrap();
    }
    later.push(empty.commit);
    later.push(fleet.step(2, |d| d.encrypt(CHANNEL, b"epoch 3")).unwrap());
    assert_group(&mut fleet, 0..3, 3, 0..3);

    for (n, message) in later.iter().enumerate() {
        for (which, state) in ["before", "after"].iter().zip(&copies) {
            assert!(
                process_from::<D>(&identity, state, message).is_err(),
                "{lib}: the removed device's copy {which} processing its Remove read message {n}"
            );
        }
    }
}

/// A member reads the revocations a Remove Commit carries before it merges the Commit: a check
/// that fails leaves it in its epoch, and the same Commit merges once the check passes.
pub fn revocations_are_read_before_merging<D: Device>() {
    let lib = D::LIBRARY;
    let mut fleet = channel::<D>(3);
    let revoked = fleet.identity(2).device_id();
    let authenticated_data = revocations(&fleet, 0, &[revoked]);
    let commit = fleet
        .step(0, |d| {
            d.remove(CHANNEL, &[revoked.to_vec()], &authenticated_data)
        })
        .unwrap();
    fleet.step(0, |d| d.merge_pending_commit(CHANNEL)).unwrap();

    let mut device = D::load(fleet.identity(1), fleet.state(1)).unwrap();
    let members = device.members(CHANNEL).unwrap();
    let mut read = None;
    let refused = device.process_with(CHANNEL, &commit.commit, |staged| {
        read = Some(staged.clone());
        Err("refused".into())
    });
    assert!(refused.is_err(), "{lib}: a failed check fails the step");
    assert_eq!(
        read,
        Some(Staged {
            sender: leaf(fleet.identity(0)),
            removed: vec![leaf(fleet.identity(2))],
            authenticated_data: authenticated_data.clone(),
        }),
        "{lib}: the check reads the Commit"
    );
    assert_eq!(
        revocation::revoked(&authenticated_data, &ACCOUNT, &members).unwrap(),
        [revoked],
        "{lib}: the revocation counts"
    );
    assert_eq!(
        (
            device.epoch(CHANNEL).unwrap(),
            device.members(CHANNEL).unwrap()
        ),
        (1, members),
        "{lib}: a refused Commit is not merged"
    );

    let received = device.process(CHANNEL, &commit.commit).unwrap();
    assert_eq!(
        received,
        Received::Commit {
            sender: leaf(fleet.identity(0))
        },
        "{lib}: the Commit merges once its check passes"
    );
    assert_eq!(device.epoch(CHANNEL).unwrap(), 2, "{lib}: the new epoch");
    assert_eq!(
        device.members(CHANNEL).unwrap(),
        leaves(&fleet, 0..2),
        "{lib}: the new epoch's members"
    );
}

/// An empty Commit, with an UpdatePath and no proposals, starts a new epoch with the same members;
/// after a browser's revocation it carries the revocation.
pub fn empty_commit_starts_a_new_epoch<D: Device>() {
    let lib = D::LIBRARY;
    let mut fleet = channel::<D>(3);
    let browser: DeviceId = [0xb0; 16];

    // After a browser's revocation, then on schedule, without revocations.
    for (epoch, committer, authenticated_data, revoked) in [
        (1, 2, revocations(&fleet, 2, &[browser]), vec![browser]),
        (2, 0, Vec::new(), Vec::new()),
    ] {
        let commit = fleet
            .step(committer, |d| d.empty_commit(CHANNEL, &authenticated_data))
            .unwrap();
        assert!(
            commit.welcome.is_none(),
            "{lib}: an empty Commit has no Welcome"
        );
        let framing = wire::framing(&commit.commit).unwrap();
        assert_eq!(
            (framing.wire_format, framing.epoch, framing.content_type),
            (wire::PUBLIC_MESSAGE, epoch, wire::COMMIT),
            "{lib}: the empty Commit is a PublicMessage Commit of epoch {epoch}"
        );
        assert_eq!(
            wire::public_commit(&commit.commit).unwrap(),
            PublicCommit {
                sender: committer as u32,
                authenticated_data: authenticated_data.clone(),
                removed: Vec::new(),
                other_proposals: 0,
                path: true,
            },
            "{lib}: an empty Commit has an UpdatePath and no proposals"
        );
        assert_group(&mut fleet, [committer], epoch, 0..3);
        fleet
            .step(committer, |d| d.merge_pending_commit(CHANNEL))
            .unwrap();

        for member in (0..3).filter(|&member| member != committer) {
            let read = process_commit(&mut fleet, member, &commit.commit).unwrap();
            assert_eq!(
                read,
                Read {
                    staged: Staged {
                        sender: leaf(fleet.identity(committer)),
                        removed: Vec::new(),
                        authenticated_data: authenticated_data.clone(),
                    },
                    revoked: revoked.clone(),
                },
                "{lib}: device {member} read the empty Commit"
            );
        }
        assert_group(&mut fleet, 0..3, epoch + 1, 0..3);
    }
}

/// Every member of an epoch derives the same history key from the MLS exporter; a Remove and an
/// empty Commit each change it, and the removed device cannot derive the new ones.
pub fn history_key_rotates_with_each_epoch<D: Device>() {
    let lib = D::LIBRARY;
    let mut fleet = channel::<D>(3);
    let removed = 2;
    let identity = fleet.identity(removed).clone();

    let epoch_key = |fleet: &mut Fleet<D>, members: &[usize]| {
        let key = history_key(fleet, members[0]);
        for &member in members {
            assert_eq!(
                history_key(fleet, member),
                key,
                "{lib}: device {member} derives the epoch's history key"
            );
        }
        key
    };
    let first = epoch_key(&mut fleet, &[0, 1, 2]);
    // Messages and reloads do not change it.
    let message = fleet
        .step(0, |d| d.encrypt(CHANNEL, b"in epoch 1"))
        .unwrap();
    fleet.step(1, |d| d.process(CHANNEL, &message)).unwrap();
    assert_eq!(epoch_key(&mut fleet, &[0, 1, 2]), first, "{lib}: epoch 1");

    let copy = fleet.state(removed).to_vec();
    let remove = remove_device(&mut fleet, 0, removed, [1]).unwrap();
    let second = epoch_key(&mut fleet, &[0, 1]);
    assert_ne!(second, first, "{lib}: a Remove changes the history key");

    let empty = fleet.step(1, |d| d.empty_commit(CHANNEL, &[])).unwrap();
    fleet.step(1, |d| d.merge_pending_commit(CHANNEL)).unwrap();
    process_commit(&mut fleet, 0, &empty.commit).unwrap();
    let third = epoch_key(&mut fleet, &[0, 1]);
    assert_ne!(
        third, second,
        "{lib}: an empty Commit changes the history key"
    );
    assert_ne!(
        third, first,
        "{lib}: an empty Commit changes the history key"
    );

    // The removed device, from its copy and after it processes what it can, has only epoch 1's key.
    let mut device = D::load(&identity, &copy).unwrap();
    assert_eq!(
        device.history_key(CHANNEL).unwrap(),
        first,
        "{lib}: its copy"
    );
    device.process(CHANNEL, &remove.commit).unwrap();
    assert!(
        device.process(CHANNEL, &empty.commit).is_err(),
        "{lib}: the removed device processes the empty Commit"
    );
    let key = device.history_key(CHANNEL).ok();
    for (new, epoch) in [(second, 2), (third, 3)] {
        assert_ne!(
            key,
            Some(new),
            "{lib}: the removed device derives epoch {epoch}'s history key"
        );
    }
}
