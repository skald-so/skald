//! MLS on a device, on mls-rs: KeyPackages and channel groups to `docs/protocol/membership.md`,
//! "MLS profile", and the history key of each epoch the device enters ("History key"). The
//! device's state reaches the caller only sealed ([`SealedState`]).

mod profile;
mod store;

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

use mls_rs::{ExtensionList, Group, client_builder::MlsConfig};
use skald_proto::{ChannelId, DeviceId};
use zeroize::Zeroizing;

use crate::{SealedState, SigningKey, StateKey};
use store::{Kept, Store};

/// The exporter label of a history key.
const HISTORY_LABEL: &[u8] = b"skald history";
/// The name prefixes of the sealed items: a group by its ID, a KeyPackage's private keys by its
/// ID, and a channel's history keys by the channel ID.
const GROUP_ITEM: &[u8] = b"group:";
const KEY_PACKAGE_ITEM: &[u8] = b"key-package:";
const HISTORY_ITEM: &[u8] = b"history:";

/// A device's MLS state: its groups, its KeyPackages' private keys and its history keys.
///
/// Each step starts from the state the caller loads and ends with the state the caller seals and
/// saves, before it sends what the step produced; a failed step leaves the device as it was.
pub struct MlsDevice {
    signing: SigningKey,
    state_key: StateKey,
    id: DeviceId,
    store: Store,
    history: BTreeMap<ChannelId, BTreeMap<u64, HistoryKey>>,
}

impl MlsDevice {
    /// A device with no KeyPackage or group yet, which signs with `signing` and seals its state
    /// under `state_key`.
    pub fn new(signing: &SigningKey, state_key: &StateKey) -> Self {
        Self {
            signing: SigningKey::from_bytes(&signing.to_bytes()),
            state_key: StateKey::from_bytes(&state_key.to_bytes()),
            id: DeviceId::from_signing_key(&signing.public_key()),
            store: Store::default(),
            history: BTreeMap::new(),
        }
    }

    /// The device with the state [`MlsDevice::seal`] gave.
    ///
    /// # Errors
    /// [`MlsError::SealedState`] if `state` is not the device's own as it sealed it under
    /// `state_key`: an item is altered, sealed under another key, by another device or under
    /// another name, or named twice.
    pub fn load(
        signing: &SigningKey,
        state_key: &StateKey,
        state: &SealedState,
    ) -> Result<Self, MlsError> {
        let mut device = Self::new(signing, state_key);
        let mut names = BTreeSet::new();
        for (name, sealed) in state.items() {
            if !names.insert(name) {
                return Err(MlsError::SealedState);
            }
            let plaintext = device
                .state_key
                .open(device.id, name, sealed)
                .ok_or(MlsError::SealedState)?;
            device
                .restore(name, &plaintext)
                .ok_or(MlsError::SealedState)?;
        }
        Ok(device)
    }

    fn restore(&mut self, name: &[u8], plaintext: &[u8]) -> Option<()> {
        if let Some(id) = name.strip_prefix(GROUP_ITEM) {
            self.store.restore(Kept::Group(id.to_vec()), plaintext)
        } else if let Some(id) = name.strip_prefix(KEY_PACKAGE_ITEM) {
            self.store.restore(Kept::KeyPackage(id.to_vec()), plaintext)
        } else {
            let channel = ChannelId::from_bytes(name.strip_prefix(HISTORY_ITEM)?.try_into().ok()?);
            let keys = plaintext
                .chunks(HistoryKey::ENCODED_LEN)
                .map(HistoryKey::decode)
                .collect::<Option<_>>()?;
            self.history.insert(channel, keys);
            Some(())
        }
    }

    /// The device's state, each group, KeyPackage and channel's history keys an item sealed with a
    /// fresh nonce.
    ///
    /// # Panics
    /// If the random generator fails.
    pub fn seal(&self) -> SealedState {
        let mut state = SealedState::default();
        let mut seal = |name: Vec<u8>, plaintext: &[u8]| {
            let sealed = self.state_key.seal(self.id, &name, plaintext);
            state.push(name, sealed);
        };
        for (kept, plaintext) in self.store.items() {
            let (prefix, id) = match kept {
                Kept::Group(id) => (GROUP_ITEM, id),
                Kept::KeyPackage(id) => (KEY_PACKAGE_ITEM, id),
            };
            seal([prefix, &id].concat(), &plaintext);
        }
        for (channel, keys) in &self.history {
            let mut plaintext =
                Zeroizing::new(Vec::with_capacity(keys.len() * HistoryKey::ENCODED_LEN));
            for (epoch, key) in keys {
                plaintext.extend_from_slice(&epoch.to_be_bytes());
                plaintext.extend_from_slice(key.0.as_ref());
            }
            seal([HISTORY_ITEM, channel.as_bytes()].concat(), &plaintext);
        }
        state
    }

    /// The device ID, derived from the signing key.
    pub fn id(&self) -> DeviceId {
        self.id
    }

    /// A new KeyPackage, in an `MLSMessage`; its private keys join the device's state.
    ///
    /// # Errors
    /// [`MlsError::Mls`] if mls-rs fails.
    pub fn key_package(&mut self) -> Result<Vec<u8>, MlsError> {
        let message = self.client().generate_key_package_message(
            ExtensionList::new(),
            ExtensionList::new(),
            None,
        )?;
        Ok(message.to_bytes()?)
    }

    /// Creates `channel`'s group at epoch 0, its `group_id` the channel ID and the device its one
    /// member, and keeps the history key of epoch 0.
    ///
    /// # Errors
    /// [`MlsError::GroupExists`] if the device holds a group for `channel`, which stays as it was;
    /// [`MlsError::Mls`] if mls-rs fails.
    pub fn create_group(&mut self, channel: ChannelId) -> Result<(), MlsError> {
        if self.store.has_group(channel.as_bytes()) {
            return Err(MlsError::GroupExists(channel));
        }
        let mut group = self.client().create_group_with_id(
            channel.as_bytes().to_vec(),
            ExtensionList::new(),
            ExtensionList::new(),
            None,
        )?;
        let key = HistoryKey::of(&group)?;
        group.write_to_storage()?;
        self.history
            .entry(channel)
            .or_default()
            .insert(group.current_epoch(), key);
        Ok(())
    }

    /// The epoch of `channel`'s group.
    ///
    /// # Errors
    /// [`MlsError::NoGroup`] if the device holds no group for `channel`; [`MlsError::Mls`] if
    /// mls-rs fails.
    pub fn epoch(&self, channel: ChannelId) -> Result<u64, MlsError> {
        Ok(self.group(channel)?.current_epoch())
    }

    /// The member devices of `channel`'s group, in leaf order.
    ///
    /// # Errors
    /// [`MlsError::NoGroup`] if the device holds no group for `channel`; [`MlsError::Mls`] if
    /// mls-rs fails.
    pub fn members(&self, channel: ChannelId) -> Result<Vec<MemberDevice>, MlsError> {
        self.group(channel)?
            .roster()
            .members_iter()
            .map(|member| {
                let (device, signing) =
                    profile::device_of(&member.signing_identity).map_err(|_| MlsError::Profile)?;
                Ok(MemberDevice { device, signing })
            })
            .collect()
    }

    /// The history key of `channel` in `epoch`, if the device holds it.
    pub fn history_key(&self, channel: ChannelId, epoch: u64) -> Option<&HistoryKey> {
        self.history.get(&channel)?.get(&epoch)
    }

    fn client(&self) -> mls_rs::Client<impl MlsConfig + use<>> {
        profile::client(&self.signing, self.id, &self.store)
    }

    fn group(&self, channel: ChannelId) -> Result<Group<impl MlsConfig + use<>>, MlsError> {
        if !self.store.has_group(channel.as_bytes()) {
            return Err(MlsError::NoGroup(channel));
        }
        Ok(self.client().load_group(channel.as_bytes())?)
    }
}

/// A member device of a group: the device ID and the signature key of its leaf.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemberDevice {
    pub device: DeviceId,
    /// The Ed25519 public signing key.
    pub signing: [u8; 32],
}

/// The history key of an epoch of a channel's group: `docs/protocol/membership.md`, "History
/// key". Its bytes are zeroed on drop and never shown.
pub struct HistoryKey(Zeroizing<[u8; 16]>);

impl HistoryKey {
    /// The epoch's 8 bytes and the key's 16, as a channel's history item holds them.
    const ENCODED_LEN: usize = 24;

    /// `MLS-Exporter("skald history", "", 16)` in `group`'s current epoch.
    fn of(group: &Group<impl MlsConfig>) -> Result<Self, MlsError> {
        let secret = group.export_secret(HISTORY_LABEL, b"", 16)?;
        let mut key = Zeroizing::new([0; 16]);
        key.copy_from_slice(secret.as_bytes());
        Ok(Self(key))
    }

    fn decode(encoded: &[u8]) -> Option<(u64, Self)> {
        let (epoch, key) = encoded.split_first_chunk::<8>()?;
        let key: &[u8; 16] = key.try_into().ok()?;
        Some((u64::from_be_bytes(*epoch), Self(Zeroizing::new(*key))))
    }

    /// The key's 16 bytes.
    pub fn to_bytes(&self) -> Zeroizing<[u8; 16]> {
        self.0.clone()
    }
}

impl fmt::Debug for HistoryKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("HistoryKey(..)")
    }
}

/// Why an MLS step of a device fails.
#[derive(Debug)]
#[non_exhaustive]
pub enum MlsError {
    /// The sealed state is not the device's own as it sealed it.
    SealedState,
    /// The device holds no group for the channel.
    NoGroup(ChannelId),
    /// The device holds a group for the channel already.
    GroupExists(ChannelId),
    /// A leaf breaks the MLS profile.
    Profile,
    /// mls-rs refuses the step or fails.
    Mls(mls_rs::error::MlsError),
}

impl fmt::Display for MlsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SealedState => f.write_str("the sealed state is not the device's own"),
            Self::NoGroup(channel) => write!(f, "no group for the channel {channel}"),
            Self::GroupExists(channel) => write!(f, "a group for the channel {channel} already"),
            Self::Profile => f.write_str("a leaf breaks the MLS profile"),
            Self::Mls(error) => write!(f, "mls-rs: {error}"),
        }
    }
}

impl std::error::Error for MlsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Mls(error) => Some(error),
            _ => None,
        }
    }
}

impl From<mls_rs::error::MlsError> for MlsError {
    fn from(error: mls_rs::error::MlsError) -> Self {
        Self::Mls(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device() -> MlsDevice {
        MlsDevice::new(
            &SigningKey::from_bytes(&[1; 32]),
            &StateKey::from_bytes(&[2; 32]),
        )
    }

    /// Whether `device` loads `items`, each a name and the plaintext it seals under that name.
    fn loads(device: &MlsDevice, items: &[(Vec<u8>, Vec<u8>)]) -> bool {
        let state = SealedState::from_items(items.iter().map(|(name, plaintext)| {
            let sealed = device.state_key.seal(device.id, name, plaintext);
            (name.clone(), sealed)
        }));
        MlsDevice::load(&device.signing, &device.state_key, &state).is_ok()
    }

    fn named(prefix: &[u8], id: &[u8]) -> Vec<u8> {
        [prefix, id].concat()
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn an_item_the_device_did_not_seal_as_it_seals_its_own_is_refused() {
        let mut device = device();
        device.key_package().expect("a KeyPackage");
        let [(Kept::KeyPackage(id), data)] = <[_; 1]>::try_from(device.store.items()).ok().unwrap()
        else {
            panic!("one KeyPackage");
        };
        let data = data.to_vec();
        let key_package = named(KEY_PACKAGE_ITEM, &id);
        let channel = [7; 16];
        let history = named(HISTORY_ITEM, &channel);
        let key = [[0; 8].as_slice(), &[9; 16]].concat();

        for (name, plaintext) in [
            (key_package.clone(), data.clone()),
            (named(GROUP_ITEM, b"a group"), vec![0; 4]),
            (history.clone(), key.clone()),
            (history.clone(), Vec::new()),
        ] {
            assert!(loads(&device, &[(name.clone(), plaintext)]), "{name:?}");
        }
        for (name, plaintext) in [
            (b"other:".to_vec(), Vec::new()),
            (Vec::new(), Vec::new()),
            (key_package.clone(), b"not a KeyPackage".to_vec()),
            (key_package, [data.as_slice(), &[0]].concat()),
            (named(GROUP_ITEM, b"a group"), vec![0; 3]),
            (named(GROUP_ITEM, b"a group"), vec![0, 0, 0, 1]),
            (named(HISTORY_ITEM, &channel[..15]), key.clone()),
            (named(HISTORY_ITEM, &[7; 17]), key.clone()),
            (history.clone(), key[..23].to_vec()),
            (history, [key.as_slice(), &[0]].concat()),
        ] {
            assert!(!loads(&device, &[(name.clone(), plaintext)]), "{name:?}");
        }
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn an_item_named_twice_is_refused() {
        let mut device = device();
        device.create_group(ChannelId::from_bytes([3; 16])).unwrap();
        let items: Vec<(Vec<u8>, Vec<u8>)> = device
            .seal()
            .items()
            .map(|(name, sealed)| (name.to_vec(), sealed.to_vec()))
            .collect();
        let load = |items: Vec<(Vec<u8>, Vec<u8>)>| {
            MlsDevice::load(
                &device.signing,
                &device.state_key,
                &SealedState::from_items(items),
            )
        };
        assert!(load(items.clone()).is_ok());
        let later: Vec<(Vec<u8>, Vec<u8>)> = load(items.clone())
            .unwrap()
            .seal()
            .items()
            .map(|(name, sealed)| (name.to_vec(), sealed.to_vec()))
            .collect();
        for at in 0..items.len() {
            for again in [&items[at], &later[at]] {
                let mut twice = items.clone();
                twice.push(again.clone());
                assert!(load(twice).is_err(), "item {at} twice");
            }
        }
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn a_history_key_is_the_epochs_mls_exporter_secret() {
        let mut device = device();
        let channel = ChannelId::from_bytes([5; 16]);
        device.create_group(channel).unwrap();
        let group = device.group(channel).unwrap();
        assert_eq!(group.current_epoch(), 0);
        let exported = group.export_secret(b"skald history", b"", 16).unwrap();
        let key = device.history_key(channel, 0).expect("epoch 0's key");
        assert_eq!(key.to_bytes().as_slice(), exported.as_bytes());
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn each_error_says_what_failed() {
        let channel = ChannelId::from_bytes([4; 16]);
        let messages: BTreeSet<String> = [
            MlsError::SealedState,
            MlsError::NoGroup(channel),
            MlsError::GroupExists(channel),
            MlsError::Profile,
            MlsError::Mls(mls_rs::error::MlsError::GroupNotFound),
        ]
        .iter()
        .map(|error| {
            let message = error.to_string();
            assert!(!message.is_empty());
            assert_eq!(
                std::error::Error::source(error).is_some(),
                matches!(error, MlsError::Mls(_)),
                "{error:?}"
            );
            message
        })
        .collect();
        assert_eq!(messages.len(), 5, "{messages:?}");
        assert!(messages.contains(&format!("no group for the channel {channel}")));
    }
}
