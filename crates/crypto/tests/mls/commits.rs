//! Commits that change a channel's group: `docs/protocol/membership.md`, "Group changes", "Commit
//! ordering" and "History key". A member commits the Remove of one device or several, or an empty
//! Commit, each carrying the `authenticated_data` the caller gives; its own Commit stays pending
//! until the caller says the server stored or refused it. Another device's Commit gives the caller
//! its sender, the devices it adds and removes and its `authenticated_data` before the new epoch is
//! saved; the caller keeps the epoch or refuses it. Every step loads the state the previous step
//! sealed.

use skald_crypto::{GroupChange, MlsError, Processed, SealedState};
use skald_proto::{ChannelId, DeviceId};

use crate::fixtures::{NOW, device_id, load};
use crate::framing::{COMMIT, PUBLIC_MESSAGE};
use crate::steps::{
    add, bytes, channel_of, history_key, join, receive, send, step, stored, with_key_packages,
};
use crate::wire::{MLS10, Reader};

/// What the caller sees of another device's Commit before its epoch is saved, by device ID.
#[derive(Debug, PartialEq, Eq)]
pub struct Seen {
    pub epoch: u64,
    pub sender: DeviceId,
    pub added: Vec<DeviceId>,
    pub removed: Vec<DeviceId>,
    pub authenticated_data: Vec<u8>,
}

impl Seen {
    fn of(change: &GroupChange) -> Self {
        Seen {
            epoch: change.epoch,
            sender: change.sender,
            added: change.added.iter().map(|member| member.device).collect(),
            removed: change.removed.iter().map(|member| member.device).collect(),
            authenticated_data: change.authenticated_data.to_vec(),
        }
    }
}

/// The device IDs of `seeds`.
pub fn devices(seeds: &[u8]) -> Vec<DeviceId> {
    seeds.iter().map(|&seed| device_id(seed)).collect()
}

/// Device `seed` commits the Remove of devices `removed` in `channel` with `authenticated_data`;
/// gives the Commit, or why it is refused.
pub fn try_remove(
    seed: u8,
    state: &mut SealedState,
    channel: ChannelId,
    removed: &[u8],
    authenticated_data: &[u8],
) -> Result<Vec<u8>, MlsError> {
    step(seed, state, |device| {
        device
            .remove_devices(channel, &devices(removed), authenticated_data, NOW)
            .map(bytes)
    })
}

/// Device `seed` commits the Remove of devices `removed` in `channel`; gives the Commit.
pub fn remove(
    seed: u8,
    state: &mut SealedState,
    channel: ChannelId,
    removed: &[u8],
    authenticated_data: &[u8],
) -> Vec<u8> {
    try_remove(seed, state, channel, removed, authenticated_data).expect("a Commit of the Removes")
}

/// Device `seed` commits an empty Commit in `channel`; gives the Commit.
pub fn empty(
    seed: u8,
    state: &mut SealedState,
    channel: ChannelId,
    authenticated_data: &[u8],
) -> Vec<u8> {
    step(seed, state, |device| {
        bytes(
            device
                .commit_empty(channel, authenticated_data, NOW)
                .expect("an empty Commit"),
        )
    })
}

/// The server stored device `seed`'s pending Commit in `channel`, one that adds no device.
pub fn store(seed: u8, state: &mut SealedState, channel: ChannelId) {
    let welcome = step(seed, state, |device| {
        device
            .commit_stored(channel)
            .expect("the stored Commit merges")
    });
    assert!(
        welcome.is_none(),
        "a Commit that adds no device has no Welcome"
    );
}

/// Device `seed` processes `commit`, another device's, in `channel` at `now`; the caller keeps the
/// new epoch if `keep`. Gives what the caller saw, if it was asked, and the outcome.
pub fn try_process(
    seed: u8,
    state: &mut SealedState,
    channel: ChannelId,
    commit: &[u8],
    now: u64,
    keep: bool,
) -> (Option<Seen>, Result<Processed, MlsError>) {
    let mut seen = None;
    let outcome = step(seed, state, |device| {
        device.process_commit(channel, commit, now, |change| {
            seen = Some(Seen::of(change));
            keep
        })
    });
    (seen, outcome)
}

/// Device `seed` processes `commit` in `channel` at `now` and keeps the new epoch; gives what the
/// caller saw before it was saved.
pub fn process_at(
    seed: u8,
    state: &mut SealedState,
    channel: ChannelId,
    commit: &[u8],
    now: u64,
) -> Seen {
    let (seen, outcome) = try_process(seed, state, channel, commit, now, true);
    let seen = seen.expect("the caller sees the Commit before its epoch is saved");
    match outcome {
        Ok(Processed::Entered { epoch }) => {
            assert_eq!(epoch, seen.epoch, "the epoch entered is the one seen");
        }
        Ok(Processed::Removed) => panic!("device {seed} was removed"),
        Err(error) => panic!("device {seed} processes the Commit: {error:?}"),
    }
    seen
}

/// Device `seed` processes `commit` in `channel` at the tests' time and keeps the new epoch.
pub fn process(seed: u8, state: &mut SealedState, channel: ChannelId, commit: &[u8]) -> Seen {
    process_at(seed, state, channel, commit, NOW)
}

/// Device `seed` processes `commit` in `channel`, which removes it.
pub fn processes_own_remove(seed: u8, state: &mut SealedState, channel: ChannelId, commit: &[u8]) {
    match try_process(seed, state, channel, commit, NOW, true).1 {
        Ok(Processed::Removed) => {}
        Ok(Processed::Entered { epoch }) => panic!("device {seed} stayed, in epoch {epoch}"),
        Err(error) => panic!("device {seed} processes its own Remove: {error:?}"),
    }
}

/// Device `seed`, reloaded from `state`, is in `epoch` of `channel` with `members` in leaf order,
/// and holds that epoch's history key and none of the next.
pub fn assert_in(
    seed: u8,
    state: &SealedState,
    channel: ChannelId,
    epoch: u64,
    members: &[u8],
    what: &str,
) {
    let device = load(seed, state);
    assert_eq!(device.epoch(channel).expect("the group"), epoch, "{what}");
    let got: Vec<DeviceId> = device
        .members(channel)
        .expect("the group")
        .iter()
        .map(|member| member.device)
        .collect();
    assert_eq!(got, devices(members), "{what}: the members");
    assert!(device.history_key(channel, epoch).is_some(), "{what}");
    assert!(device.history_key(channel, epoch + 1).is_none(), "{what}");
}

/// Whether device `seed` has a Commit pending in `channel`: the server storing it merges it. Leaves
/// `state` as it was.
fn has_pending(seed: u8, state: &SealedState, channel: ChannelId) -> bool {
    step(seed, &mut state.clone(), |device| {
        device.commit_stored(channel).is_ok()
    })
}

/// The fields of a Commit in a PublicMessage the tests check, as RFC 9420, §6 and §12.4, encodes
/// them, apart from the crate under test.
struct CommitFields {
    authenticated_data: Vec<u8>,
    /// The bytes of `proposals`.
    proposals: Vec<u8>,
    has_path: bool,
}

fn commit_fields(bytes: &[u8]) -> CommitFields {
    let mut reader = Reader { bytes, at: 0 };
    assert_eq!(reader.u16(), MLS10, "the protocol version");
    assert_eq!(reader.u16(), PUBLIC_MESSAGE, "a PublicMessage");
    reader.vector(); // group_id
    reader.take(8); // epoch
    assert_eq!(reader.u8(), 1, "a member's Commit");
    reader.take(4); // leaf_index
    let authenticated_data = reader.vector().to_vec();
    assert_eq!(reader.u8(), COMMIT, "a Commit");
    let proposals = reader.vector().to_vec();
    let has_path = match reader.u8() {
        0 => false,
        1 => true,
        other => panic!("optional<UpdatePath> {other}"),
    };
    CommitFields {
        authenticated_data,
        proposals,
        has_path,
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_devices_own_commit_stays_pending_until_the_server_stores_or_refuses_it() {
    let channel = ChannelId::generate();
    let mut states = channel_of(channel, 1, &[2]);

    remove(1, &mut states[0], channel, &[2], b"");
    assert_in(1, &states[0], channel, 1, &[1, 2], "the Remove pending");
    assert!(
        has_pending(1, &states[0], channel),
        "pending after a reload"
    );
    step(1, &mut states[0], |device| device.commit_refused(channel))
        .expect("the refused Commit is dropped");
    assert_in(1, &states[0], channel, 1, &[1, 2], "the Remove refused");
    assert!(
        !has_pending(1, &states[0], channel),
        "the refused Commit is gone"
    );
    let message = send(1, &mut states[0], channel, b"after the refused Commit");
    assert_eq!(
        receive(2, &mut states[1], channel, &message),
        Some((device_id(1), b"after the refused Commit".to_vec())),
        "the group stays in its epoch"
    );

    let commit = remove(1, &mut states[0], channel, &[2], b"");
    assert_in(
        1,
        &states[0],
        channel,
        1,
        &[1, 2],
        "the Remove pending again",
    );
    store(1, &mut states[0], channel);
    assert_in(1, &states[0], channel, 2, &[1], "the Remove stored");
    assert!(
        !has_pending(1, &states[0], channel),
        "the stored Commit merged"
    );
    processes_own_remove(2, &mut states[1], channel, &commit);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn an_empty_commit_starts_an_epoch_with_the_same_members() {
    let channel = ChannelId::generate();
    let seeds = [3, 4, 5];
    let mut states = channel_of(channel, seeds[0], &seeds[1..]);
    let before = history_key(3, &states[0], channel, 1);

    let commit = empty(3, &mut states[0], channel, b"");
    let fields = commit_fields(&commit);
    assert_eq!(fields.proposals, b"", "an empty Commit has no proposals");
    assert!(fields.has_path, "an empty Commit has an UpdatePath");
    store(3, &mut states[0], channel);
    for at in 1..seeds.len() {
        assert_eq!(
            process(seeds[at], &mut states[at], channel, &commit),
            Seen {
                epoch: 2,
                sender: device_id(3),
                added: vec![],
                removed: vec![],
                authenticated_data: vec![],
            }
        );
    }

    let key = history_key(3, &states[0], channel, 2);
    assert_ne!(key, before, "the history key rotates");
    for (at, &seed) in seeds.iter().enumerate() {
        assert_in(
            seed,
            &states[at],
            channel,
            2,
            &seeds,
            "after the empty Commit",
        );
        assert_eq!(history_key(seed, &states[at], channel, 2), key);
    }
    let message = send(5, &mut states[2], channel, b"in the new epoch");
    for (at, seed) in [(0, 3), (1, 4)] {
        assert_eq!(
            receive(seed, &mut states[at], channel, &message),
            Some((device_id(5), b"in the new epoch".to_vec())),
            "device {seed} reads it"
        );
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_commit_gives_its_sender_adds_removes_and_authenticated_data_before_its_epoch_is_saved() {
    let channel = ChannelId::generate();
    let mut states = channel_of(channel, 6, &[7, 8]);

    // An Add by device 6.
    let (mut joiner, packages) = with_key_packages(9, 1);
    let commit = add(6, &mut states[0], channel, &[&packages[0]]);
    let welcome = stored(6, &mut states[0], channel);
    for (at, seed) in [(1, 7), (2, 8)] {
        assert_eq!(
            process(seed, &mut states[at], channel, &commit),
            Seen {
                epoch: 2,
                sender: device_id(6),
                added: devices(&[9]),
                removed: vec![],
                authenticated_data: vec![],
            },
            "device {seed} sees the Add"
        );
    }
    assert_eq!(join(9, &mut joiner, &welcome), channel);
    states.push(joiner);

    // A Remove by device 6, carrying the revocations by which it holds device 8 revoked.
    let revocations = b"the encoded revocations".to_vec();
    let commit = remove(6, &mut states[0], channel, &[8], &revocations);
    assert_eq!(commit_fields(&commit).authenticated_data, revocations);
    store(6, &mut states[0], channel);
    for (at, seed) in [(1, 7), (3, 9)] {
        assert_eq!(
            process(seed, &mut states[at], channel, &commit),
            Seen {
                epoch: 3,
                sender: device_id(6),
                added: vec![],
                removed: devices(&[8]),
                authenticated_data: revocations.clone(),
            },
            "device {seed} sees the Remove"
        );
    }
    processes_own_remove(8, &mut states[2], channel, &commit);

    // An empty Commit by device 7, after a browser's revocation.
    let revocation = b"a browser's revocation".to_vec();
    let commit = empty(7, &mut states[1], channel, &revocation);
    assert_eq!(commit_fields(&commit).authenticated_data, revocation);
    store(7, &mut states[1], channel);
    for (at, seed) in [(0, 6), (3, 9)] {
        assert_eq!(
            process(seed, &mut states[at], channel, &commit),
            Seen {
                epoch: 4,
                sender: device_id(7),
                added: vec![],
                removed: vec![],
                authenticated_data: revocation.clone(),
            },
            "device {seed} sees the empty Commit"
        );
    }
    for (at, seed) in [(0, 6), (1, 7), (3, 9)] {
        assert_in(
            seed,
            &states[at],
            channel,
            4,
            &[6, 7, 9],
            "after the changes",
        );
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_refused_epoch_leaves_the_state_in_the_old_epoch() {
    let channel = ChannelId::generate();
    let mut states = channel_of(channel, 10, &[11]);
    let (mut joiner, packages) = with_key_packages(12, 1);
    let commit = add(10, &mut states[0], channel, &[&packages[0]]);
    let welcome = stored(10, &mut states[0], channel);

    let (seen, outcome) = try_process(11, &mut states[1], channel, &commit, NOW, false);
    assert_eq!(
        seen.expect("the caller sees the Commit").added,
        devices(&[12])
    );
    assert!(outcome.is_err(), "the caller refuses the new epoch");
    assert_in(11, &states[1], channel, 1, &[10, 11], "the epoch refused");

    // Nothing of the refused epoch was kept: the same Commit is processed again.
    process(11, &mut states[1], channel, &commit);
    assert_in(11, &states[1], channel, 2, &[10, 11, 12], "the epoch kept");
    assert_eq!(
        history_key(11, &states[1], channel, 2),
        history_key(10, &states[0], channel, 2)
    );
    assert_eq!(join(12, &mut joiner, &welcome), channel);
    let message = send(10, &mut states[0], channel, b"in epoch 2");
    assert_eq!(
        receive(11, &mut states[1], channel, &message),
        Some((device_id(10), b"in epoch 2".to_vec()))
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn of_two_commits_made_at_once_both_devices_end_in_the_epoch_of_the_one_stored_first() {
    let channel = ChannelId::generate();
    let mut states = channel_of(channel, 13, &[14, 15]);

    let first = remove(13, &mut states[0], channel, &[15], b"");
    let second = empty(14, &mut states[1], channel, b"");
    // The server stores the first and refuses the second.
    store(13, &mut states[0], channel);
    assert_eq!(
        process(14, &mut states[1], channel, &first).sender,
        device_id(13),
        "device 14 takes the stored Commit"
    );
    assert!(
        !has_pending(14, &states[1], channel),
        "device 14 dropped its own Commit"
    );
    assert!(
        step(14, &mut states[1], |device| device.commit_refused(channel)).is_err(),
        "nothing is left to refuse"
    );
    assert!(
        try_process(13, &mut states[0], channel, &second, NOW, true)
            .1
            .is_err(),
        "the Commit of the old epoch is refused"
    );
    processes_own_remove(15, &mut states[2], channel, &first);

    for (at, seed) in [(0, 13), (1, 14)] {
        assert_in(
            seed,
            &states[at],
            channel,
            2,
            &[13, 14],
            "the stored Commit's epoch",
        );
    }
    assert_eq!(
        history_key(13, &states[0], channel, 2),
        history_key(14, &states[1], channel, 2)
    );
    let message = send(14, &mut states[1], channel, b"in the same epoch");
    assert_eq!(
        receive(13, &mut states[0], channel, &message),
        Some((device_id(14), b"in the same epoch".to_vec()))
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_device_catches_up_on_a_hundred_missed_commits_and_holds_the_history_key_of_each_epoch() {
    let channel = ChannelId::generate();
    let mut states = channel_of(channel, 16, &[17]);
    // Device 17 is offline while device 16 commits 100 times.
    let commits: Vec<Vec<u8>> = (0..100)
        .map(|_| {
            let commit = empty(16, &mut states[0], channel, b"");
            store(16, &mut states[0], channel);
            commit
        })
        .collect();

    assert!(
        try_process(17, &mut states[1], channel, &commits[1], NOW, true)
            .1
            .is_err(),
        "a Commit after a missed one is refused"
    );
    assert_in(17, &states[1], channel, 1, &[16, 17], "nothing processed");
    for (at, commit) in commits.iter().enumerate() {
        assert_eq!(
            process(17, &mut states[1], channel, commit).epoch,
            at as u64 + 2
        );
    }

    assert_in(17, &states[1], channel, 101, &[16, 17], "caught up");
    let keys: Vec<[u8; 16]> = (1..=101)
        .map(|epoch| {
            let key = history_key(17, &states[1], channel, epoch);
            assert_eq!(
                key,
                history_key(16, &states[0], channel, epoch),
                "epoch {epoch}"
            );
            key
        })
        .collect();
    for (at, key) in keys.iter().enumerate() {
        assert!(!keys[..at].contains(key), "each epoch has a key of its own");
    }
    let message = send(16, &mut states[0], channel, b"after 100 Commits");
    assert_eq!(
        receive(17, &mut states[1], channel, &message),
        Some((device_id(16), b"after 100 Commits".to_vec()))
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_malformed_or_truncated_commit_is_refused_without_a_panic_and_changes_nothing() {
    let channel = ChannelId::generate();
    let mut states = channel_of(channel, 18, &[19]);
    let message = send(18, &mut states[0], channel, b"an application message");
    let commit = empty(18, &mut states[0], channel, b"");
    store(18, &mut states[0], channel);

    let mut inputs: Vec<(Vec<u8>, String)> = [0, 1, 4, commit.len() / 2, commit.len() - 1]
        .into_iter()
        .map(|len| (commit[..len].to_vec(), format!("the first {len} bytes")))
        .collect();
    for at in [3, 20, commit.len() / 2, commit.len() - 1] {
        let mut altered = commit.clone();
        altered[at] ^= 0x01;
        inputs.push((altered, format!("byte {at} altered")));
    }
    inputs.push((message, "an application message".to_owned()));
    for (input, what) in inputs {
        assert!(
            try_process(19, &mut states[1], channel, &input, NOW, true)
                .1
                .is_err(),
            "refused: {what}"
        );
        assert_in(19, &states[1], channel, 1, &[18, 19], &what);
    }

    process(19, &mut states[1], channel, &commit);
    assert_in(19, &states[1], channel, 2, &[18, 19], "the intact Commit");
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_commit_of_another_channels_group_is_refused() {
    let (first, second) = (ChannelId::generate(), ChannelId::generate());
    let mut firsts = channel_of(first, 20, &[21]);
    let mut seconds = channel_of(second, 20, &[21]);
    let commit = empty(20, &mut seconds[0], second, b"");
    store(20, &mut seconds[0], second);

    assert!(
        try_process(21, &mut firsts[1], first, &commit, NOW, true)
            .1
            .is_err(),
        "refused in another channel's group"
    );
    assert_in(21, &firsts[1], first, 1, &[20, 21], "the other group");
    let (mut outsider, _) = with_key_packages(22, 1);
    assert!(
        try_process(22, &mut outsider, second, &commit, NOW, true)
            .1
            .is_err(),
        "refused by a device with no group of the channel"
    );
    assert!(load(22, &outsider).epoch(second).is_err(), "no group");

    process(21, &mut seconds[1], second, &commit);
    assert_in(21, &seconds[1], second, 2, &[20, 21], "its own group");
    let message = send(20, &mut firsts[0], first, b"the first channel");
    assert_eq!(
        receive(21, &mut firsts[1], first, &message),
        Some((device_id(20), b"the first channel".to_vec()))
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn removing_a_device_that_is_not_a_member_is_refused_and_leaves_the_group_as_it_was() {
    let channel = ChannelId::generate();
    let mut states = channel_of(channel, 23, &[24]);

    for (removed, what) in [
        (&[25][..], "a device that is not a member"),
        (
            &[24, 25][..],
            "a member and a device that is not one, in one Commit",
        ),
    ] {
        assert!(
            try_remove(23, &mut states[0], channel, removed, b"").is_err(),
            "refused: {what}"
        );
        assert!(
            !has_pending(23, &states[0], channel),
            "{what}: nothing pending"
        );
        assert_in(23, &states[0], channel, 1, &[23, 24], what);
    }

    remove(23, &mut states[0], channel, &[24], b"");
    store(23, &mut states[0], channel);
    assert_in(23, &states[0], channel, 2, &[23], "the member removed");
}
