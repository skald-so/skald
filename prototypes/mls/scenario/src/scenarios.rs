//! The scenarios each prototype runs, as `docs/dev/mls-prototypes.md`, "Scenarios", lists them.
//! Each one panics on a failed check; [`crate::scenario_tests!`] makes them tests.

use std::collections::BTreeMap;

use crate::{
    Admitted, Commit, DeliveryService, Device, DeviceId, Fleet, Identity, Joining, Leaf, Received,
    Refusal, Result, Staged, browser,
    certificate::Kind,
    identity::device_id,
    key_packages,
    revocation::{self, AccountId},
    wire::{self, CommitSender, Framing, PublicCommit},
};

/// The channel ID, a UUIDv7, used as the group's `group_id`.
pub const CHANNEL: &[u8] = &[
    0x01, 0x9a, 0x6e, 0x2c, 0x5b, 0x10, 0x7d, 0x3e, 0x8f, 0x41, 0x92, 0x0c, 0x6d, 0x55, 0x3a, 0x07,
];

/// Another channel's ID.
pub const OTHER_CHANNEL: &[u8] = &[
    0x01, 0x9a, 0x6e, 0x2c, 0x5b, 0x11, 0x7d, 0x3e, 0x8f, 0x41, 0x92, 0x0c, 0x6d, 0x55, 0x3a, 0x08,
];

/// The account of every device of a fleet, the channel's member.
pub const ACCOUNT: AccountId = [
    0x01, 0x9a, 0x6e, 0x2c, 0x5a, 0x00, 0x7a, 0x11, 0x8b, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88,
];

/// An account that is not the channel's member.
pub const OTHER_ACCOUNT: AccountId = [
    0x01, 0x9a, 0x6e, 0x2c, 0x5a, 0x01, 0x7a, 0x11, 0x8b, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x89,
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
    let admitted = fleet.admitted();
    let commit = fleet.step(0, |d| d.add(CHANNEL, &key_packages, &admitted))?;
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
        let fields = wire::key_package(key_package).unwrap();
        assert_eq!(
            (
                fields.cipher_suite,
                fields.signature_key,
                fields.credential_type,
                fields.identity
            ),
            (
                wire::CIPHER_SUITE,
                identity.public_key().to_vec(),
                wire::BASIC,
                identity.device_id().to_vec()
            ),
            "{lib}: device {device}'s KeyPackage"
        );
    }

    fleet.step(0, |d| d.create_group(CHANNEL)).unwrap();
    assert_group(&mut fleet, [0], 0, [0]);

    let admitted = fleet.admitted();
    let commit = fleet
        .step(0, |d| d.add(CHANNEL, &key_packages, &admitted))
        .unwrap();
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

    let admitted = fleet.admitted();
    let commit = fleet
        .step(1, |d| d.add(CHANNEL, &[key_package], &admitted))
        .unwrap();
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

    let received = fleet
        .step(0, |d| d.process(CHANNEL, &commit.commit))
        .unwrap();
    assert_eq!(
        received,
        Received::Commit {
            sender: leaf(fleet.identity(1))
        },
        "{lib}: device 0 processed device 1's Commit"
    );
    // A member sees the added device's credential before it merges the Commit.
    let read = process_commit(&mut fleet, 2, &commit.commit).unwrap();
    let mut want = Read::removal(leaf(fleet.identity(1)), Vec::new(), Vec::new());
    want.staged.added = vec![leaf(fleet.identity(new))];
    assert_eq!(read, want, "{lib}: device 2 read device 1's Commit");

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

/// What a member read from another member's Commit before it merged the Commit: the Commit, the
/// devices the counting revocations in its `authenticated_data` revoke, and the devices it adds
/// that are not admitted, which the member warns its user of ("Admitted devices").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Read {
    pub staged: Staged,
    pub revoked: Vec<DeviceId>,
    pub not_admitted: Vec<Leaf>,
}

impl Read {
    /// What a member reads from a Commit by `sender` that removes `removed` and adds no one.
    pub fn removal(sender: Leaf, removed: Vec<Leaf>, authenticated_data: Vec<u8>) -> Self {
        Self {
            staged: Staged {
                sender,
                removed,
                added: Vec::new(),
                authenticated_data,
            },
            revoked: Vec::new(),
            not_admitted: Vec::new(),
        }
    }
}

/// Device `device` processes another member's Commit, as "Group changes" and "Admitted devices"
/// say: before it merges the Commit, it reads the revocations in the Commit's
/// `authenticated_data` with the fleet's roster, and checks each device the Commit adds. A Commit
/// without `authenticated_data` carries no revocations.
pub fn process_commit<D: Device>(
    fleet: &mut Fleet<D>,
    device: usize,
    commit: &[u8],
) -> Result<Read> {
    let roster = fleet.roster().clone();
    let admitted = roster.admitted();
    fleet.step(device, |d| {
        let mut read = None;
        d.process_with(CHANNEL, commit, |staged| {
            let revoked = if staged.authenticated_data.is_empty() {
                Vec::new()
            } else {
                revocation::revoked(&staged.authenticated_data, &roster)?
            };
            let not_admitted = staged
                .added
                .iter()
                .filter(|leaf| !admitted.admits(leaf))
                .cloned()
                .collect();
            read = Some(Read {
                staged: staged.clone(),
                revoked,
                not_admitted,
            });
            Ok(())
        })?;
        read.ok_or_else(|| "a Commit merged without its check".into())
    })
}

/// Member `device`, after it processed a Commit, commits the Remove of each member that is not
/// admitted and merges it ("Admitted devices"). Returns the Commit, if there was one to make.
pub fn remove_not_admitted<D: Device>(
    fleet: &mut Fleet<D>,
    device: usize,
) -> Result<Option<Commit>> {
    let admitted = fleet.admitted();
    fleet.step(device, |d| {
        let not_admitted: Vec<_> = d
            .members(CHANNEL)?
            .into_iter()
            .filter(|leaf| !admitted.admits(leaf))
            .map(|leaf| leaf.identity)
            .collect();
        if not_admitted.is_empty() {
            return Ok(None);
        }
        let commit = d.remove(CHANNEL, &not_admitted, &[])?;
        d.merge_pending_commit(CHANNEL)?;
        Ok(Some(commit))
    })
}

/// Member `committer` revokes device `removed` and commits its Remove, with the revocation in the
/// Commit's `authenticated_data`, and merges it; the roster gets the revocation. Every device in
/// `members` processes the Commit and reads the revocation before it merges. Returns the Commit.
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
        revoked: vec![id],
        ..Read::removal(
            leaf(fleet.identity(committer)),
            vec![leaf(fleet.identity(removed))],
            authenticated_data,
        )
    };
    for member in members {
        let read = process_commit(fleet, member, &commit.commit)?;
        if read != want {
            return Err(format!("{lib}: device {member} read {read:?}, want {want:?}").into());
        }
    }
    let revocation = revocation::revoke(fleet.identity(committer), &ACCOUNT, &id, REVOKED_AT);
    fleet.roster_mut().add_revocation(revocation);
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
            sender: CommitSender::Member(1),
            authenticated_data: revocations(&fleet, 1, &[identity.device_id()]),
            removed: vec![3],
            external_init: false,
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
            added: Vec::new(),
            authenticated_data: authenticated_data.clone(),
        }),
        "{lib}: the check reads the Commit"
    );
    assert_eq!(
        revocation::revoked(&authenticated_data, fleet.roster()).unwrap(),
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
                sender: CommitSender::Member(committer as u32),
                authenticated_data: authenticated_data.clone(),
                removed: Vec::new(),
                external_init: false,
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
                    revoked: revoked.clone(),
                    ..Read::removal(
                        leaf(fleet.identity(committer)),
                        Vec::new(),
                        authenticated_data.clone(),
                    )
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

/// The fields of `key_package`, changed by `change` and signed again by `signer`: a KeyPackage
/// that fails only the check the change breaks.
pub fn resigned(
    key_package: &[u8],
    signer: &Identity,
    change: impl FnOnce(&mut wire::KeyPackage),
) -> Vec<u8> {
    let mut fields = wire::key_package(key_package).unwrap();
    change(&mut fields);
    fields.sign(signer);
    fields.encode()
}

/// Device `adder` adds the KeyPackage of device `added` to a new group `group_id` it creates;
/// returns the Welcome.
fn add_to_new_group<D: Device>(
    fleet: &mut Fleet<D>,
    adder: usize,
    group_id: &[u8],
    key_package: &[u8],
) -> Vec<u8> {
    let admitted = fleet.admitted();
    fleet
        .step(adder, |d| {
            d.create_group(group_id)?;
            let commit = d.add(group_id, &[key_package.to_vec()], &admitted)?;
            d.merge_pending_commit(group_id)?;
            commit
                .welcome
                .ok_or_else(|| "an Add without a Welcome".into())
        })
        .unwrap()
}

/// Device `sender` sends a message in `group_id`, and device `receiver` decrypts it.
#[track_caller]
fn assert_reads<D: Device>(fleet: &mut Fleet<D>, group_id: &[u8], sender: usize, receiver: usize) {
    let lib = D::LIBRARY;
    let message = fleet
        .step(sender, |d| d.encrypt(group_id, b"hello"))
        .unwrap();
    let received = fleet
        .step(receiver, |d| d.process(group_id, &message))
        .unwrap();
    assert_eq!(
        received,
        Received::Application {
            sender: leaf(fleet.identity(sender)),
            data: b"hello".to_vec()
        },
        "{lib}: device {receiver} reads device {sender} in group {group_id:02x?}"
    );
}

/// A device keeps KeyPackages as "KeyPackages" says: with none unused on the server it makes 20,
/// and a last-resort one; each has the profile's ciphersuite and credential and a lifetime of at
/// most 60 days that holds now, and only the last-resort one has the `last_resort` extension.
/// When 9 remain unused, it makes 11 more. The device keeps the private keys of every one.
pub fn device_keeps_key_packages<D: Device>() {
    let lib = D::LIBRARY;
    let mut fleet = Fleet::<D>::new(3).unwrap();
    let device = 2;
    let identity = fleet.identity(device).clone();
    let make = |fleet: &mut Fleet<D>, count: usize| -> Vec<Vec<u8>> {
        (0..count)
            .map(|_| fleet.step(device, |d| d.key_package()).unwrap())
            .collect()
    };

    let start = key_packages::now();
    let mut server = make(&mut fleet, key_packages::to_upload(0));
    let last_resort = fleet.step(device, |d| d.last_resort_key_package()).unwrap();
    let end = key_packages::now();
    assert_eq!(server.len(), 20, "{lib}: KeyPackages made");

    let mut init_keys = Vec::new();
    for (n, key_package) in server.iter().chain([&last_resort]).enumerate() {
        let fields = wire::key_package(key_package).unwrap();
        assert_eq!(
            (
                fields.cipher_suite,
                fields.credential_type,
                &fields.identity,
                &fields.signature_key
            ),
            (
                wire::CIPHER_SUITE,
                wire::BASIC,
                &identity.device_id().to_vec(),
                &identity.public_key().to_vec()
            ),
            "{lib}: KeyPackage {n}'s profile"
        );
        let lifetime = fields.lifetime;
        let made = lifetime.not_before + key_packages::SKEW;
        assert!(
            (start..=end).contains(&made),
            "{lib}: KeyPackage {n} made at {made}"
        );
        assert_eq!(
            lifetime,
            key_packages::lifetime(made),
            "{lib}: KeyPackage {n}'s lifetime"
        );
        assert_eq!(
            lifetime.not_after - lifetime.not_before,
            key_packages::MAX_LIFETIME,
            "{lib}: KeyPackage {n}'s lifetime is 60 days"
        );
        assert!(
            lifetime.holds(end),
            "{lib}: KeyPackage {n}'s lifetime holds"
        );
        assert!(
            !key_packages::due(&lifetime, end),
            "{lib}: KeyPackage {n} is not due for replacement"
        );
        assert_eq!(
            fields.last_resort(),
            n == 20,
            "{lib}: KeyPackage {n}'s last_resort extension"
        );
        init_keys.push(fields.init_key);
    }
    init_keys.sort();
    init_keys.dedup();
    assert_eq!(
        init_keys.len(),
        21,
        "{lib}: every KeyPackage has its own keys"
    );

    // The server hands out 11; with 9 left, the device makes 11 more.
    let first = server.remove(0);
    server.drain(..10);
    assert_eq!(key_packages::to_upload(server.len()), 11);
    server.extend(make(&mut fleet, key_packages::to_upload(server.len())));
    assert_eq!(key_packages::to_upload(server.len()), 0);
    assert_eq!(server.len(), 20);

    // A KeyPackage of the first upload and one of the second each add the device to a group.
    let newest = server.last().unwrap().clone();
    for (adder, group_id, key_package) in [(0, CHANNEL, &first), (1, OTHER_CHANNEL, &newest)] {
        let welcome = add_to_new_group(&mut fleet, adder, group_id, key_package);
        let joined = fleet.step(device, |d| d.join(&welcome)).unwrap();
        assert_eq!(joined, group_id, "{lib}: the device joined");
        assert_reads(&mut fleet, group_id, adder, device);
    }
}

/// Before an Add, the adding device checks the KeyPackage as "KeyPackages" says. A KeyPackage
/// whose MLS signature does not verify, whose lifetime does not hold, whose ciphersuite is not
/// `0x0001`, or whose `identity` is an admitted device's with another `signature_key`, is refused,
/// alone and next to another device's valid one; the device then adds the valid ones.
pub fn key_packages_that_fail_a_check_are_refused<D: Device>() {
    let lib = D::LIBRARY;
    let mut fleet = channel::<D>(2);
    let new = fleet.add_device().unwrap();
    let peer = fleet.add_device().unwrap();
    let other = fleet.add_device_without_certificate().unwrap();
    let identity = fleet.identity(new).clone();
    let valid = fleet.step(new, |d| d.key_package()).unwrap();
    let peer_valid = fleet.step(peer, |d| d.key_package()).unwrap();

    // Re-encoded and signed again, the device's KeyPackage is the same.
    assert_eq!(
        wire::key_package(&valid).unwrap().encode(),
        valid,
        "{lib}: the KeyPackage re-encoded"
    );
    assert_eq!(
        resigned(&valid, &identity, |_| {}),
        valid,
        "{lib}: the KeyPackage signed again"
    );

    let mut bad_signature = valid.clone();
    let last = bad_signature.len() - 1;
    bad_signature[last] = !bad_signature[last];
    let other_key_package = fleet.step(other, |d| d.key_package()).unwrap();
    let refused = [
        (bad_signature, "a KeyPackage signature that does not verify"),
        (
            resigned(&valid, fleet.identity(other), |_| {}),
            "signatures by a key that is not its signature_key",
        ),
        (
            resigned(&valid, &identity, |kp| {
                kp.lifetime = wire::Lifetime {
                    not_before: 1_700_000_000,
                    not_after: 1_705_184_000,
                }
            }),
            "an expired lifetime",
        ),
        (
            resigned(&valid, &identity, |kp| {
                kp.lifetime = wire::Lifetime {
                    not_before: 4_000_000_000,
                    not_after: 4_005_184_000,
                }
            }),
            "a lifetime that has not begun",
        ),
        (
            resigned(&valid, &identity, |kp| kp.cipher_suite = 0x0003),
            "ciphersuite 0x0003",
        ),
        (
            resigned(&other_key_package, fleet.identity(other), |kp| {
                kp.identity = identity.device_id().to_vec()
            }),
            "an admitted device's identity with another signature_key",
        ),
        (
            resigned(&valid, &identity, |kp| kp.identity = vec![0x1d; 16]),
            "an identity without a certificate",
        ),
    ];

    let admitted = fleet.admitted();
    let commit = fleet
        .step(0, |d| {
            for (key_package, why) in &refused {
                for key_packages in [
                    vec![key_package.clone()],
                    vec![peer_valid.clone(), key_package.clone()],
                ] {
                    assert!(
                        d.add(CHANNEL, &key_packages, &admitted).is_err(),
                        "{lib}: {why} is refused"
                    );
                }
                assert_eq!(d.epoch(CHANNEL)?, 1, "{lib}: {why} leaves the epoch");
            }
            // A refused Add leaves nothing pending: the next Add commits only its KeyPackages.
            let commit = d.add(CHANNEL, &[valid.clone(), peer_valid.clone()], &admitted)?;
            d.merge_pending_commit(CHANNEL)?;
            Ok(commit)
        })
        .unwrap();
    let read = process_commit(&mut fleet, 1, &commit.commit).unwrap();
    assert_eq!(
        read.staged.added,
        leaves(&fleet, [new, peer]),
        "{lib}: the Add's devices"
    );
    let welcome = commit.welcome.expect("the Add Commit has a Welcome");
    for device in [new, peer] {
        fleet.step(device, |d| d.join(&welcome)).unwrap();
    }
    assert_group(&mut fleet, [0, 1, new, peer], 2, [0, 1, new, peer]);
}

/// A KeyPackage, valid as MLS, of a device that is not admitted is refused: one without a
/// certificate, a `client` of an account that is not a member, a `browser` of the member account,
/// an excluded device and a revoked one. A `service` device in the channel's `services` is added.
pub fn devices_that_are_not_admitted_are_refused<D: Device>() {
    let lib = D::LIBRARY;
    let mut fleet = channel::<D>(2);
    let uncertified = fleet.add_device_without_certificate().unwrap();
    let foreign = fleet.add_device_without_certificate().unwrap();
    fleet.certify(foreign, OTHER_ACCOUNT, Kind::Client).unwrap();
    let browser = fleet.add_device_without_certificate().unwrap();
    fleet.certify(browser, ACCOUNT, Kind::Browser).unwrap();
    let excluded = fleet.add_device().unwrap();
    let excluded_id = fleet.identity(excluded).device_id();
    fleet.roster_mut().exclude(ACCOUNT, excluded_id);
    let revoked = fleet.add_device().unwrap();
    let revocation = revocation::revoke(
        fleet.identity(0),
        &ACCOUNT,
        &fleet.identity(revoked).device_id(),
        REVOKED_AT,
    );
    fleet.roster_mut().add_revocation(revocation);
    let service = fleet.add_device_without_certificate().unwrap();
    fleet.certify(service, ACCOUNT, Kind::Service).unwrap();
    let service_id = fleet.identity(service).device_id();
    fleet.roster_mut().add_service(service_id);

    let admitted = fleet.admitted();
    for (device, why) in [
        (uncertified, "a device without a certificate"),
        (foreign, "a client of an account that is not a member"),
        (browser, "a browser"),
        (excluded, "an excluded device"),
        (revoked, "a revoked device"),
    ] {
        let key_package = fleet.step(device, |d| d.key_package()).unwrap();
        assert!(
            fleet
                .step(0, |d| d.add(CHANNEL, &[key_package], &admitted))
                .is_err(),
            "{lib}: {why} is refused"
        );
    }

    let key_package = fleet.step(service, |d| d.key_package()).unwrap();
    let welcome = fleet
        .step(0, |d| {
            let commit = d.add(CHANNEL, &[key_package], &admitted)?;
            d.merge_pending_commit(CHANNEL)?;
            commit
                .welcome
                .ok_or_else(|| "an Add without a Welcome".into())
        })
        .unwrap();
    fleet.step(service, |d| d.join(&welcome)).unwrap();
    assert_group(&mut fleet, [0, service], 2, [0, 1, service]);
}

/// The last-resort KeyPackage adds its device to two groups, and the device joins both; a
/// regular KeyPackage joins it to one group only.
pub fn last_resort_key_package_adds_to_two_groups<D: Device>() {
    let lib = D::LIBRARY;
    let mut fleet = Fleet::<D>::new(3).unwrap();
    let device = 2;
    let last_resort = fleet.step(device, |d| d.last_resort_key_package()).unwrap();
    for (adder, group_id) in [(0, CHANNEL), (1, OTHER_CHANNEL)] {
        let welcome = add_to_new_group(&mut fleet, adder, group_id, &last_resort);
        let joined = fleet.step(device, |d| d.join(&welcome)).unwrap();
        assert_eq!(joined, group_id, "{lib}: the last-resort KeyPackage joined");
        assert_reads(&mut fleet, group_id, device, adder);
    }
    // Its private keys outlive both joins.
    assert_reads(&mut fleet, CHANNEL, 0, device);
    assert_reads(&mut fleet, OTHER_CHANNEL, 1, device);

    let regular = fleet.step(device, |d| d.key_package()).unwrap();
    let groups: [&[u8]; 2] = [&CHANNEL[..15], &OTHER_CHANNEL[..15]];
    let welcomes: Vec<_> = [0, 1]
        .into_iter()
        .zip(groups)
        .map(|(adder, group_id)| add_to_new_group(&mut fleet, adder, group_id, &regular))
        .collect();
    fleet.step(device, |d| d.join(&welcomes[0])).unwrap();
    assert!(
        fleet.step(device, |d| d.join(&welcomes[1])).is_err(),
        "{lib}: a regular KeyPackage joined a second group"
    );
}

/// A member adds a device that is not admitted, as a member with another view of the roster
/// would. Another member sees the added device before it merges the Commit, then commits its
/// Remove; everyone, the removed device too, processes the Remove, and nothing is left to remove.
pub fn device_added_without_admission_is_removed<D: Device>() {
    let lib = D::LIBRARY;
    let mut fleet = channel::<D>(3);
    let outsider = fleet.add_device_without_certificate().unwrap();
    let outsider_leaf = leaf(fleet.identity(outsider));
    let key_package = fleet.step(outsider, |d| d.key_package()).unwrap();

    let view = fleet.admitted().with(&outsider_leaf);
    assert!(!fleet.admitted().admits(&outsider_leaf));
    let commit = fleet
        .step(1, |d| d.add(CHANNEL, &[key_package], &view))
        .unwrap();
    fleet.step(1, |d| d.merge_pending_commit(CHANNEL)).unwrap();
    let welcome = commit.welcome.expect("the Add Commit has a Welcome");
    fleet.step(outsider, |d| d.join(&welcome)).unwrap();

    let read = process_commit(&mut fleet, 0, &commit.commit).unwrap();
    let mut want = Read::removal(leaf(fleet.identity(1)), Vec::new(), Vec::new());
    want.staged.added = vec![outsider_leaf.clone()];
    want.not_admitted = vec![outsider_leaf.clone()];
    assert_eq!(read, want, "{lib}: device 0 read the Add");
    assert_group(&mut fleet, [0, 1, outsider], 2, [0, 1, 2, outsider]);

    let remove = remove_not_admitted(&mut fleet, 0)
        .unwrap()
        .expect("a Remove of the outsider");
    assert_eq!(
        wire::public_commit(&remove.commit).unwrap(),
        PublicCommit {
            sender: CommitSender::Member(0),
            authenticated_data: Vec::new(),
            removed: vec![3],
            external_init: false,
            other_proposals: 0,
            path: true,
        },
        "{lib}: the Remove Commit"
    );
    let want = Read::removal(
        leaf(fleet.identity(0)),
        vec![outsider_leaf.clone()],
        Vec::new(),
    );
    for member in [1, outsider] {
        let read = process_commit(&mut fleet, member, &remove.commit).unwrap();
        assert_eq!(read, want, "{lib}: device {member} read the Remove");
    }
    // Device 2 never saw the outsider admitted: it processes the Add, then the Remove.
    process_commit(&mut fleet, 2, &commit.commit).unwrap();
    process_commit(&mut fleet, 2, &remove.commit).unwrap();
    assert_group(&mut fleet, 0..3, 3, 0..3);
    for member in 0..3 {
        assert!(
            remove_not_admitted(&mut fleet, member).unwrap().is_none(),
            "{lib}: device {member} has no one left to remove"
        );
    }

    let message = fleet.step(2, |d| d.encrypt(CHANNEL, b"after")).unwrap();
    assert!(
        fleet
            .step(outsider, |d| d.process(CHANNEL, &message))
            .is_err(),
        "{lib}: the removed outsider reads a later message"
    );
}

/// A fleet whose devices send their Commits and GroupInfo through a [`DeliveryService`] and
/// process the channel's Commits in the order it accepted them ("Commit ordering").
pub struct Network<D: Device> {
    pub fleet: Fleet<D>,
    pub service: DeliveryService<D::Observer>,
    /// For each device in the channel, how many of the accepted Commits it has merged or processed.
    processed: BTreeMap<usize, usize>,
}

impl<D: Device> Network<D> {
    /// `count` new devices and a service that knows no channel.
    pub fn new(count: usize) -> Self {
        Self {
            fleet: Fleet::new(count).unwrap(),
            service: DeliveryService::default(),
            processed: BTreeMap::new(),
        }
    }

    /// A new channel of `count` devices on a new fleet, its first Commit through the service.
    pub fn channel(count: usize) -> Self {
        let mut network = Self::new(count);
        network.create(count);
        network
    }

    /// Device 0 creates the channel with devices 1 to `count - 1`, as [`create_channel`] does,
    /// and sends its Commit; the other devices join from its Welcome. Returns the Commit.
    pub fn create(&mut self, count: usize) -> Commit {
        let key_packages = (1..count)
            .map(|device| self.fleet.step(device, |d| d.key_package()).unwrap())
            .collect::<Vec<_>>();
        let admitted = self.fleet.admitted();
        let commit = self
            .fleet
            .step(0, |d| {
                d.create_group(CHANNEL)?;
                d.add(CHANNEL, &key_packages, &admitted)
            })
            .unwrap();
        self.send(0, &commit).unwrap();
        let welcome = commit
            .welcome
            .clone()
            .expect("the Add Commit has a Welcome");
        for device in 1..count {
            self.join(device, &welcome);
        }
        commit
    }

    pub fn id(&self, device: usize) -> DeviceId {
        self.fleet.identity(device).device_id()
    }

    /// Device `device` sends `commit`, its pending Commit. If the service accepts it, the device
    /// merges it and uploads its GroupInfo; if the service refuses it, the device drops it.
    pub fn send(&mut self, device: usize, commit: &Commit) -> Result<(), Refusal> {
        let id = self.id(device);
        let sent = self.service.send(&id, &commit.commit);
        if sent.is_ok() {
            self.fleet
                .step(device, |d| d.merge_pending_commit(CHANNEL))
                .unwrap();
            self.service
                .upload_group_info(&id, &commit.group_info)
                .unwrap();
            self.processed
                .insert(device, self.service.commits(CHANNEL).len());
        } else {
            self.fleet
                .step(device, |d| d.clear_pending_commit(CHANNEL))
                .unwrap();
        }
        sent
    }

    /// Device `device` joins from `welcome`, the Welcome of the last Commit the service accepted,
    /// once [`check_joining`] passes on it with the fleet's admitted devices.
    pub fn join(&mut self, device: usize, welcome: &[u8]) {
        let admitted = self.fleet.admitted();
        self.join_with(device, welcome, &admitted);
    }

    /// Device `device` joins from `welcome`, the Welcome of the last Commit the service accepted,
    /// once [`check_joining`] passes on it with `admitted`, the device's view of the roster.
    pub fn join_with(&mut self, device: usize, welcome: &[u8], admitted: &Admitted) {
        let group_id = self
            .fleet
            .step(device, |d| {
                d.join_with(welcome, |joining| check_joining(admitted, joining))
            })
            .unwrap();
        assert_eq!(group_id, CHANNEL, "{}: device {device} joined", D::LIBRARY);
        self.processed
            .insert(device, self.service.commits(CHANNEL).len());
    }

    /// Device `device` commits its external join from the service's latest GroupInfo, as
    /// [`join_external`] does with `admitted`, the device's view of the roster. Returns the
    /// Commit and what the device read of the GroupInfo.
    pub fn external_commit(&mut self, device: usize, admitted: &Admitted) -> (Commit, Joining) {
        let group_info = self
            .service
            .group_info(CHANNEL)
            .expect("a GroupInfo")
            .to_vec();
        let (read, commit) = join_external(&mut self.fleet, device, &group_info, admitted);
        (commit.unwrap(), read.expect("the GroupInfo checked"))
    }

    /// Device `device` sends `commit`, its external Commit. If the service accepts it, the device
    /// uploads its GroupInfo; if the service refuses it, the device drops the group.
    pub fn send_external(&mut self, device: usize, commit: &Commit) -> Result<(), Refusal> {
        let id = self.id(device);
        let sent = self.service.send(&id, &commit.commit);
        if sent.is_ok() {
            self.service
                .upload_group_info(&id, &commit.group_info)
                .unwrap();
            self.processed
                .insert(device, self.service.commits(CHANNEL).len());
        } else {
            self.fleet
                .step(device, |d| d.delete_group(CHANNEL))
                .unwrap();
        }
        sent
    }

    /// Device `device` processes, in their order, the accepted Commits it has not processed, each
    /// as [`process_commit`] does. Returns what it read of each.
    pub fn sync(&mut self, device: usize) -> Vec<Read> {
        let done = self.processed[&device];
        let commits = self.service.commits(CHANNEL)[done..].to_vec();
        commits
            .iter()
            .map(|commit| {
                let read = process_commit(&mut self.fleet, device, commit).unwrap();
                *self.processed.get_mut(&device).unwrap() += 1;
                read
            })
            .collect()
    }

    /// The service and every device in `devices` are at the service's epoch with the leaves of
    /// the devices `members`; the service's GroupInfo is of that epoch; the devices derive one
    /// history key.
    #[track_caller]
    pub fn assert_converged(&mut self, devices: &[usize], members: &[usize]) {
        let lib = D::LIBRARY;
        let epoch = self.service.epoch(CHANNEL).unwrap();
        assert_eq!(
            self.service.members(CHANNEL).unwrap(),
            Some(leaves(&self.fleet, members.iter().copied())),
            "{lib}: the service's members"
        );
        let group_info = wire::group_info(self.service.group_info(CHANNEL).unwrap()).unwrap();
        assert_eq!(
            group_info.epoch, epoch,
            "{lib}: the service's GroupInfo is of its epoch"
        );
        assert_group(
            &mut self.fleet,
            devices.iter().copied(),
            epoch,
            members.iter().copied(),
        );
        let key = history_key(&mut self.fleet, devices[0]);
        for &device in devices {
            assert_eq!(
                history_key(&mut self.fleet, device),
                key,
                "{lib}: device {device}'s history key"
            );
        }
    }
}

/// The fields of `group_info`, changed by `change` and signed again by `signer`.
pub fn resigned_group_info(
    group_info: &[u8],
    signer: &Identity,
    change: impl FnOnce(&mut wire::GroupInfo),
) -> Vec<u8> {
    let mut fields = wire::group_info(group_info).unwrap();
    change(&mut fields);
    fields.sign(signer);
    fields.encode()
}

/// Each Commit comes with the GroupInfo of its new epoch, signed by its sender, with the
/// `ratchet_tree` and `external_pub` extensions; the service keeps the latest. It refuses a
/// GroupInfo of an earlier epoch, from a device that is not a member, or without either extension,
/// and takes one from any member.
pub fn commits_carry_the_new_epochs_group_info<D: Device>() {
    let lib = D::LIBRARY;
    let mut network = Network::<D>::new(3);
    let check = |network: &Network<D>, commit: &Commit, epoch: u64, signer: u32, why: &str| {
        let fields = wire::group_info(&commit.group_info).unwrap();
        assert_eq!(
            (
                fields.cipher_suite,
                &fields.group_id[..],
                fields.epoch,
                fields.signer
            ),
            (wire::CIPHER_SUITE, CHANNEL, epoch, signer),
            "{lib}: {why}'s GroupInfo"
        );
        assert!(
            fields.has(wire::RATCHET_TREE) && fields.has(wire::EXTERNAL_PUB),
            "{lib}: {why}'s GroupInfo has ratchet_tree and external_pub"
        );
        assert_eq!(
            network.service.group_info(CHANNEL),
            Some(&commit.group_info[..]),
            "{lib}: the service keeps {why}'s GroupInfo"
        );
    };

    let create = network.create(3);
    check(&network, &create, 1, 0, "the channel's first Commit");
    network.assert_converged(&[0, 1, 2], &[0, 1, 2]);

    // A member other than the creator adds a device; the new device commits; a member removes one.
    let new = network.fleet.add_device().unwrap();
    let key_package = network.fleet.step(new, |d| d.key_package()).unwrap();
    let admitted = network.fleet.admitted();
    let add = network
        .fleet
        .step(1, |d| d.add(CHANNEL, &[key_package], &admitted))
        .unwrap();
    network.send(1, &add).unwrap();
    check(&network, &add, 2, 1, "an Add");
    network.join(new, &add.welcome.clone().expect("a Welcome"));
    let empty = network
        .fleet
        .step(new, |d| d.empty_commit(CHANNEL, &[]))
        .unwrap();
    network.send(new, &empty).unwrap();
    check(&network, &empty, 3, 3, "the new device's empty Commit");
    network.sync(2);
    let removed = network.id(1).to_vec();
    let remove = network
        .fleet
        .step(2, |d| d.remove(CHANNEL, &[removed], &[]))
        .unwrap();
    network.send(2, &remove).unwrap();
    check(&network, &remove, 4, 2, "a Remove");
    for device in [0, new] {
        network.sync(device);
    }
    network.assert_converged(&[0, 2, new], &[0, 2, new]);

    let latest = remove.group_info.clone();
    let signer = network.fleet.identity(2).clone();
    let without = |extension_type: u16| {
        let group_info = resigned_group_info(&latest, &signer, |fields| {
            fields
                .extensions
                .retain(|extension| extension.extension_type != extension_type)
        });
        let fields = wire::group_info(&group_info).unwrap();
        assert!(
            !fields.has(extension_type) && fields.extensions.len() == 1,
            "{lib}: a GroupInfo without extension {extension_type}"
        );
        group_info
    };
    for (device, group_info, why) in [
        (0, create.group_info.clone(), "a GroupInfo of epoch 1"),
        (
            0,
            empty.group_info.clone(),
            "the GroupInfo of the last epoch",
        ),
        (1, latest.clone(), "a GroupInfo from the removed device"),
        (
            0,
            without(wire::EXTERNAL_PUB),
            "a GroupInfo without external_pub",
        ),
        (
            0,
            without(wire::RATCHET_TREE),
            "a GroupInfo without ratchet_tree",
        ),
    ] {
        let id = network.id(device);
        assert!(
            network.service.upload_group_info(&id, &group_info).is_err(),
            "{lib}: {why} is refused"
        );
        assert_eq!(
            network.service.group_info(CHANNEL),
            Some(&latest[..]),
            "{lib}: {why} leaves the latest GroupInfo"
        );
    }
    assert_eq!(
        resigned_group_info(&latest, &signer, |_| {}),
        latest,
        "{lib}: the GroupInfo signed again"
    );
    let id = network.id(0);
    network.service.upload_group_info(&id, &latest).unwrap();
    assert_eq!(network.service.group_info(CHANNEL), Some(&latest[..]));
}

/// Devices 0 and 1 commit in one epoch: device 0 an Add, device 1 the Remove of device 3. The
/// service accepts device 0's and refuses device 1's with `epoch-mismatch`; device 1 drops it,
/// processes the Add and commits the Remove again. Everyone converges.
pub fn two_commits_in_one_epoch_converge<D: Device>() {
    let lib = D::LIBRARY;
    let mut network = Network::<D>::channel(4);
    let new = network.fleet.add_device().unwrap();
    let key_package = network.fleet.step(new, |d| d.key_package()).unwrap();
    let admitted = network.fleet.admitted();
    let removed = network.id(3).to_vec();
    let add = network
        .fleet
        .step(0, |d| d.add(CHANNEL, &[key_package], &admitted))
        .unwrap();
    let remove = |network: &mut Network<D>| {
        network
            .fleet
            .step(1, |d| {
                d.remove(CHANNEL, std::slice::from_ref(&removed), &[])
            })
            .unwrap()
    };
    let refused = remove(&mut network);

    assert_eq!(network.send(0, &add), Ok(()), "{lib}: the first Commit");
    assert_eq!(
        network.send(1, &refused),
        Err(Refusal::EpochMismatch),
        "{lib}: the second Commit of epoch 1"
    );
    assert!(
        network
            .fleet
            .step(1, |d| d.merge_pending_commit(CHANNEL))
            .is_err(),
        "{lib}: device 1 dropped its refused Commit"
    );
    assert_group(&mut network.fleet, [1], 1, 0..4);
    network.join(new, &add.welcome.clone().expect("a Welcome"));

    let read = network.sync(1);
    assert_eq!(
        read.iter()
            .map(|read| read.staged.added.clone())
            .collect::<Vec<_>>(),
        [vec![leaf(network.fleet.identity(new))]],
        "{lib}: device 1 processed the accepted Add"
    );
    let members = network.fleet.step(1, |d| d.members(CHANNEL)).unwrap();
    assert!(
        members.contains(&leaf(network.fleet.identity(3))),
        "{lib}: device 3 is still a member"
    );
    let again = remove(&mut network);
    assert_eq!(network.send(1, &again), Ok(()), "{lib}: the Remove again");
    for device in [0, 2, 3, new] {
        network.sync(device);
    }
    assert_eq!(
        network.service.commits(CHANNEL)[1..],
        [add.commit, again.commit],
        "{lib}: the accepted Commits"
    );
    network.assert_converged(&[0, 1, 2, new], &[0, 1, 2, new]);
}

/// Devices 0, 1 and 2 commit in one epoch: device 0 an Add, devices 1 and 2 the Remove of device
/// 3. The service accepts device 1's and refuses the others with `epoch-mismatch`. After
/// processing it, device 2's change is no longer needed and device 0 commits its Add again.
pub fn three_commits_in_one_epoch_converge<D: Device>() {
    let lib = D::LIBRARY;
    let mut network = Network::<D>::channel(4);
    let new = network.fleet.add_device().unwrap();
    let new_leaf = leaf(network.fleet.identity(new));
    let key_package = network.fleet.step(new, |d| d.key_package()).unwrap();
    let admitted = network.fleet.admitted();
    let removed = network.id(3).to_vec();
    let add = |network: &mut Network<D>| {
        network
            .fleet
            .step(0, |d| {
                d.add(CHANNEL, std::slice::from_ref(&key_package), &admitted)
            })
            .unwrap()
    };
    let refused_add = add(&mut network);
    let [accepted, refused_remove] = [1, 2].map(|device| {
        network
            .fleet
            .step(device, |d| {
                d.remove(CHANNEL, std::slice::from_ref(&removed), &[])
            })
            .unwrap()
    });

    assert_eq!(
        network.send(1, &accepted),
        Ok(()),
        "{lib}: the first Commit"
    );
    for (device, commit) in [(0, &refused_add), (2, &refused_remove)] {
        assert_eq!(
            network.send(device, commit),
            Err(Refusal::EpochMismatch),
            "{lib}: device {device}'s Commit of epoch 1"
        );
        assert_group(&mut network.fleet, [device], 1, 0..4);
    }
    for device in [0, 2, 3] {
        network.sync(device);
    }
    let removed_leaf = leaf(network.fleet.identity(3));
    for (device, missing) in [(2, &removed_leaf), (0, &new_leaf)] {
        let members = network.fleet.step(device, |d| d.members(CHANNEL)).unwrap();
        assert!(
            !members.contains(missing),
            "{lib}: device {device}'s change"
        );
    }
    let again = add(&mut network);
    assert_eq!(network.send(0, &again), Ok(()), "{lib}: the Add again");
    network.join(new, &again.welcome.clone().expect("a Welcome"));
    for device in [1, 2] {
        network.sync(device);
    }
    assert_eq!(
        network.service.commits(CHANNEL)[1..],
        [accepted.commit, again.commit],
        "{lib}: the accepted Commits"
    );
    network.assert_converged(&[0, 1, 2, new], &[0, 1, 2, new]);
}

/// A Commit for an epoch before the channel's current one gets `epoch-mismatch` and changes
/// nothing at the service: one made before the last accepted Commit, that Commit again, and the
/// channel's first Commit again. The refused device then catches up and commits.
pub fn commit_for_an_old_epoch_is_refused<D: Device>() {
    let lib = D::LIBRARY;
    let mut network = Network::<D>::channel(3);
    let first = network.service.commits(CHANNEL)[0].clone();
    let [stale, accepted] = [1, 0].map(|device| {
        network
            .fleet
            .step(device, |d| d.empty_commit(CHANNEL, &[]))
            .unwrap()
    });
    assert_eq!(network.send(0, &accepted), Ok(()));
    let group_info = network.service.group_info(CHANNEL).unwrap().to_vec();

    assert_eq!(
        network.send(1, &stale),
        Err(Refusal::EpochMismatch),
        "{lib}: a Commit made in epoch 1"
    );
    for (device, commit, why) in [
        (0, &accepted.commit, "the accepted Commit again"),
        (0, &first, "the channel's first Commit again"),
    ] {
        let id = network.id(device);
        assert_eq!(
            network.service.send(&id, commit),
            Err(Refusal::EpochMismatch),
            "{lib}: {why}"
        );
    }
    assert_eq!(network.service.epoch(CHANNEL), Some(2), "{lib}: the epoch");
    assert_eq!(
        network.service.commits(CHANNEL),
        [first, accepted.commit.clone()],
        "{lib}: the accepted Commits"
    );
    assert_eq!(
        network.service.group_info(CHANNEL),
        Some(&group_info[..]),
        "{lib}: the latest GroupInfo"
    );

    network.sync(1);
    let again = network
        .fleet
        .step(1, |d| d.empty_commit(CHANNEL, &[]))
        .unwrap();
    assert_eq!(network.send(1, &again), Ok(()), "{lib}: the Commit again");
    for device in [0, 2] {
        network.sync(device);
    }
    network.assert_converged(&[0, 1, 2], &[0, 1, 2]);
}

/// A Commit for the channel's current epoch gets `not-a-member` unless its sender is a member of
/// that epoch and the device sending it: a device outside the group forks the channel and commits
/// in its own group, in its own name and in device 0's; a member's Commit comes from another
/// device. The service changes nothing, refuses the fork's GroupInfo, and accepts the member's
/// Commit from the member.
pub fn commit_from_a_non_member_is_refused<D: Device>() {
    let lib = D::LIBRARY;
    let mut network = Network::<D>::channel(3);
    let outsider = network.fleet.add_device().unwrap();
    let (fork, forged) = network
        .fleet
        .step(outsider, |d| {
            d.create_group(CHANNEL)?;
            let fork = d.empty_commit(CHANNEL, &[])?;
            d.merge_pending_commit(CHANNEL)?;
            Ok((fork, d.empty_commit(CHANNEL, &[])?))
        })
        .unwrap();
    assert_eq!(
        wire::framing(&forged.commit).unwrap().epoch,
        1,
        "{lib}: the forged Commit is of the channel's epoch"
    );
    let commit = network
        .fleet
        .step(1, |d| d.empty_commit(CHANNEL, &[]))
        .unwrap();
    let group_info = network.service.group_info(CHANNEL).unwrap().to_vec();

    for (device, message, why) in [
        (outsider, &forged.commit, "the fork's Commit"),
        (0, &forged.commit, "the fork's Commit from device 0"),
        (2, &commit.commit, "device 1's Commit from device 2"),
        (
            outsider,
            &commit.commit,
            "device 1's Commit from the outsider",
        ),
    ] {
        let id = network.id(device);
        assert_eq!(
            network.service.send(&id, message),
            Err(Refusal::NotAMember),
            "{lib}: {why}"
        );
    }
    let id = network.id(outsider);
    assert!(
        network
            .service
            .upload_group_info(&id, &fork.group_info)
            .is_err(),
        "{lib}: the fork's GroupInfo is refused"
    );
    assert_eq!(network.service.epoch(CHANNEL), Some(1), "{lib}: the epoch");
    assert_eq!(network.service.commits(CHANNEL).len(), 1);
    assert_eq!(
        network.service.group_info(CHANNEL),
        Some(&group_info[..]),
        "{lib}: the latest GroupInfo"
    );

    assert_eq!(network.send(1, &commit), Ok(()), "{lib}: device 1's Commit");
    for device in [0, 2] {
        network.sync(device);
    }
    network.assert_converged(&[0, 1, 2], &[0, 1, 2]);
}

/// Devices 0 and 1 both create the channel and commit its epoch 0. The service keeps device 0's
/// Commit, refuses device 1's with `epoch-mismatch`, and takes the epoch's GroupInfo only from
/// device 0. Device 1 drops its group and joins the kept one; everyone is in it.
pub fn two_creators_end_in_the_kept_group<D: Device>() {
    let lib = D::LIBRARY;
    let mut network = Network::<D>::new(3);
    let [kp0, kp1, kp2, other_kp2] =
        [0, 1, 2, 2].map(|device| network.fleet.step(device, |d| d.key_package()).unwrap());
    let admitted = network.fleet.admitted();
    let [kept, refused] = [(0, [kp1, kp2]), (1, [kp0, other_kp2])].map(|(device, key_packages)| {
        network
            .fleet
            .step(device, |d| {
                d.create_group(CHANNEL)?;
                d.add(CHANNEL, &key_packages, &admitted)
            })
            .unwrap()
    });
    let [id0, id1] = [0, 1].map(|device| network.id(device));

    assert_eq!(network.service.send(&id0, &kept.commit), Ok(()));
    assert_eq!(
        network.service.send(&id1, &refused.commit),
        Err(Refusal::EpochMismatch),
        "{lib}: the second Commit of epoch 0"
    );
    assert!(
        network
            .service
            .upload_group_info(&id1, &refused.group_info)
            .is_err(),
        "{lib}: the refused group's GroupInfo"
    );
    assert_eq!(network.service.group_info(CHANNEL), None);
    network
        .service
        .upload_group_info(&id0, &kept.group_info)
        .unwrap();
    network
        .fleet
        .step(0, |d| d.merge_pending_commit(CHANNEL))
        .unwrap();
    network.processed.insert(0, 1);

    network.fleet.step(1, |d| d.delete_group(CHANNEL)).unwrap();
    assert!(
        network.fleet.step(1, |d| d.epoch(CHANNEL)).is_err(),
        "{lib}: device 1 dropped its group"
    );
    let welcome = kept.welcome.expect("a Welcome");
    for device in [1, 2] {
        network.join(device, &welcome);
    }
    network.assert_converged(&[0, 1, 2], &[0, 1, 2]);
    assert_reads(&mut network.fleet, CHANNEL, 1, 0);
    assert_reads(&mut network.fleet, CHANNEL, 0, 2);
}

/// The check a device makes of a group before it joins it, by external Commit or from a Welcome
/// ("Group changes"): the GroupInfo's signer and every member it lists are admitted.
pub fn check_joining(admitted: &Admitted, joining: &Joining) -> Result<()> {
    admitted.check(&joining.signer)?;
    joining
        .members
        .iter()
        .try_for_each(|member| admitted.check(member))
}

/// Device `device` joins by external Commit from `group_info` once [`check_joining`] passes on
/// it with `admitted`, the device's view of the roster. Returns what the check read, if the
/// library verified the GroupInfo, and the Commit.
pub fn join_external<D: Device>(
    fleet: &mut Fleet<D>,
    device: usize,
    group_info: &[u8],
    admitted: &Admitted,
) -> (Option<Joining>, Result<Commit>) {
    let mut read = None;
    let commit = fleet.step(device, |d| {
        d.join_external(group_info, |joining| {
            read = Some(joining.clone());
            check_joining(admitted, joining)
        })
    });
    (read, commit)
}

/// Device `device` joins from `welcome` once [`check_joining`] passes on it with the fleet's
/// admitted devices. Returns what the check read, if the library got that far, and the result.
pub fn join_from_welcome<D: Device>(
    fleet: &mut Fleet<D>,
    device: usize,
    welcome: &[u8],
) -> (Option<Joining>, Result<Vec<u8>>) {
    let admitted = fleet.admitted();
    let mut read = None;
    let joined = fleet.step(device, |d| {
        d.join_with(welcome, |joining| {
            read = Some(joining.clone());
            check_joining(&admitted, joining)
        })
    });
    (read, joined)
}

/// What a member reads from the external Commit of `joiner`, which removes `removed`: the joiner,
/// its new leaf, is the sender and the one device added.
pub fn external_read(joiner: Leaf, removed: Vec<Leaf>, not_admitted: Vec<Leaf>) -> Read {
    Read {
        staged: Staged {
            sender: joiner.clone(),
            removed,
            added: vec![joiner],
            authenticated_data: Vec::new(),
        },
        revoked: Vec::new(),
        not_admitted,
    }
}

/// An admitted device outside the group joins by itself ("Group changes"): it checks the latest
/// GroupInfo's signer and members and joins with an external Commit, which the service accepts
/// only from it. Members check the joiner as an Add. A second joiner's external Commit from the
/// same GroupInfo gets `epoch-mismatch`; it drops the group and joins from the latest GroupInfo.
pub fn device_joins_by_itself<D: Device>() {
    let lib = D::LIBRARY;
    let mut network = Network::<D>::channel(3);
    // Device 1 commits, so the latest GroupInfo is not the creator's.
    let empty = network
        .fleet
        .step(1, |d| d.empty_commit(CHANNEL, &[]))
        .unwrap();
    network.send(1, &empty).unwrap();
    for member in [0, 2] {
        network.sync(member);
    }
    let [joiner, late] = [0, 1].map(|_| network.fleet.add_device().unwrap());
    let admitted = network.fleet.admitted();

    let (commit, read) = network.external_commit(joiner, &admitted);
    assert_eq!(
        read,
        Joining {
            signer: leaf(network.fleet.identity(1)),
            members: leaves(&network.fleet, 0..3),
        },
        "{lib}: the joiner read the GroupInfo"
    );
    assert!(
        commit.welcome.is_none(),
        "{lib}: an external Commit's Welcome"
    );
    assert_eq!(
        wire::framing(&commit.commit).unwrap(),
        Framing {
            wire_format: wire::PUBLIC_MESSAGE,
            group_id: CHANNEL.to_vec(),
            epoch: 2,
            content_type: wire::COMMIT,
        },
        "{lib}: the external Commit is a PublicMessage of epoch 2"
    );
    assert_eq!(
        wire::public_commit(&commit.commit).unwrap(),
        PublicCommit {
            sender: CommitSender::NewMember,
            authenticated_data: Vec::new(),
            removed: Vec::new(),
            external_init: true,
            other_proposals: 0,
            path: true,
        },
        "{lib}: the external Commit"
    );
    assert_group(&mut network.fleet, [joiner], 3, 0..4);
    let (stale, _) = network.external_commit(late, &admitted);

    // Only the joiner sends its external Commit.
    for device in [0, late] {
        let id = network.id(device);
        assert_eq!(
            network.service.send(&id, &commit.commit),
            Err(Refusal::NotAMember),
            "{lib}: the joiner's Commit from device {device}"
        );
    }
    assert_eq!(network.service.epoch(CHANNEL), Some(2), "{lib}: the epoch");
    assert_eq!(
        network.send_external(joiner, &commit),
        Ok(()),
        "{lib}: the joiner's Commit"
    );
    let group_info = wire::group_info(network.service.group_info(CHANNEL).unwrap()).unwrap();
    assert_eq!(
        (group_info.epoch, group_info.signer),
        (3, 3),
        "{lib}: the service keeps the joiner's GroupInfo"
    );
    assert_eq!(
        network.send_external(late, &stale),
        Err(Refusal::EpochMismatch),
        "{lib}: an external Commit of epoch 2"
    );
    assert!(
        network.fleet.step(late, |d| d.epoch(CHANNEL)).is_err(),
        "{lib}: the refused joiner dropped the group"
    );

    let joiner_leaf = leaf(network.fleet.identity(joiner));
    for member in 0..3 {
        assert_eq!(
            network.sync(member),
            [external_read(joiner_leaf.clone(), Vec::new(), Vec::new())],
            "{lib}: device {member} read the external Commit"
        );
    }
    let (again, _) = network.external_commit(late, &admitted);
    assert_eq!(
        network.send_external(late, &again),
        Ok(()),
        "{lib}: the second joiner's Commit from the latest GroupInfo"
    );
    for device in [0, 1, 2, joiner] {
        network.sync(device);
    }
    network.assert_converged(&[0, 1, 2, joiner, late], &[0, 1, 2, joiner, late]);
    assert_reads(&mut network.fleet, CHANNEL, joiner, 0);
    assert_reads(&mut network.fleet, CHANNEL, 2, late);
}

/// A device that is not admitted joins by itself, as anyone with the latest GroupInfo can. Members
/// check it as an Add, find it not admitted and remove it ("Admitted devices").
pub fn joiner_that_is_not_admitted_is_removed<D: Device>() {
    let lib = D::LIBRARY;
    let mut network = Network::<D>::channel(3);
    let outsider = network.fleet.add_device_without_certificate().unwrap();
    let outsider_leaf = leaf(network.fleet.identity(outsider));
    let view = network.fleet.admitted().with(&outsider_leaf);
    let (commit, _) = network.external_commit(outsider, &view);
    assert_eq!(
        network.send_external(outsider, &commit),
        Ok(()),
        "{lib}: the outsider's external Commit"
    );

    let want = external_read(
        outsider_leaf.clone(),
        Vec::new(),
        vec![outsider_leaf.clone()],
    );
    assert_eq!(
        network.sync(0),
        [want],
        "{lib}: device 0 read the external Commit"
    );
    let id = network.id(outsider).to_vec();
    let remove = network
        .fleet
        .step(0, |d| d.remove(CHANNEL, &[id], &[]))
        .unwrap();
    assert_eq!(network.send(0, &remove), Ok(()), "{lib}: the Remove");
    for device in [1, 2, outsider] {
        network.sync(device);
    }
    network.assert_converged(&[0, 1, 2], &[0, 1, 2]);
    for member in 0..3 {
        assert!(
            remove_not_admitted(&mut network.fleet, member)
                .unwrap()
                .is_none(),
            "{lib}: device {member} has no one left to remove"
        );
    }
    let message = network
        .fleet
        .step(2, |d| d.encrypt(CHANNEL, b"after"))
        .unwrap();
    assert!(
        network
            .fleet
            .step(outsider, |d| d.process(CHANNEL, &message))
            .is_err(),
        "{lib}: the removed outsider reads a later message"
    );
}

/// A device joining by itself refuses a GroupInfo whose signer is not a member, signed by another
/// key than its signer leaf's or for a leaf that does not exist, or not admitted, and one that
/// lists a member that is not admitted; it joins nothing. It joins from one that passes.
pub fn group_info_that_fails_a_check_is_refused<D: Device>() {
    let lib = D::LIBRARY;
    let mut network = Network::<D>::channel(3);
    let outsider = network.fleet.add_device_without_certificate().unwrap();
    let joiner = network.fleet.add_device().unwrap();
    let outsider_identity = network.fleet.identity(outsider).clone();
    let outsider_leaf = leaf(&outsider_identity);
    let admitted = network.fleet.admitted();

    // Device 1 adds the outsider, as a member with another view of the roster would; the outsider
    // commits; device 0 removes it.
    let view = admitted.clone().with(&outsider_leaf);
    let key_package = network.fleet.step(outsider, |d| d.key_package()).unwrap();
    let add = network
        .fleet
        .step(1, |d| d.add(CHANNEL, &[key_package], &view))
        .unwrap();
    network.send(1, &add).unwrap();
    network.join_with(outsider, &add.welcome.clone().expect("a Welcome"), &view);
    let by_outsider = network
        .fleet
        .step(outsider, |d| d.empty_commit(CHANNEL, &[]))
        .unwrap();
    network.send(outsider, &by_outsider).unwrap();
    network.sync(0);
    let id = network.id(outsider).to_vec();
    let remove = network
        .fleet
        .step(0, |d| d.remove(CHANNEL, &[id], &[]))
        .unwrap();
    network.send(0, &remove).unwrap();
    let latest = remove.group_info.clone();

    let with_outsider: Vec<_> = leaves(&network.fleet, [0, 1, 2, outsider]);
    for (group_info, want, why) in [
        (
            add.group_info.clone(),
            Some(Joining {
                signer: leaf(network.fleet.identity(1)),
                members: with_outsider.clone(),
            }),
            "a GroupInfo that lists a member that is not admitted",
        ),
        (
            by_outsider.group_info.clone(),
            Some(Joining {
                signer: outsider_leaf.clone(),
                members: with_outsider.clone(),
            }),
            "a GroupInfo signed by a member that is not admitted",
        ),
        (
            resigned_group_info(&latest, &outsider_identity, |_| {}),
            None,
            "a GroupInfo signed by another key than its signer leaf's",
        ),
        (
            resigned_group_info(&latest, &outsider_identity, |fields| fields.signer = 9),
            None,
            "a GroupInfo signed for a leaf that does not exist",
        ),
    ] {
        let (read, commit) = join_external(&mut network.fleet, joiner, &group_info, &admitted);
        assert!(commit.is_err(), "{lib}: {why} is refused");
        assert_eq!(read, want, "{lib}: what the joiner read of {why}");
        assert!(
            network.fleet.step(joiner, |d| d.epoch(CHANNEL)).is_err(),
            "{lib}: {why} leaves the joiner without the group"
        );
    }

    let (commit, read) = network.external_commit(joiner, &admitted);
    assert_eq!(
        read,
        Joining {
            signer: leaf(network.fleet.identity(0)),
            members: leaves(&network.fleet, 0..3),
        },
        "{lib}: the joiner read the latest GroupInfo"
    );
    assert_eq!(network.send_external(joiner, &commit), Ok(()));
    for device in [0, 1, 2] {
        network.sync(device);
    }
    network.assert_converged(&[0, 1, 2, joiner], &[0, 1, 2, joiner]);
}

/// A device that gets a Welcome checks its signer and members as a GroupInfo ("Group changes"):
/// it refuses one whose group lists a member that is not admitted, and one from a device that is
/// not admitted, in a group of its own with the channel's ID; it joins nothing. It joins from one
/// that passes.
pub fn welcome_that_fails_a_check_is_refused<D: Device>() {
    let lib = D::LIBRARY;
    let mut fleet = channel::<D>(3);
    let outsider = fleet.add_device_without_certificate().unwrap();
    let joiner = fleet.add_device().unwrap();
    let outsider_leaf = leaf(fleet.identity(outsider));
    let joiner_leaf = leaf(fleet.identity(joiner));
    let [outsider_kp, joiner_kp] =
        [outsider, joiner].map(|device| fleet.step(device, |d| d.key_package()).unwrap());
    let view = fleet.admitted().with(&outsider_leaf);

    // Device 1, with another view of the roster, adds both; it never sends the Commit.
    let listing = fleet
        .step(1, |d| {
            let commit = d.add(CHANNEL, &[outsider_kp, joiner_kp.clone()], &view)?;
            d.clear_pending_commit(CHANNEL)?;
            commit
                .welcome
                .ok_or_else(|| "an Add without a Welcome".into())
        })
        .unwrap();
    let forged = add_to_new_group(&mut fleet, outsider, CHANNEL, &joiner_kp);

    let mut with_outsider = leaves(&fleet, [0, 1, 2, outsider]);
    with_outsider.push(joiner_leaf.clone());
    for (welcome, want, why) in [
        (
            listing,
            Joining {
                signer: leaf(fleet.identity(1)),
                members: with_outsider,
            },
            "a Welcome to a group with a member that is not admitted",
        ),
        (
            forged,
            Joining {
                signer: outsider_leaf.clone(),
                members: vec![outsider_leaf.clone(), joiner_leaf.clone()],
            },
            "a Welcome from a device that is not admitted",
        ),
    ] {
        let (read, joined) = join_from_welcome(&mut fleet, joiner, &welcome);
        assert!(joined.is_err(), "{lib}: {why} is refused");
        assert_eq!(read, Some(want), "{lib}: what the joiner read of {why}");
        assert!(
            fleet.step(joiner, |d| d.epoch(CHANNEL)).is_err(),
            "{lib}: {why} leaves the joiner without the group"
        );
    }

    let admitted = fleet.admitted();
    let commit = fleet
        .step(0, |d| {
            let commit = d.add(CHANNEL, &[joiner_kp], &admitted)?;
            d.merge_pending_commit(CHANNEL)?;
            Ok(commit)
        })
        .unwrap();
    let (read, joined) = join_from_welcome(&mut fleet, joiner, &commit.welcome.expect("a Welcome"));
    assert_eq!(joined.unwrap(), CHANNEL, "{lib}: the joiner joined");
    let mut members = leaves(&fleet, 0..3);
    members.push(joiner_leaf);
    assert_eq!(
        read,
        Some(Joining {
            signer: leaf(fleet.identity(0)),
            members,
        }),
        "{lib}: what the joiner read of the Welcome"
    );
    for member in [1, 2] {
        process_commit(&mut fleet, member, &commit.commit).unwrap();
    }
    assert_group(&mut fleet, [0, 1, 2, joiner], 2, [0, 1, 2, joiner]);
    assert_reads(&mut fleet, CHANNEL, joiner, 1);
}

/// A device that missed a Commit fails on the next one. It rejoins with an external Commit that
/// also removes its old leaf ("Rejoining"): members read the Commit as its Remove and its Add, and
/// it holds one leaf.
pub fn device_that_missed_a_commit_rejoins<D: Device>() {
    let lib = D::LIBRARY;
    let mut network = Network::<D>::channel(3);
    for committer in [0, 1] {
        network.sync(committer);
        let commit = network
            .fleet
            .step(committer, |d| d.empty_commit(CHANNEL, &[]))
            .unwrap();
        network.send(committer, &commit).unwrap();
    }
    // Device 2 missed device 0's Commit and gets device 1's.
    let next = network.service.commits(CHANNEL)[2].clone();
    assert!(
        process_commit(&mut network.fleet, 2, &next).is_err(),
        "{lib}: device 2 processed the Commit after the one it missed"
    );
    assert_group(&mut network.fleet, [2], 1, 0..3);
    network.sync(0);

    let admitted = network.fleet.admitted();
    let (commit, read) = network.external_commit(2, &admitted);
    assert_eq!(
        read,
        Joining {
            signer: leaf(network.fleet.identity(1)),
            members: leaves(&network.fleet, 0..3),
        },
        "{lib}: device 2 read the latest GroupInfo"
    );
    assert_eq!(
        wire::public_commit(&commit.commit).unwrap(),
        PublicCommit {
            sender: CommitSender::NewMember,
            authenticated_data: Vec::new(),
            removed: vec![2],
            external_init: true,
            other_proposals: 0,
            path: true,
        },
        "{lib}: the external Commit removes the old leaf"
    );
    assert_eq!(
        network.send_external(2, &commit),
        Ok(()),
        "{lib}: the rejoin"
    );
    let own = leaf(network.fleet.identity(2));
    for member in [0, 1] {
        assert_eq!(
            network.sync(member),
            [external_read(own.clone(), vec![own.clone()], Vec::new())],
            "{lib}: device {member} read the rejoin"
        );
    }
    network.assert_converged(&[0, 1, 2], &[0, 1, 2]);
    assert_reads(&mut network.fleet, CHANNEL, 2, 0);
    assert_reads(&mut network.fleet, CHANNEL, 1, 2);
}

/// The Commits a device that was offline misses in [`offline_device_catches_up`].
pub const MISSED_COMMITS: usize = 100;

/// A device offline for [`MISSED_COMMITS`] Commits, among them an Add and a Remove, processes each
/// in order, then reads and sends new messages ("Rejoining").
pub fn offline_device_catches_up<D: Device>() {
    let lib = D::LIBRARY;
    let mut network = Network::<D>::channel(3);
    let visitor = network.fleet.add_device().unwrap();
    let visitor_leaf = leaf(network.fleet.identity(visitor));
    let (added, removed) = (40, 70);
    let admitted = network.fleet.admitted();
    for n in 0..MISSED_COMMITS {
        let committer = n % 2;
        network.sync(committer);
        let commit = if n == added {
            let key_package = network.fleet.step(visitor, |d| d.key_package()).unwrap();
            network
                .fleet
                .step(committer, |d| d.add(CHANNEL, &[key_package], &admitted))
        } else if n == removed {
            let id = network.id(visitor).to_vec();
            network
                .fleet
                .step(committer, |d| d.remove(CHANNEL, &[id], &[]))
        } else {
            network
                .fleet
                .step(committer, |d| d.empty_commit(CHANNEL, &[]))
        }
        .unwrap();
        network.send(committer, &commit).unwrap();
        if let Some(welcome) = &commit.welcome {
            network.join(visitor, welcome);
        }
    }

    let read = network.sync(2);
    assert_eq!(
        read.len(),
        MISSED_COMMITS,
        "{lib}: the Commits device 2 read"
    );
    for (n, read) in read.iter().enumerate() {
        let committer = leaf(network.fleet.identity(n % 2));
        let (added, removed) = match n {
            n if n == added => (vec![visitor_leaf.clone()], Vec::new()),
            n if n == removed => (Vec::new(), vec![visitor_leaf.clone()]),
            _ => (Vec::new(), Vec::new()),
        };
        assert_eq!(
            (
                &read.staged.sender,
                &read.staged.added,
                &read.staged.removed
            ),
            (&committer, &added, &removed),
            "{lib}: missed Commit {n}"
        );
    }
    network.sync(0);
    network.assert_converged(&[0, 1, 2], &[0, 1, 2]);
    assert_reads(&mut network.fleet, CHANNEL, 0, 2);
    assert_reads(&mut network.fleet, CHANNEL, 2, 1);
}

/// The ID and the signing key of each of the fleet's devices `devices`, one after another: the
/// admitted devices as [`browser::member_ids`] takes them.
fn encoded_admitted<D: Device>(
    fleet: &Fleet<D>,
    devices: impl IntoIterator<Item = usize>,
) -> Vec<u8> {
    devices
        .into_iter()
        .flat_map(|device| {
            let identity = fleet.identity(device);
            [
                identity.device_id().to_vec(),
                identity.public_key().to_vec(),
            ]
            .concat()
        })
        .collect()
}

/// A linked browser takes the channel's member devices from its latest GroupInfo with the ratchet
/// tree, signed by a member leaf ("Envelope"), and checks them before it writes under a wrap's key
/// ("Wraps"): after the channel's first Commit, and after a Remove, for a wrap by any member.
pub fn browser_reads_the_members_from_the_group_info<D: Device>() {
    let lib = D::LIBRARY;
    let mut network = Network::<D>::channel(3);
    let admitted = network.fleet.admitted();
    let first = network.service.group_info(CHANNEL).unwrap().to_vec();
    for signer in 0..3 {
        assert_eq!(
            browser::members::<D::Observer>(&first, CHANNEL, 1, &network.id(signer), &admitted)
                .unwrap(),
            leaves(&network.fleet, 0..3),
            "{lib}: the members of epoch 1 for a wrap by device {signer}"
        );
    }

    let id = network.id(1).to_vec();
    let remove = network
        .fleet
        .step(2, |d| d.remove(CHANNEL, &[id], &[]))
        .unwrap();
    network.send(2, &remove).unwrap();
    let latest = network.service.group_info(CHANNEL).unwrap().to_vec();
    assert_eq!(
        wire::group_info(&latest).unwrap().signer,
        2,
        "{lib}: device 2's leaf signed the latest GroupInfo"
    );
    for signer in [0, 2] {
        assert_eq!(
            browser::members::<D::Observer>(&latest, CHANNEL, 2, &network.id(signer), &admitted)
                .unwrap(),
            leaves(&network.fleet, [0, 2]),
            "{lib}: the members of epoch 2 for a wrap by device {signer}"
        );
    }

    // The same check with its arguments and result as bytes, as a WASM module exports it.
    assert_eq!(
        browser::member_ids::<D::Observer>(
            &latest,
            CHANNEL,
            2,
            &network.id(0),
            &encoded_admitted(&network.fleet, 0..3),
        )
        .unwrap(),
        [network.id(0), network.id(2)].concat(),
        "{lib}: the member IDs of epoch 2"
    );
}

/// `bytes` with its one occurrence of `from` replaced by `to`, of the same length.
#[track_caller]
fn replace_once(bytes: &mut [u8], from: &[u8], to: &[u8]) {
    let at: Vec<_> = bytes
        .windows(from.len())
        .enumerate()
        .filter(|(_, window)| *window == from)
        .map(|(at, _)| at)
        .collect();
    assert_eq!(at.len(), 1, "one occurrence");
    bytes[at[0]..at[0] + to.len()].copy_from_slice(to);
}

/// The data of the GroupInfo's `ratchet_tree` extension.
fn ratchet_tree(fields: &mut wire::GroupInfo) -> &mut Vec<u8> {
    &mut fields
        .extensions
        .iter_mut()
        .find(|extension| extension.extension_type == wire::RATCHET_TREE)
        .expect("a ratchet_tree extension")
        .data
}

/// A browser rejects a GroupInfo signed by a key that is not a member leaf's: a removed device's
/// at its old leaf, another key at the signer's leaf, or for a leaf that does not exist. It rejects
/// one whose tree was altered to list a removed device in a member's leaf, signed again by its
/// signer or not, and one with the tree of an earlier epoch; one of another channel or epoch than the wrap's; one that does not list the
/// wrap's signer; and one that lists a member that is not admitted ("Wraps").
pub fn group_info_that_fails_the_browser_check_is_rejected<D: Device>() {
    let lib = D::LIBRARY;
    let mut network = Network::<D>::channel(3);
    let first = network.service.group_info(CHANNEL).unwrap().to_vec();
    let id = network.id(1).to_vec();
    let remove = network
        .fleet
        .step(2, |d| d.remove(CHANNEL, &[id], &[]))
        .unwrap();
    network.send(2, &remove).unwrap();
    let latest = network.service.group_info(CHANNEL).unwrap().to_vec();

    let (member, removed, signer) = (
        network.fleet.identity(0).clone(),
        network.fleet.identity(1).clone(),
        network.fleet.identity(2).clone(),
    );
    let outsider = Identity::generate();
    let admitted = network.fleet.admitted();
    // Device 1 in place of device 0 in leaf 0: every device the tree lists is admitted.
    let altered = |fields: &mut wire::GroupInfo| {
        let tree = ratchet_tree(fields);
        replace_once(tree, &member.device_id(), &removed.device_id());
        replace_once(tree, &member.public_key(), &removed.public_key());
    };
    let mut unsigned = wire::group_info(&latest).unwrap();
    altered(&mut unsigned);
    // The tree of epoch 1, which lists device 1 in its leaf; each leaf's signature verifies.
    let first_tree = ratchet_tree(&mut wire::group_info(&first).unwrap()).clone();
    let stale = |fields: &mut wire::GroupInfo| *ratchet_tree(fields) = first_tree.clone();
    let mut excluding = network.fleet.roster().clone();
    excluding.exclude(ACCOUNT, member.device_id());
    let check = |group_info: &[u8], channel: &[u8], epoch: u64, wrap_signer: &Identity| {
        browser::members::<D::Observer>(
            group_info,
            channel,
            epoch,
            &wrap_signer.device_id(),
            &admitted,
        )
    };

    assert!(
        check(&latest, CHANNEL, 2, &member).is_ok(),
        "{lib}: the latest GroupInfo passes"
    );
    for (rejected, why) in [
        (
            check(
                &resigned_group_info(&latest, &removed, |fields| fields.signer = 1),
                CHANNEL,
                2,
                &member,
            ),
            "a GroupInfo signed by a removed device at its old leaf",
        ),
        (
            check(
                &resigned_group_info(&latest, &outsider, |_| {}),
                CHANNEL,
                2,
                &member,
            ),
            "a GroupInfo signed by another key than its signer leaf's",
        ),
        (
            check(
                &resigned_group_info(&latest, &outsider, |fields| fields.signer = 9),
                CHANNEL,
                2,
                &member,
            ),
            "a GroupInfo signed for a leaf that does not exist",
        ),
        (
            check(&unsigned.encode(), CHANNEL, 2, &signer),
            "a GroupInfo with an altered tree",
        ),
        (
            check(
                &resigned_group_info(&latest, &signer, altered),
                CHANNEL,
                2,
                &signer,
            ),
            "a GroupInfo with an altered tree, signed again by its signer",
        ),
        (
            check(
                &resigned_group_info(&latest, &signer, stale),
                CHANNEL,
                2,
                &signer,
            ),
            "a GroupInfo with the tree of an earlier epoch, signed again by its signer",
        ),
        (
            check(&latest, OTHER_CHANNEL, 2, &member),
            "a GroupInfo of another channel",
        ),
        (
            check(&latest, CHANNEL, 1, &member),
            "a GroupInfo of a later epoch than the wrap's",
        ),
        (
            check(&first, CHANNEL, 2, &member),
            "a GroupInfo of an earlier epoch than the wrap's",
        ),
        (
            check(&latest, CHANNEL, 2, &removed),
            "a GroupInfo that does not list the wrap's signer",
        ),
        (
            browser::members::<D::Observer>(
                &latest,
                CHANNEL,
                2,
                &signer.device_id(),
                &excluding.admitted(),
            ),
            "a GroupInfo that lists a member that is not admitted",
        ),
    ] {
        assert!(rejected.is_err(), "{lib}: {why} is rejected");
    }

    let admitted = encoded_admitted(&network.fleet, 0..3);
    let member_ids = |signer: &[u8], admitted: &[u8]| {
        browser::member_ids::<D::Observer>(&latest, CHANNEL, 2, signer, admitted)
    };
    assert!(member_ids(&member.device_id(), &admitted).is_ok());
    assert!(
        member_ids(&member.device_id(), &[&admitted[..], &[0]].concat()).is_err(),
        "{lib}: admitted devices with a trailing byte are rejected"
    );
    assert!(
        member_ids(&member.device_id()[..15], &admitted).is_err(),
        "{lib}: a signer ID of 15 bytes is rejected"
    );
}
