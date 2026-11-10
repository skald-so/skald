//! Interoperability of two MLS libraries: one channel with devices on both, as
//! `docs/dev/mls-prototypes.md`, "Interoperability", lists its scenarios. In each, a device on
//! library `A` takes the step and devices on library `B` process it; [`crate::interop_tests!`]
//! runs them in both directions.

use crate::{
    Admitted, Commit, Device, DeviceId, Fleet, HistoryKey, Identity, Joining, Leaf, Received,
    Result, Staged,
    scenarios::{
        CHANNEL, Read, create_channel, external_read, history_key, join_external, leaf, leaves,
        process_commit, process_from, remove_device, revocations,
    },
    wire::{self, CommitSender, PublicCommit},
};

/// The library a device of a [`Mixed`] fleet is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    A,
    B,
}

/// A device on library `A` or `B`: a [`Fleet`] of them is a channel with devices on both. Its
/// saved state is its library's after a byte that names the library. [`Device::new`] makes a
/// device on `A`, [`Mixed::on`] one on either.
pub enum Mixed<A, B> {
    A(A),
    B(B),
}

/// Runs `$body` on the device of a [`Mixed`], named `$device`, whatever its library.
macro_rules! on_device {
    ($mixed:expr, $device:ident => $body:expr) => {
        match $mixed {
            Mixed::A($device) => $body,
            Mixed::B($device) => $body,
        }
    };
}

impl<A: Device, B: Device> Mixed<A, B> {
    /// A new device on the library `side`.
    pub fn on(side: Side, identity: &Identity) -> Result<Self> {
        Ok(match side {
            Side::A => Self::A(A::new(identity)?),
            Side::B => Self::B(B::new(identity)?),
        })
    }

    /// The device's library.
    pub fn side(&self) -> Side {
        match self {
            Self::A(_) => Side::A,
            Self::B(_) => Side::B,
        }
    }

    /// The name of the device's library.
    pub fn library(&self) -> &'static str {
        match self {
            Self::A(_) => A::LIBRARY,
            Self::B(_) => B::LIBRARY,
        }
    }
}

impl<A: Device, B: Device> Device for Mixed<A, B> {
    const LIBRARY: &'static str = "interop";
    /// Library `A`'s: no interop scenario has a Delivery Service.
    type Observer = A::Observer;

    fn new(identity: &Identity) -> Result<Self> {
        Self::on(Side::A, identity)
    }

    fn load(identity: &Identity, state: &[u8]) -> Result<Self> {
        match state.split_first() {
            Some((&0, state)) => Ok(Self::A(A::load(identity, state)?)),
            Some((&1, state)) => Ok(Self::B(B::load(identity, state)?)),
            _ => Err("saved state: no library".into()),
        }
    }

    fn save(&self) -> Result<Vec<u8>> {
        let (tag, state) = match self {
            Self::A(device) => (0, device.save()?),
            Self::B(device) => (1, device.save()?),
        };
        Ok([vec![tag], state].concat())
    }

    fn key_package(&mut self) -> Result<Vec<u8>> {
        on_device!(self, device => device.key_package())
    }

    fn last_resort_key_package(&mut self) -> Result<Vec<u8>> {
        on_device!(self, device => device.last_resort_key_package())
    }

    fn create_group(&mut self, group_id: &[u8]) -> Result<()> {
        on_device!(self, device => device.create_group(group_id))
    }

    fn add(
        &mut self,
        group_id: &[u8],
        key_packages: &[Vec<u8>],
        admitted: &Admitted,
    ) -> Result<Commit> {
        on_device!(self, device => device.add(group_id, key_packages, admitted))
    }

    fn remove(
        &mut self,
        group_id: &[u8],
        devices: &[Vec<u8>],
        authenticated_data: &[u8],
    ) -> Result<Commit> {
        on_device!(self, device => device.remove(group_id, devices, authenticated_data))
    }

    fn empty_commit(&mut self, group_id: &[u8], authenticated_data: &[u8]) -> Result<Commit> {
        on_device!(self, device => device.empty_commit(group_id, authenticated_data))
    }

    fn merge_pending_commit(&mut self, group_id: &[u8]) -> Result<()> {
        on_device!(self, device => device.merge_pending_commit(group_id))
    }

    fn clear_pending_commit(&mut self, group_id: &[u8]) -> Result<()> {
        on_device!(self, device => device.clear_pending_commit(group_id))
    }

    fn delete_group(&mut self, group_id: &[u8]) -> Result<()> {
        on_device!(self, device => device.delete_group(group_id))
    }

    fn join_with(
        &mut self,
        welcome: &[u8],
        check: impl FnOnce(&Joining) -> Result<()>,
    ) -> Result<Vec<u8>> {
        on_device!(self, device => device.join_with(welcome, check))
    }

    fn join_external(
        &mut self,
        group_info: &[u8],
        check: impl FnOnce(&Joining) -> Result<()>,
    ) -> Result<Commit> {
        on_device!(self, device => device.join_external(group_info, check))
    }

    fn encrypt(&mut self, group_id: &[u8], data: &[u8]) -> Result<Vec<u8>> {
        on_device!(self, device => device.encrypt(group_id, data))
    }

    fn process_with(
        &mut self,
        group_id: &[u8],
        message: &[u8],
        check: impl FnOnce(&Staged) -> Result<()>,
    ) -> Result<Received> {
        on_device!(self, device => device.process_with(group_id, message, check))
    }

    fn epoch(&self, group_id: &[u8]) -> Result<u64> {
        on_device!(self, device => device.epoch(group_id))
    }

    fn members(&self, group_id: &[u8]) -> Result<Vec<Leaf>> {
        on_device!(self, device => device.members(group_id))
    }

    fn history_key(&self, group_id: &[u8]) -> Result<HistoryKey> {
        on_device!(self, device => device.history_key(group_id))
    }
}

/// New devices, device `n` on the library `sides[n]`.
#[track_caller]
pub fn fleet<A: Device, B: Device>(sides: &[Side]) -> Fleet<Mixed<A, B>> {
    let mut fleet = Fleet::new(0).unwrap();
    for &side in sides {
        add_device(&mut fleet, side);
    }
    fleet
}

/// Adds a new device on the library `side`; returns its index. Its saved state is that library's.
#[track_caller]
pub fn add_device<A: Device, B: Device>(fleet: &mut Fleet<Mixed<A, B>>, side: Side) -> usize {
    let device = fleet
        .add_device_with(|identity| Mixed::on(side, identity))
        .unwrap();
    let (got, library) = fleet.step(device, |d| Ok((d.side(), d.library()))).unwrap();
    let want = match side {
        Side::A => A::LIBRARY,
        Side::B => B::LIBRARY,
    };
    assert_eq!(
        (got, library),
        (side, want),
        "{} to {}: device {device}'s library",
        A::LIBRARY,
        B::LIBRARY
    );
    device
}

/// A channel on new devices, device `n` on `sides[n]`, that device 0 creates as
/// [`create_channel`] does.
#[track_caller]
pub fn channel<A: Device, B: Device>(sides: &[Side]) -> Fleet<Mixed<A, B>> {
    let mut fleet = fleet(sides);
    create_channel(&mut fleet, sides.len()).unwrap();
    fleet
}

/// Every device in `devices` is at `epoch` with the leaves of the devices `members` in leaf order
/// and derives one history key, which it returns; each sends a message that every other one
/// decrypts with its sender's leaf.
#[track_caller]
pub fn assert_interop<A: Device, B: Device>(
    fleet: &mut Fleet<Mixed<A, B>>,
    devices: &[usize],
    epoch: u64,
    members: &[usize],
) -> HistoryKey {
    let dir = format!("{} to {}", A::LIBRARY, B::LIBRARY);
    let members = leaves(fleet, members.iter().copied());
    let key = history_key(fleet, devices[0]);
    for &device in devices {
        let (library, got_epoch, got_members) = fleet
            .step(device, |d| {
                Ok((d.library(), d.epoch(CHANNEL)?, d.members(CHANNEL)?))
            })
            .unwrap();
        assert_eq!(
            got_epoch, epoch,
            "{dir}: device {device}'s epoch, on {library}"
        );
        assert_eq!(
            got_members, members,
            "{dir}: device {device}'s members, on {library}"
        );
        assert_eq!(
            history_key(fleet, device),
            key,
            "{dir}: device {device}'s history key, on {library}"
        );
    }
    for &sender in devices {
        let data = format!("epoch {epoch}, from device {sender}").into_bytes();
        let message = fleet.step(sender, |d| d.encrypt(CHANNEL, &data)).unwrap();
        for &receiver in devices.iter().filter(|&&receiver| receiver != sender) {
            assert_eq!(
                fleet
                    .step(receiver, |d| d.process(CHANNEL, &message))
                    .unwrap(),
                Received::Application {
                    sender: leaf(fleet.identity(sender)),
                    data: data.clone(),
                },
                "{dir}: device {receiver} decrypts device {sender} in epoch {epoch}"
            );
        }
    }
    key
}

/// Device 0, on `A`, creates the channel's group and commits the Add of devices on both libraries
/// from their KeyPackages; they join from its Welcome.
pub fn creates_the_channel<A: Device, B: Device>() {
    let mut fleet = channel::<A, B>(&[Side::A, Side::B, Side::B, Side::A]);
    assert_interop(&mut fleet, &[0, 1, 2, 3], 1, &[0, 1, 2, 3]);
}

/// Device 1, on `A`, in a channel that a device on `B` created, commits the Add of a device on
/// each library; the members on `B` read the added leaves before they merge, and the new devices
/// join from its Welcome.
pub fn adds_a_device<A: Device, B: Device>() {
    let dir = format!("{} to {}", A::LIBRARY, B::LIBRARY);
    let mut fleet = channel::<A, B>(&[Side::B, Side::A, Side::B]);
    let before = assert_interop(&mut fleet, &[0, 1, 2], 1, &[0, 1, 2]);
    let added = [
        add_device(&mut fleet, Side::B),
        add_device(&mut fleet, Side::A),
    ];
    let key_packages = added.map(|device| fleet.step(device, |d| d.key_package()).unwrap());

    let admitted = fleet.admitted();
    let commit = fleet
        .step(1, |d| d.add(CHANNEL, &key_packages, &admitted))
        .unwrap();
    fleet.step(1, |d| d.merge_pending_commit(CHANNEL)).unwrap();
    let mut want = Read::removal(leaf(fleet.identity(1)), Vec::new(), Vec::new());
    want.staged.added = leaves(&fleet, added);
    for member in [0, 2] {
        assert_eq!(
            process_commit(&mut fleet, member, &commit.commit).unwrap(),
            want,
            "{dir}: device {member} read the Add"
        );
    }
    let welcome = commit.welcome.expect("the Add Commit has a Welcome");
    for device in added {
        assert_eq!(
            fleet.step(device, |d| d.join(&welcome)).unwrap(),
            CHANNEL,
            "{dir}: device {device} joined from the Welcome"
        );
    }
    let after = assert_interop(&mut fleet, &[0, 1, 2, 3, 4], 2, &[0, 1, 2, 3, 4]);
    assert_ne!(after, before, "{dir}: the Add changes the history key");
}

/// Device 1, on `A`, commits the Remove of device 3, on `B`, with its revocation in the Commit's
/// `authenticated_data`; the members on `B` read it before they merge. The removed device
/// processes its Remove and reads no later message.
pub fn removes_a_device<A: Device, B: Device>() {
    let dir = format!("{} to {}", A::LIBRARY, B::LIBRARY);
    let mut fleet = channel::<A, B>(&[Side::B, Side::A, Side::B, Side::B]);
    let removed = 3;
    let before = assert_interop(&mut fleet, &[0, 1, 2, 3], 1, &[0, 1, 2, 3]);
    let identity = fleet.identity(removed).clone();
    let copy = fleet.state(removed).to_vec();

    // `remove_device` checks what each member on `B` reads.
    let commit = remove_device(&mut fleet, 1, removed, [0, 2]).unwrap();
    assert_eq!(
        wire::public_commit(&commit.commit).unwrap().removed,
        [3],
        "{dir}: the Remove Commit"
    );
    let after = assert_interop(&mut fleet, &[0, 1, 2], 2, &[0, 1, 2]);
    assert_ne!(after, before, "{dir}: the Remove changes the history key");

    let mut device = Mixed::<A, B>::load(&identity, &copy).unwrap();
    let mut read = None;
    device
        .process_with(CHANNEL, &commit.commit, |staged| {
            read = Some(staged.removed.clone());
            Ok(())
        })
        .unwrap();
    assert_eq!(
        read,
        Some(vec![leaf(&identity)]),
        "{dir}: the removed device processes its Remove"
    );
    let later = fleet.step(1, |d| d.encrypt(CHANNEL, b"after")).unwrap();
    for (which, state) in [("before", copy), ("after", device.save().unwrap())] {
        assert!(
            process_from::<Mixed<A, B>>(&identity, &state, &later).is_err(),
            "{dir}: the removed device's copy {which} its Remove reads a later message"
        );
    }
}

/// Device 1, on `A`, commits an empty Commit with a browser's revocation in its
/// `authenticated_data`; the members on `B` read it before they merge.
pub fn makes_an_empty_commit<A: Device, B: Device>() {
    let dir = format!("{} to {}", A::LIBRARY, B::LIBRARY);
    let mut fleet = channel::<A, B>(&[Side::B, Side::A, Side::B]);
    let before = assert_interop(&mut fleet, &[0, 1, 2], 1, &[0, 1, 2]);
    let browser: DeviceId = [0xb0; 16];
    let authenticated_data = revocations(&fleet, 1, &[browser]);

    let commit = fleet
        .step(1, |d| d.empty_commit(CHANNEL, &authenticated_data))
        .unwrap();
    assert_eq!(
        wire::public_commit(&commit.commit).unwrap(),
        PublicCommit {
            sender: CommitSender::Member(1),
            authenticated_data: authenticated_data.clone(),
            removed: Vec::new(),
            external_init: false,
            other_proposals: 0,
            path: true,
        },
        "{dir}: the empty Commit"
    );
    fleet.step(1, |d| d.merge_pending_commit(CHANNEL)).unwrap();
    let want = Read {
        revoked: vec![browser],
        ..Read::removal(leaf(fleet.identity(1)), Vec::new(), authenticated_data)
    };
    for member in [0, 2] {
        assert_eq!(
            process_commit(&mut fleet, member, &commit.commit).unwrap(),
            want,
            "{dir}: device {member} read the empty Commit"
        );
    }
    let after = assert_interop(&mut fleet, &[0, 1, 2], 2, &[0, 1, 2]);
    assert_ne!(
        after, before,
        "{dir}: the empty Commit changes the history key"
    );
}

/// Device 2, on `A`, joins by external Commit from the GroupInfo of a channel of devices on `B`;
/// they check it as an Add. Then it misses a Commit and rejoins from the next GroupInfo with an
/// external Commit that also removes its old leaf.
pub fn joins_by_external_commit<A: Device, B: Device>() {
    let dir = format!("{} to {}", A::LIBRARY, B::LIBRARY);
    let mut fleet = fleet::<A, B>(&[Side::B, Side::B]);
    let created = create_channel(&mut fleet, 2).unwrap();
    let joiner = add_device(&mut fleet, Side::A);
    let own = leaf(fleet.identity(joiner));
    let admitted = fleet.admitted();

    let (read, join) = join_external(&mut fleet, joiner, &created.group_info, &admitted);
    assert_eq!(
        read,
        Some(Joining {
            signer: leaf(fleet.identity(0)),
            members: leaves(&fleet, 0..2),
        }),
        "{dir}: the joiner read the GroupInfo"
    );
    let join = join.unwrap();
    assert_external(&dir, &join, &[]);
    for member in [0, 1] {
        assert_eq!(
            process_commit(&mut fleet, member, &join.commit).unwrap(),
            external_read(own.clone(), Vec::new(), Vec::new()),
            "{dir}: device {member} read the external Commit"
        );
    }
    let joined = assert_interop(&mut fleet, &[0, 1, 2], 2, &[0, 1, 2]);

    // Device 2 misses device 0's empty Commit, then rejoins from its GroupInfo.
    let missed = fleet.step(0, |d| d.empty_commit(CHANNEL, &[])).unwrap();
    fleet.step(0, |d| d.merge_pending_commit(CHANNEL)).unwrap();
    process_commit(&mut fleet, 1, &missed.commit).unwrap();
    let (read, rejoin) = join_external(&mut fleet, joiner, &missed.group_info, &admitted);
    assert_eq!(
        read.map(|read| read.signer),
        Some(leaf(fleet.identity(0))),
        "{dir}: the rejoining device read the GroupInfo"
    );
    let rejoin = rejoin.unwrap();
    assert_external(&dir, &rejoin, &[2]);
    for member in [0, 1] {
        assert_eq!(
            process_commit(&mut fleet, member, &rejoin.commit).unwrap(),
            external_read(own.clone(), vec![own.clone()], Vec::new()),
            "{dir}: device {member} read the rejoin"
        );
    }
    let rejoined = assert_interop(&mut fleet, &[0, 1, 2], 4, &[0, 1, 2]);
    assert_ne!(
        rejoined, joined,
        "{dir}: the rejoin changes the history key"
    );
}

/// `commit` is an external Commit that removes the leaves `removed`.
#[track_caller]
fn assert_external(dir: &str, commit: &Commit, removed: &[u32]) {
    assert!(
        commit.welcome.is_none(),
        "{dir}: an external Commit's Welcome"
    );
    assert_eq!(
        wire::public_commit(&commit.commit).unwrap(),
        PublicCommit {
            sender: CommitSender::NewMember,
            authenticated_data: Vec::new(),
            removed: removed.to_vec(),
            external_init: true,
            other_proposals: 0,
            path: true,
        },
        "{dir}: the external Commit"
    );
}

/// In a channel of devices on both libraries, each device sends three messages; every other
/// device decrypts them, the first two out of order, with the sender's leaf.
pub fn messages_flow_both_ways<A: Device, B: Device>() {
    let dir = format!("{} to {}", A::LIBRARY, B::LIBRARY);
    let count = 4;
    let mut fleet = channel::<A, B>(&[Side::A, Side::B, Side::A, Side::B]);
    for sender in 0..count {
        let sent: Vec<_> = (0..3)
            .map(|n| {
                let data = format!("message {n} from device {sender}").into_bytes();
                let message = fleet.step(sender, |d| d.encrypt(CHANNEL, &data)).unwrap();
                (data, message)
            })
            .collect();
        for receiver in (0..count).filter(|&receiver| receiver != sender) {
            for n in [1, 0, 2] {
                let (data, message) = &sent[n];
                assert_eq!(
                    fleet
                        .step(receiver, |d| d.process(CHANNEL, message))
                        .unwrap(),
                    Received::Application {
                        sender: leaf(fleet.identity(sender)),
                        data: data.clone(),
                    },
                    "{dir}: device {receiver} decrypts message {n} of device {sender}"
                );
            }
        }
    }
}

/// The harness: a device of a [`Mixed`] fleet runs each step on its library, the steps no
/// scenario takes among them.
pub fn devices_run_on_their_library<A: Device, B: Device>() {
    let dir = format!("{} to {}", A::LIBRARY, B::LIBRARY);
    let mut fleet = channel::<A, B>(&[Side::A, Side::B]);
    for device in 0..2 {
        let identity = fleet.identity(device).clone();
        let last_resort = fleet.step(device, |d| d.last_resort_key_package()).unwrap();
        let fields = wire::key_package(&last_resort).unwrap();
        assert_eq!(
            (fields.last_resort(), fields.identity),
            (true, identity.device_id().to_vec()),
            "{dir}: device {device}'s last-resort KeyPackage"
        );

        fleet
            .step(device, |d| {
                d.empty_commit(CHANNEL, &[])?;
                d.clear_pending_commit(CHANNEL)
            })
            .unwrap();
        assert!(
            fleet
                .step(device, |d| d.merge_pending_commit(CHANNEL))
                .is_err(),
            "{dir}: device {device} merged a cleared Commit"
        );
        assert_eq!(fleet.step(device, |d| d.epoch(CHANNEL)).unwrap(), 1);

        fleet.step(device, |d| d.delete_group(CHANNEL)).unwrap();
        assert!(
            fleet.step(device, |d| d.epoch(CHANNEL)).is_err(),
            "{dir}: device {device} dropped the group"
        );
    }
    assert!(
        Mixed::<A, B>::load(fleet.identity(0), &[]).is_err(),
        "{dir}: a saved state without its library"
    );
}

/// Defines one test per interop scenario with a device on `$a` taking each step and devices on
/// `$b` processing it; run it again with the two swapped for the other direction.
#[macro_export]
macro_rules! interop_tests {
    ($a:ty, $b:ty) => {
        $crate::interop_tests!(
            @tests $a, $b;
            creates_the_channel,
            adds_a_device,
            removes_a_device,
            makes_an_empty_commit,
            joins_by_external_commit,
            messages_flow_both_ways,
            devices_run_on_their_library
        );
    };
    (@tests $a:ty, $b:ty; $($scenario:ident),+) => {
        $(
            #[test]
            fn $scenario() {
                $crate::interop::$scenario::<$a, $b>();
            }
        )+
    };
}
