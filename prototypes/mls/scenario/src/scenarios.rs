//! The scenarios each prototype runs, as `docs/dev/mls-prototypes.md`, "Scenarios", lists them.
//! Each one panics on a failed check; [`crate::scenario_tests!`] makes them tests.

use crate::{
    Commit, Device, DeviceId, Fleet, Identity, Leaf, Received, Result, Staged,
    certificate::Kind,
    identity::device_id,
    key_packages,
    revocation::{self, AccountId},
    wire::{self, Framing, PublicCommit},
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
        assert!(
            (start..=end).any(|now| key_packages::lifetime(now) == lifetime),
            "{lib}: KeyPackage {n}'s lifetime {lifetime:?}"
        );
        assert!(
            lifetime.not_after - lifetime.not_before <= key_packages::MAX_LIFETIME
                && lifetime.holds(end)
                && !key_packages::due(&lifetime, end),
            "{lib}: KeyPackage {n}'s lifetime {lifetime:?}"
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

    let now = key_packages::now();
    let hour = 60 * 60;
    let mut bad_signature = valid.clone();
    *bad_signature.last_mut().unwrap() ^= 1;
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
                    not_before: now - key_packages::MAX_LIFETIME - hour,
                    not_after: now - hour,
                }
            }),
            "an expired lifetime",
        ),
        (
            resigned(&valid, &identity, |kp| {
                kp.lifetime = wire::Lifetime {
                    not_before: now + hour,
                    not_after: now + hour + key_packages::MAX_LIFETIME,
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
            sender: 0,
            authenticated_data: Vec::new(),
            removed: vec![3],
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
