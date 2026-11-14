//! Devices, keys, certificates, the caller's time and sealed states for the tests.

use std::sync::LazyLock;

use skald_crypto::{MlsDevice, SealedState, SigningKey, StateKey, issue};
use skald_proto::{AccountId, Cert, CertBody, ChannelId, DeviceId, DeviceKind, Unknown};

/// The caller's time of the steps, in seconds since the Unix epoch, far from the machine's clock.
pub const NOW: u64 = 1_800_000_000;
/// The seed of the key that issues the certificates of [`admitted`].
const MASTER: u8 = 255;

/// The signing key of device `seed`, as it comes from key storage.
pub fn signing_key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

pub fn device_id(seed: u8) -> DeviceId {
    DeviceId::from_signing_key(&signing_key(seed).public_key())
}

/// The valid certificates of the tests' devices, seeds 0 to 99, as the caller gives the devices
/// it admits.
pub fn admitted() -> &'static [Cert] {
    static CERTS: LazyLock<Vec<Cert>> = LazyLock::new(|| {
        (0..100)
            .map(|seed| {
                let key = signing_key(seed);
                let body = CertBody {
                    device: device_id(seed),
                    signing: key.public_key(),
                    account: AccountId::from_bytes([MASTER; 16]),
                    kind: DeviceKind::Client,
                    name: format!("Device {seed}"),
                    hpke: [seed; 32],
                    issuer: None,
                    issued: NOW,
                    ssh: None,
                    unknown: Unknown::default(),
                };
                issue(&body, &signing_key(MASTER), &key).expect("a test device's certificate")
            })
            .collect()
    });
    &CERTS
}

/// The state key of `seed`, as it comes from key storage.
pub fn state_key(seed: u8) -> StateKey {
    StateKey::from_bytes(&[seed; 32])
}

/// Device `seed` with its state key `seed`, loaded from `state`.
pub fn load(seed: u8, state: &SealedState) -> MlsDevice {
    MlsDevice::load(&signing_key(seed), &state_key(seed), state)
        .expect("the device's own sealed state loads")
}

/// A new device `seed`'s state after `step`: loaded from its sealed state, stepped and sealed.
pub fn after(seed: u8, state: &SealedState, step: impl FnOnce(&mut MlsDevice)) -> SealedState {
    let mut device = load(seed, state);
    step(&mut device);
    device.seal()
}

/// The sealed state of a new device `seed`.
pub fn new_state(seed: u8) -> SealedState {
    MlsDevice::new(&signing_key(seed), &state_key(seed)).seal()
}

/// The sealed state of device `seed` with a KeyPackage and a group for each of `channels`,
/// reloaded between steps.
pub fn state_with_groups(seed: u8, channels: &[ChannelId]) -> SealedState {
    let mut state = after(seed, &new_state(seed), |device| {
        device.key_package(NOW).expect("a KeyPackage");
    });
    for &channel in channels {
        state = after(seed, &state, |device| {
            device.create_group(channel, NOW).expect("a new group");
        });
    }
    state
}

/// The items of `state`, each its name and its sealed bytes.
pub fn items(state: &SealedState) -> Vec<(Vec<u8>, Vec<u8>)> {
    state
        .items()
        .map(|(name, sealed)| (name.to_vec(), sealed.to_vec()))
        .collect()
}

/// A sealed state of these items.
pub fn state_of(items: Vec<(Vec<u8>, Vec<u8>)>) -> SealedState {
    SealedState::from_items(items)
}

/// Whether any item of `state`, its name or its sealed bytes, holds `needle`.
pub fn holds(state: &SealedState, needle: &[u8]) -> bool {
    items(state).iter().any(|(name, sealed)| {
        [name, sealed]
            .iter()
            .any(|bytes| bytes.windows(needle.len()).any(|window| window == needle))
    })
}
