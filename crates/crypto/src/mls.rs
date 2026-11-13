//! MLS on a device, on mls-rs: KeyPackages and channel groups to `docs/protocol/membership.md`,
//! "MLS profile", formed as "Creating a channel" says, their messages, and the history key of each
//! epoch the device enters ("History key"). The device's state reaches the caller only sealed
//! ([`SealedState`]).

mod profile;
mod store;

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

use mls_rs::{
    ExtensionList, Group, MlsMessage, WireFormat, client_builder::MlsConfig,
    group::ReceivedMessage, identity::SigningIdentity,
};
use skald_proto::{ChannelId, DeviceId};
use zeroize::Zeroizing;

use crate::{SealedState, SigningKey, StateKey};
use store::{Kept, Store};

/// The exporter label of a history key.
const HISTORY_LABEL: &[u8] = b"skald history";
/// The name prefixes of the sealed items: a group by its ID, a KeyPackage's private keys by its
/// ID, a channel's history keys by the channel ID, and the Welcome of a channel's pending Commit by
/// the channel ID.
const GROUP_ITEM: &[u8] = b"group:";
const KEY_PACKAGE_ITEM: &[u8] = b"key-package:";
const HISTORY_ITEM: &[u8] = b"history:";
const WELCOME_ITEM: &[u8] = b"welcome:";

/// A device's MLS state: its groups, its KeyPackages' private keys, its history keys and the
/// Welcomes of its pending Commits.
///
/// Each step starts from the state the caller loads and ends with the state the caller seals and
/// saves, before it sends what the step produced; a failed step leaves the device as it was.
pub struct MlsDevice {
    signing: SigningKey,
    state_key: StateKey,
    id: DeviceId,
    store: Store,
    history: BTreeMap<ChannelId, BTreeMap<u64, HistoryKey>>,
    /// The Welcome of each channel's pending Commit, handed out once the server stored it.
    welcomes: BTreeMap<ChannelId, Vec<u8>>,
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
            welcomes: BTreeMap::new(),
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
        } else if let Some(id) = name.strip_prefix(WELCOME_ITEM) {
            let channel = ChannelId::from_bytes(id.try_into().ok()?);
            self.welcomes.insert(channel, plaintext.to_vec());
            Some(())
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

    /// The device's state, each group, KeyPackage, channel's history keys and pending Commit's
    /// Welcome an item sealed with a fresh nonce.
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
        for (channel, welcome) in &self.welcomes {
            seal([WELCOME_ITEM, channel.as_bytes()].concat(), welcome);
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
        self.save(channel, &mut group)
    }

    /// Adds the devices of `key_packages`, each in an `MLSMessage`, to `channel`'s group and
    /// commits; gives the Commit, a PublicMessage. The Commit stays pending until the caller says
    /// the server stored it ([`MlsDevice::commit_stored`]) or refused it
    /// ([`MlsDevice::commit_refused`]).
    ///
    /// # Errors
    /// [`MlsError::NoGroup`] if the device holds no group for `channel`; [`MlsError::Malformed`]
    /// if an item of `key_packages` is not a KeyPackage; [`MlsError::Mls`] if mls-rs refuses a
    /// KeyPackage, a Commit is pending already or mls-rs fails. One refused KeyPackage refuses
    /// them all.
    pub fn add_devices(
        &mut self,
        channel: ChannelId,
        key_packages: &[impl AsRef<[u8]>],
    ) -> Result<Vec<u8>, MlsError> {
        let mut group = self.group(channel)?;
        let mut commit = group.commit_builder();
        for package in key_packages {
            commit = commit.add_member(message(package.as_ref(), WireFormat::KeyPackage)?)?;
        }
        let output = commit.build()?;
        let commit = output.commit_message.to_bytes()?;
        let welcome = match output.welcome_messages.as_slice() {
            [] => None,
            [welcome] => Some(welcome.to_bytes()?),
            _ => unreachable!("mls-rs makes one Welcome for all the devices a Commit adds"),
        };
        group.write_to_storage()?;
        if let Some(welcome) = welcome {
            self.welcomes.insert(channel, welcome);
        }
        Ok(commit)
    }

    /// The server stored `channel`'s pending Commit: it merges, and the device keeps the new
    /// epoch's history key. Gives the Commit's Welcome, if it adds devices; it is handed out here
    /// only.
    ///
    /// # Errors
    /// [`MlsError::NoGroup`] if the device holds no group for `channel`;
    /// [`MlsError::NoCommitPending`] if no Commit is pending in it; [`MlsError::Mls`] if mls-rs
    /// fails.
    pub fn commit_stored(&mut self, channel: ChannelId) -> Result<Option<Vec<u8>>, MlsError> {
        let mut group = self.pending(channel)?;
        group.apply_pending_commit()?;
        self.save(channel, &mut group)?;
        Ok(self.welcomes.remove(&channel))
    }

    /// The server refused `channel`'s pending Commit: the device drops it and its Welcome. A
    /// refused Commit of epoch 0 drops the group too, as the server keeps another group of the
    /// channel, which the device joins from its Welcome.
    ///
    /// # Errors
    /// [`MlsError::NoGroup`] if the device holds no group for `channel`;
    /// [`MlsError::NoCommitPending`] if no Commit is pending in it; [`MlsError::Mls`] if mls-rs
    /// fails.
    pub fn commit_refused(&mut self, channel: ChannelId) -> Result<(), MlsError> {
        let mut group = self.pending(channel)?;
        if group.current_epoch() == 0 {
            self.store.remove_group(channel.as_bytes());
            self.history.remove(&channel);
        } else {
            group.clear_pending_commit();
            group.write_to_storage()?;
        }
        self.welcomes.remove(&channel);
        Ok(())
    }

    /// Joins a channel's group from `welcome`, an `MLSMessage`; gives the channel. `check` gets
    /// the Welcome's signer and members for the checks of "Group changes" before the group is
    /// saved; `false` refuses the Welcome.
    ///
    /// # Errors
    /// [`MlsError::Malformed`] if `welcome` is not a Welcome of a channel's group;
    /// [`MlsError::GroupExists`] if the device holds a group for its channel: mls-rs would replace
    /// it; [`MlsError::Refused`] if `check` refuses it; [`MlsError::Profile`] if a leaf breaks the
    /// profile; [`MlsError::Mls`] if mls-rs refuses it, such as one not for this device, or fails.
    /// A refused Welcome leaves no group.
    pub fn join(
        &mut self,
        welcome: &[u8],
        check: impl FnOnce(&Joining) -> bool,
    ) -> Result<ChannelId, MlsError> {
        let welcome = message(welcome, WireFormat::Welcome)?;
        let (mut group, info) = self.client().join_group(None, &welcome, None)?;
        let channel = ChannelId::from_bytes(
            group
                .group_id()
                .try_into()
                .map_err(|_| MlsError::Malformed)?,
        );
        if self.store.has_group(channel.as_bytes()) {
            return Err(MlsError::GroupExists(channel));
        }
        let members = members_of(&group)?;
        let signer = group
            .member_at_index(info.sender)
            .ok_or(MlsError::Malformed)?;
        let joining = Joining {
            channel,
            epoch: group.current_epoch(),
            signer: member_device(&signer.signing_identity)?,
            members,
        };
        if !check(&joining) {
            return Err(MlsError::Refused);
        }
        self.save(channel, &mut group)?;
        Ok(channel)
    }

    /// `plaintext` encrypted in `channel`'s group: an application message, a PrivateMessage.
    ///
    /// # Errors
    /// [`MlsError::NoGroup`] if the device holds no group for `channel`; [`MlsError::Mls`] if
    /// mls-rs fails.
    pub fn encrypt(&mut self, channel: ChannelId, plaintext: &[u8]) -> Result<Vec<u8>, MlsError> {
        let mut group = self.group(channel)?;
        let message = group
            .encrypt_application_message(plaintext, Vec::new())?
            .to_bytes()?;
        group.write_to_storage()?;
        Ok(message)
    }

    /// The application message `message`, a PrivateMessage of `channel`'s group, decrypted.
    ///
    /// # Errors
    /// [`MlsError::NoGroup`] if the device holds no group for `channel`; [`MlsError::Malformed`]
    /// if `message` is not a PrivateMessage or holds no application message; [`MlsError::Mls`] if
    /// mls-rs refuses it, such as a replay or a message of another group, or fails.
    pub fn decrypt(&mut self, channel: ChannelId, message: &[u8]) -> Result<Decrypted, MlsError> {
        let message = self::message(message, WireFormat::PrivateMessage)?;
        let mut group = self.group(channel)?;
        let ReceivedMessage::ApplicationMessage(received) =
            group.process_incoming_message(message)?
        else {
            return Err(MlsError::Malformed);
        };
        let sender = group
            .member_at_index(received.sender_index)
            .ok_or(MlsError::Malformed)?;
        let sender = member_device(&sender.signing_identity)?.device;
        group.write_to_storage()?;
        Ok(Decrypted {
            sender,
            plaintext: received.data().to_vec(),
        })
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
        members_of(&self.group(channel)?)
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

    /// `channel`'s group with its pending Commit.
    fn pending(&self, channel: ChannelId) -> Result<Group<impl MlsConfig + use<>>, MlsError> {
        let group = self.group(channel)?;
        if !group.has_pending_commit() {
            return Err(MlsError::NoCommitPending(channel));
        }
        Ok(group)
    }

    /// Saves `group` of `channel`, which entered its current epoch, and keeps the epoch's history
    /// key; on an error the device stays as it was.
    fn save(
        &mut self,
        channel: ChannelId,
        group: &mut Group<impl MlsConfig>,
    ) -> Result<(), MlsError> {
        let key = HistoryKey::of(group)?;
        group.write_to_storage()?;
        self.history
            .entry(channel)
            .or_default()
            .insert(group.current_epoch(), key);
        Ok(())
    }
}

/// `bytes` as one `MLSMessage` of `wire_format`.
fn message(bytes: &[u8], wire_format: WireFormat) -> Result<MlsMessage, MlsError> {
    let message = MlsMessage::from_bytes(bytes).map_err(|_| MlsError::Malformed)?;
    if message.wire_format() == wire_format {
        Ok(message)
    } else {
        Err(MlsError::Malformed)
    }
}

/// The member devices of `group`, in leaf order.
fn members_of(group: &Group<impl MlsConfig>) -> Result<Vec<MemberDevice>, MlsError> {
    group
        .roster()
        .members_iter()
        .map(|member| member_device(&member.signing_identity))
        .collect()
}

fn member_device(leaf: &SigningIdentity) -> Result<MemberDevice, MlsError> {
    let (device, signing) = profile::device_of(leaf).map_err(|_| MlsError::Profile)?;
    Ok(MemberDevice { device, signing })
}

/// A channel's group a device joins from a Welcome, before the group is saved: what the caller
/// checks as "Group changes" says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Joining {
    pub channel: ChannelId,
    pub epoch: u64,
    /// The member that signed the Welcome's GroupInfo.
    pub signer: MemberDevice,
    /// Every member, in leaf order.
    pub members: Vec<MemberDevice>,
}

/// An application message a device decrypted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decrypted {
    /// The device ID of the sender's credential.
    pub sender: DeviceId,
    pub plaintext: Vec<u8>,
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
    /// No Commit is pending in the channel's group.
    NoCommitPending(ChannelId),
    /// The caller refused the Welcome.
    Refused,
    /// The input is not an MLS message of the kind the step takes, or not of a channel's group.
    Malformed,
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
            Self::NoCommitPending(channel) => {
                write!(f, "no Commit pending in the channel {channel}")
            }
            Self::Refused => f.write_str("the caller refused the Welcome"),
            Self::Malformed => f.write_str("not an MLS message of the kind the step takes"),
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

    /// Device `seed`, a new one.
    fn seeded(seed: u8) -> MlsDevice {
        MlsDevice::new(
            &SigningKey::from_bytes(&[seed; 32]),
            &StateKey::from_bytes(&[seed; 32]),
        )
    }

    /// `device` reloaded from the state it seals.
    fn reloaded(device: &MlsDevice) -> MlsDevice {
        MlsDevice::load(&device.signing, &device.state_key, &device.seal()).unwrap()
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
            (named(WELCOME_ITEM, &channel), b"a Welcome".to_vec()),
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
            (named(WELCOME_ITEM, &channel[..15]), b"a Welcome".to_vec()),
            (named(WELCOME_ITEM, &[7; 17]), b"a Welcome".to_vec()),
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
    fn a_refused_commit_after_epoch_zero_leaves_the_group_in_its_epoch() {
        let channel = ChannelId::from_bytes([6; 16]);
        let mut creator = seeded(11);
        let first = seeded(12).key_package().unwrap();
        let second = seeded(13).key_package().unwrap();
        creator.create_group(channel).unwrap();
        creator.add_devices(channel, &[&first]).unwrap();
        creator.commit_stored(channel).unwrap();
        let members = creator.members(channel).unwrap();
        let key = creator.history_key(channel, 1).unwrap().to_bytes();

        creator.add_devices(channel, &[&second]).unwrap();
        creator = reloaded(&creator);
        creator.commit_refused(channel).unwrap();
        creator = reloaded(&creator);
        assert_eq!(creator.epoch(channel).unwrap(), 1);
        assert_eq!(creator.members(channel).unwrap(), members);
        assert_eq!(creator.history_key(channel, 1).unwrap().to_bytes(), key);
        assert!(matches!(
            creator.commit_stored(channel),
            Err(MlsError::NoCommitPending(_))
        ));

        creator.add_devices(channel, &[&second]).unwrap();
        assert!(creator.commit_stored(channel).unwrap().is_some());
        assert_eq!(creator.epoch(channel).unwrap(), 2);
        assert_eq!(creator.members(channel).unwrap().len(), 3);
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn only_a_private_message_is_decrypted() {
        let channel = ChannelId::from_bytes([8; 16]);
        let mut creator = seeded(14);
        creator.create_group(channel).unwrap();
        let commit = creator
            .add_devices(channel, &[seeded(15).key_package().unwrap()])
            .unwrap();
        assert!(matches!(
            creator.decrypt(channel, &commit),
            Err(MlsError::Malformed)
        ));
        let welcome = creator.commit_stored(channel).unwrap().unwrap();
        assert_eq!(creator.epoch(channel).unwrap(), 1, "the Commit was pending");
        assert!(matches!(
            creator.decrypt(channel, &welcome),
            Err(MlsError::Malformed)
        ));
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn a_welcome_to_a_group_whose_id_is_no_channel_id_is_refused() {
        let mut joiner = seeded(16);
        let package = MlsMessage::from_bytes(&joiner.key_package().unwrap()).unwrap();
        let other = seeded(17);
        let mut group = other
            .client()
            .create_group_with_id(
                vec![9; 15],
                ExtensionList::new(),
                ExtensionList::new(),
                None,
            )
            .unwrap();
        let output = group
            .commit_builder()
            .add_member(package)
            .unwrap()
            .build()
            .unwrap();
        let welcome = output.welcome_messages[0].to_bytes().unwrap();
        assert!(matches!(
            joiner.join(&welcome, |_| true),
            Err(MlsError::Malformed)
        ));
        assert!(
            joiner
                .store
                .items()
                .iter()
                .all(|(kept, _)| matches!(kept, Kept::KeyPackage(_)))
        );
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
            MlsError::NoCommitPending(channel),
            MlsError::Refused,
            MlsError::Malformed,
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
        assert_eq!(messages.len(), 8, "{messages:?}");
        assert!(messages.contains(&format!("no group for the channel {channel}")));
    }
}
