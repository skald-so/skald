//! Steps of devices in a channel's group, each loading the state the previous step sealed and
//! sealing the state it leaves, as one-shot mode does.

use skald_crypto::{MlsDevice, SealedState};
use skald_proto::{ChannelId, DeviceId};

use crate::fixtures::{NOW, admitted, load, new_state};

/// The bytes of a KeyPackage, Commit, Welcome or message the crate gives.
pub fn bytes(value: impl AsRef<[u8]>) -> Vec<u8> {
    value.as_ref().to_vec()
}

/// Runs `step` on device `seed` loaded from `state`, then seals the device into `state`.
pub fn step<T>(seed: u8, state: &mut SealedState, step: impl FnOnce(&mut MlsDevice) -> T) -> T {
    let mut device = load(seed, state);
    let output = step(&mut device);
    *state = device.seal();
    output
}

/// A new device `seed`'s state and `count` KeyPackages it made, a step each.
pub fn with_key_packages(seed: u8, count: usize) -> (SealedState, Vec<Vec<u8>>) {
    let mut state = new_state(seed);
    let packages = (0..count)
        .map(|_| {
            step(seed, &mut state, |device| {
                bytes(device.key_package(NOW).expect("a KeyPackage"))
            })
        })
        .collect();
    (state, packages)
}

/// Device `seed` with a KeyPackage of its own creates `channel`'s group at epoch 0.
pub fn creator(seed: u8, channel: ChannelId) -> SealedState {
    let (mut state, _) = with_key_packages(seed, 1);
    step(seed, &mut state, |device| {
        device.create_group(channel, NOW).expect("a new group");
    });
    state
}

/// Device `seed` adds the devices of `key_packages` to `channel`'s group and commits, the tests'
/// devices admitted; gives the Commit.
pub fn add(
    seed: u8,
    state: &mut SealedState,
    channel: ChannelId,
    key_packages: &[&[u8]],
) -> Vec<u8> {
    step(seed, state, |device| {
        bytes(
            device
                .add_devices(channel, key_packages, admitted(), NOW)
                .expect("a Commit of the Adds"),
        )
    })
}

/// The server stored device `seed`'s pending Commit in `channel`; gives the Commit's Welcome.
pub fn stored(seed: u8, state: &mut SealedState, channel: ChannelId) -> Vec<u8> {
    step(seed, state, |device| {
        bytes(
            device
                .commit_stored(channel)
                .expect("the stored Commit merges")
                .expect("the Welcome of a Commit with Adds"),
        )
    })
}

/// Device `seed` joins from `welcome`, the caller's checks passing; gives the channel joined.
pub fn join(seed: u8, state: &mut SealedState, welcome: &[u8]) -> ChannelId {
    step(seed, state, |device| {
        device
            .join(welcome, NOW, |_| true)
            .expect("the device joins")
    })
}

/// `channel`'s group as "Creating a channel" forms it: device `creator` creates it, adds each
/// of `joiners` from a KeyPackage of its own and commits, the server stores the Commit, and each
/// joiner joins from the Welcome. Gives each device's state, the creator's first.
pub fn channel_of(channel: ChannelId, creator_seed: u8, joiners: &[u8]) -> Vec<SealedState> {
    let mut joined: Vec<(SealedState, Vec<u8>)> = joiners
        .iter()
        .map(|&seed| {
            let (state, mut packages) = with_key_packages(seed, 1);
            (state, packages.remove(0))
        })
        .collect();
    let mut state = creator(creator_seed, channel);
    let packages: Vec<&[u8]> = joined
        .iter()
        .map(|(_, package)| package.as_slice())
        .collect();
    add(creator_seed, &mut state, channel, &packages);
    let welcome = stored(creator_seed, &mut state, channel);
    let mut states = vec![state];
    for (&seed, (state, _)) in joiners.iter().zip(&mut joined) {
        assert_eq!(join(seed, state, &welcome), channel, "the channel joined");
        states.push(state.clone());
    }
    states
}

/// Device `seed` encrypts `plaintext` in `channel`; gives the message.
pub fn send(seed: u8, state: &mut SealedState, channel: ChannelId, plaintext: &[u8]) -> Vec<u8> {
    step(seed, state, |device| {
        bytes(
            device
                .encrypt(channel, plaintext)
                .expect("an application message"),
        )
    })
}

/// Device `seed` decrypts `message` of `channel`: its sender and plaintext, or `None` if refused.
pub fn receive(
    seed: u8,
    state: &mut SealedState,
    channel: ChannelId,
    message: &[u8],
) -> Option<(DeviceId, Vec<u8>)> {
    step(seed, state, |device| {
        let decrypted = device.decrypt(channel, message).ok()?;
        Some((decrypted.sender, decrypted.plaintext.to_vec()))
    })
}

/// The 16 bytes of `channel`'s history key in `epoch` as device `seed` holds it in `state`.
pub fn history_key(seed: u8, state: &SealedState, channel: ChannelId, epoch: u64) -> [u8; 16] {
    *load(seed, state)
        .history_key(channel, epoch)
        .unwrap_or_else(|| panic!("device {seed} holds the history key of epoch {epoch}"))
        .to_bytes()
}
