//! The store mls-rs keeps a device's groups and KeyPackages in: mls-rs's in-memory storages have
//! no save or load, so this one gives each group and each KeyPackage as an item to seal, and takes
//! them back. It keeps a group's 3 prior epochs, as mls-rs's own stores do by default, and deletes
//! older ones: `docs/protocol/membership.md`, "MLS profile".

use std::{
    collections::BTreeMap,
    convert::Infallible,
    sync::{Arc, Mutex, MutexGuard},
};

use mls_rs::{
    GroupStateStorage, KeyPackageStorage,
    mls_rs_codec::{MlsDecode as _, MlsEncode as _},
    storage_provider::KeyPackageData,
};
use mls_rs_core::group::{EpochRecord, GroupState};
use zeroize::Zeroizing;

/// The groups and KeyPackages of one device. Each clone shares them: mls-rs takes the store by
/// value.
#[derive(Clone, Default)]
pub(super) struct Store(Arc<Mutex<Contents>>);

#[derive(Default)]
struct Contents {
    groups: BTreeMap<Vec<u8>, Group>,
    key_packages: BTreeMap<Vec<u8>, KeyPackageData>,
}

/// How many prior epochs of a group the store keeps.
const KEPT_EPOCHS: usize = 3;

/// A group's state and its prior epochs, as mls-rs writes them.
#[derive(Default)]
struct Group {
    state: Zeroizing<Vec<u8>>,
    epochs: BTreeMap<u64, Zeroizing<Vec<u8>>>,
}

/// What an item to seal holds: a group or a KeyPackage's private keys, by its ID.
pub(super) enum Kept {
    Group(Vec<u8>),
    KeyPackage(Vec<u8>),
}

impl Store {
    fn contents(&self) -> MutexGuard<'_, Contents> {
        self.0
            .lock()
            .expect("no step panics while it holds the store")
    }

    pub(super) fn has_group(&self, id: &[u8]) -> bool {
        self.contents().groups.contains_key(id)
    }

    /// Drops group `id` with its prior epochs.
    pub(super) fn remove_group(&self, id: &[u8]) {
        self.contents().groups.remove(id);
    }

    /// Deletes the private keys of each KeyPackage whose lifetime ended before `now`.
    pub(super) fn delete_expired(&self, now: u64) {
        self.contents()
            .key_packages
            .retain(|_, data| data.expiration >= now);
    }

    /// Each group and KeyPackage with its encoding, in the order of their IDs.
    pub(super) fn items(&self) -> Vec<(Kept, Zeroizing<Vec<u8>>)> {
        let contents = self.contents();
        let groups = contents
            .groups
            .iter()
            .map(|(id, group)| (Kept::Group(id.clone()), group.encode()));
        let key_packages = contents.key_packages.iter().map(|(id, data)| {
            let encoded = data
                .mls_encode_to_vec()
                .expect("a KeyPackage's data encodes");
            (Kept::KeyPackage(id.clone()), Zeroizing::new(encoded))
        });
        groups.chain(key_packages).collect()
    }

    /// Takes back an item [`Store::items`] gave; `None` if `encoded` is not one.
    pub(super) fn restore(&self, kept: Kept, encoded: &[u8]) -> Option<()> {
        let mut contents = self.contents();
        match kept {
            Kept::Group(id) => {
                contents.groups.insert(id, Group::decode(encoded)?);
            }
            Kept::KeyPackage(id) => {
                let mut bytes = encoded;
                let data = KeyPackageData::mls_decode(&mut bytes).ok()?;
                if !bytes.is_empty() {
                    return None;
                }
                contents.key_packages.insert(id, data);
            }
        }
        Some(())
    }
}

impl Group {
    /// The state's length as 4 bytes, the state, then each epoch: its ID as 8 bytes, its record's
    /// length as 4 bytes and the record; all big-endian. Sized up front, so no copy of the secrets
    /// is left behind by a reallocation.
    fn encode(&self) -> Zeroizing<Vec<u8>> {
        let len = 4
            + self.state.len()
            + (self.epochs.values())
                .map(|record| 8 + 4 + record.len())
                .sum::<usize>();
        let mut out = Zeroizing::new(Vec::with_capacity(len));
        put(&mut out, &self.state);
        for (id, record) in &self.epochs {
            out.extend_from_slice(&id.to_be_bytes());
            put(&mut out, record);
        }
        out
    }

    fn decode(mut bytes: &[u8]) -> Option<Self> {
        let mut group = Self {
            state: take(&mut bytes)?,
            epochs: BTreeMap::new(),
        };
        while !bytes.is_empty() {
            let (id, rest) = bytes.split_first_chunk::<8>()?;
            bytes = rest;
            let record = take(&mut bytes)?;
            if group
                .epochs
                .insert(u64::from_be_bytes(*id), record)
                .is_some()
            {
                return None;
            }
        }
        Some(group)
    }
}

/// Appends `bytes` with its length.
fn put(out: &mut Vec<u8>, bytes: &[u8]) {
    let len = u32::try_from(bytes.len()).expect("a group state below 4 GiB");
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(bytes);
}

/// Takes bytes [`put`] appended from the start of `bytes`.
fn take(bytes: &mut &[u8]) -> Option<Zeroizing<Vec<u8>>> {
    let (len, rest) = bytes.split_first_chunk::<4>()?;
    let (taken, rest) = rest.split_at_checked(usize::try_from(u32::from_be_bytes(*len)).ok()?)?;
    *bytes = rest;
    Some(Zeroizing::new(taken.to_vec()))
}

impl GroupStateStorage for Store {
    type Error = Infallible;

    fn state(&self, group_id: &[u8]) -> Result<Option<Zeroizing<Vec<u8>>>, Infallible> {
        Ok(self
            .contents()
            .groups
            .get(group_id)
            .map(|group| group.state.clone()))
    }

    fn epoch(
        &self,
        group_id: &[u8],
        epoch_id: u64,
    ) -> Result<Option<Zeroizing<Vec<u8>>>, Infallible> {
        let contents = self.contents();
        let record = contents
            .groups
            .get(group_id)
            .and_then(|group| group.epochs.get(&epoch_id));
        Ok(record.cloned())
    }

    fn write(
        &mut self,
        state: GroupState,
        epoch_inserts: Vec<EpochRecord>,
        epoch_updates: Vec<EpochRecord>,
    ) -> Result<(), Infallible> {
        let mut contents = self.contents();
        let group = contents.groups.entry(state.id).or_default();
        group.state = state.data;
        for record in epoch_inserts.into_iter().chain(epoch_updates) {
            group.epochs.insert(record.id, record.data);
        }
        while group.epochs.len() > KEPT_EPOCHS {
            group.epochs.pop_first();
        }
        Ok(())
    }

    fn max_epoch_id(&self, group_id: &[u8]) -> Result<Option<u64>, Infallible> {
        let contents = self.contents();
        let group = contents.groups.get(group_id);
        Ok(group.and_then(|group| group.epochs.last_key_value().map(|(id, _)| *id)))
    }
}

impl KeyPackageStorage for Store {
    type Error = Infallible;

    fn delete(&mut self, id: &[u8]) -> Result<(), Infallible> {
        self.contents().key_packages.remove(id);
        Ok(())
    }

    fn insert(&mut self, id: Vec<u8>, data: KeyPackageData) -> Result<(), Infallible> {
        self.contents().key_packages.insert(id, data);
        Ok(())
    }

    fn get(&self, id: &[u8]) -> Result<Option<KeyPackageData>, Infallible> {
        Ok(self.contents().key_packages.get(id).cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &[u8] = b"a group";

    fn record(id: u64, data: &[u8]) -> EpochRecord {
        EpochRecord::new(id, Zeroizing::new(data.to_vec()))
    }

    /// A store with group [`ID`] in its state `state`, with epochs 1 and 2 and epoch 2 updated.
    fn store_with_a_group() -> Store {
        let mut store = Store::default();
        let state = GroupState {
            id: ID.to_vec(),
            data: Zeroizing::new(b"state".to_vec()),
        };
        store
            .write(
                state,
                vec![record(1, b"one"), record(2, b"two")],
                vec![record(2, b"two, updated")],
            )
            .unwrap();
        store
    }

    fn assert_holds_the_group(store: &Store) {
        assert!(store.has_group(ID));
        assert_eq!(
            store.state(ID).unwrap().as_deref(),
            Some(&b"state".to_vec())
        );
        assert_eq!(
            store.epoch(ID, 1).unwrap().as_deref(),
            Some(&b"one".to_vec())
        );
        assert_eq!(
            store.epoch(ID, 2).unwrap().as_deref(),
            Some(&b"two, updated".to_vec())
        );
        assert_eq!(store.epoch(ID, 3).unwrap(), None);
        assert_eq!(store.max_epoch_id(ID).unwrap(), Some(2));
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn a_group_reads_back_as_written_and_restored() {
        let store = store_with_a_group();
        assert_holds_the_group(&store);
        assert!(!store.has_group(b"another group"));
        assert_eq!(store.state(b"another group").unwrap(), None);
        assert_eq!(store.epoch(b"another group", 1).unwrap(), None);
        assert_eq!(store.max_epoch_id(b"another group").unwrap(), None);

        let restored = Store::default();
        for (kept, encoded) in store.items() {
            restored.restore(kept, &encoded).expect("its own item");
        }
        assert_holds_the_group(&restored);
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn a_malformed_group_is_refused() {
        let [(_, encoded)] = <[_; 1]>::try_from(store_with_a_group().items())
            .ok()
            .unwrap();
        assert!(Group::decode(&encoded).is_some());
        assert_eq!(encoded.capacity(), encoded.len(), "sized up front");
        // The state's length and bytes end at 9, epoch 1's ID, length and record at 24.
        let (state_end, first_end) = (4 + 5, 4 + 5 + 8 + 4 + 3);
        for cut in (0..encoded.len()).filter(|cut| ![state_end, first_end].contains(cut)) {
            assert!(
                Group::decode(&encoded[..cut]).is_none(),
                "cut to {cut} bytes"
            );
        }
        let mut repeated = encoded.to_vec();
        repeated.extend_from_slice(&encoded[state_end..first_end]);
        assert!(Group::decode(&repeated).is_none(), "epoch 1 twice");
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn a_deleted_key_package_is_gone() {
        let mut store = Store::default();
        let data = KeyPackageData::new(Vec::new(), Vec::new().into(), Vec::new().into(), 0);
        KeyPackageStorage::insert(&mut store, b"a".to_vec(), data.clone()).unwrap();
        KeyPackageStorage::insert(&mut store, b"b".to_vec(), data.clone()).unwrap();
        store.delete(b"a").unwrap();
        assert_eq!(store.get(b"a").unwrap(), None);
        assert_eq!(store.get(b"b").unwrap(), Some(data));
        assert_eq!(store.items().len(), 1);
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn a_key_package_is_deleted_once_its_lifetime_ended_before_the_time_given() {
        let mut store = Store::default();
        for (id, expiration) in [(b"a", 9), (b"b", 10), (b"c", 11)] {
            let data =
                KeyPackageData::new(Vec::new(), Vec::new().into(), Vec::new().into(), expiration);
            KeyPackageStorage::insert(&mut store, id.to_vec(), data).unwrap();
        }
        store.delete_expired(10);
        assert_eq!(store.get(b"a").unwrap(), None, "ended before");
        assert!(store.get(b"b").unwrap().is_some(), "ends at the time given");
        assert!(store.get(b"c").unwrap().is_some(), "ends after");
    }
}
