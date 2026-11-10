//! Skald's MLS prototype on mls-rs with its RustCrypto provider.

use std::{
    collections::BTreeMap,
    convert::Infallible,
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};

use mls_rs::{
    CipherSuite, Client, ExtensionList, GroupStateStorage, IdentityProvider, KeyPackageStorage,
    MlsMessage,
    client_builder::MlsConfig,
    crypto::{SignaturePublicKey, SignatureSecretKey},
    error::IntoAnyError,
    extension::{ExtensionType, MlsExtension, recommended::LastResortKeyPackageExt},
    external_client::{
        ExternalClient, ExternalGroup, ExternalReceivedMessage,
        builder::{ExternalBaseConfig, WithCryptoProvider, WithIdentityProvider},
    },
    group::{
        CommitEffect, CommitOutput, ReceivedMessage,
        mls_rules::{CommitOptions, DefaultMlsRules},
        proposal::Proposal,
    },
    identity::{
        CredentialType, SigningIdentity,
        basic::{BasicCredential, BasicIdentityProvider, BasicIdentityProviderError},
    },
    mls_rs_codec::{MlsDecode, MlsEncode},
    storage_provider::KeyPackageData,
    time::MlsTime,
};
use mls_rs_core::{
    group::{EpochRecord, GroupState},
    identity::MemberValidationContext,
};
use mls_rs_crypto_rustcrypto::RustCryptoProvider;
use mls_scenario::{
    Admitted, Commit, Device, HISTORY_LABEL, HistoryKey, Identity, Leaf, Observer, Received,
    Result, Staged, key_packages,
    state::{Reader, Writer},
};
use zeroize::Zeroizing;

const CIPHER_SUITE: CipherSuite = CipherSuite::CURVE25519_AES128;

/// The prior epochs kept per group, as mls-rs's in-memory storage keeps them.
const EPOCH_RETENTION: usize = 3;

/// The device's MLS state. mls-rs's in-memory storages have no save or load; this replaces both.
#[derive(Clone, Default)]
struct Store(Arc<Mutex<StoreData>>);

#[derive(Default)]
struct StoreData {
    groups: BTreeMap<Vec<u8>, GroupData>,
    key_packages: BTreeMap<Vec<u8>, KeyPackageData>,
}

#[derive(Default)]
struct GroupData {
    state: Vec<u8>,
    epochs: BTreeMap<u64, Vec<u8>>,
}

impl Store {
    fn lock(&self) -> MutexGuard<'_, StoreData> {
        self.0.lock().expect("store lock poisoned")
    }

    fn save(&self) -> Result<Vec<u8>> {
        let data = self.lock();
        let mut writer = Writer::default();
        writer.u64(data.groups.len() as u64);
        for (id, group) in &data.groups {
            writer.bytes(id).bytes(&group.state);
            writer.u64(group.epochs.len() as u64);
            for (epoch, record) in &group.epochs {
                writer.u64(*epoch).bytes(record);
            }
        }
        writer.u64(data.key_packages.len() as u64);
        for (id, key_package) in &data.key_packages {
            writer.bytes(id).bytes(&key_package.mls_encode_to_vec()?);
        }
        Ok(writer.finish())
    }

    fn load(state: &[u8]) -> Result<Self> {
        let mut reader = Reader::new(state);
        let mut data = StoreData::default();
        for _ in 0..reader.u64()? {
            let id = reader.bytes()?.to_vec();
            let mut group = GroupData {
                state: reader.bytes()?.to_vec(),
                epochs: BTreeMap::new(),
            };
            for _ in 0..reader.u64()? {
                let epoch = reader.u64()?;
                group.epochs.insert(epoch, reader.bytes()?.to_vec());
            }
            data.groups.insert(id, group);
        }
        for _ in 0..reader.u64()? {
            let id = reader.bytes()?.to_vec();
            let key_package = KeyPackageData::mls_decode(&mut reader.bytes()?)?;
            data.key_packages.insert(id, key_package);
        }
        reader.finish()?;
        Ok(Self(Arc::new(Mutex::new(data))))
    }
}

impl GroupStateStorage for Store {
    type Error = Infallible;

    fn state(&self, group_id: &[u8]) -> Result<Option<Zeroizing<Vec<u8>>>, Infallible> {
        Ok(self
            .lock()
            .groups
            .get(group_id)
            .map(|group| group.state.clone().into()))
    }

    fn epoch(
        &self,
        group_id: &[u8],
        epoch_id: u64,
    ) -> Result<Option<Zeroizing<Vec<u8>>>, Infallible> {
        Ok(self
            .lock()
            .groups
            .get(group_id)
            .and_then(|group| group.epochs.get(&epoch_id))
            .map(|record| record.clone().into()))
    }

    fn write(
        &mut self,
        state: GroupState,
        epoch_inserts: Vec<EpochRecord>,
        epoch_updates: Vec<EpochRecord>,
    ) -> Result<(), Infallible> {
        let mut data = self.lock();
        let group = data.groups.entry(state.id).or_default();
        group.state = state.data.to_vec();
        for record in epoch_inserts {
            group.epochs.insert(record.id, record.data.to_vec());
        }
        for record in epoch_updates {
            if let Some(existing) = group.epochs.get_mut(&record.id) {
                *existing = record.data.to_vec();
            }
        }
        while group.epochs.len() > EPOCH_RETENTION {
            group.epochs.pop_first();
        }
        Ok(())
    }

    fn max_epoch_id(&self, group_id: &[u8]) -> Result<Option<u64>, Infallible> {
        Ok(self
            .lock()
            .groups
            .get(group_id)
            .and_then(|group| group.epochs.keys().next_back().copied()))
    }
}

impl KeyPackageStorage for Store {
    type Error = Infallible;

    fn delete(&mut self, id: &[u8]) -> Result<(), Infallible> {
        self.lock().key_packages.remove(id);
        Ok(())
    }

    fn insert(&mut self, id: Vec<u8>, key_package: KeyPackageData) -> Result<(), Infallible> {
        self.lock().key_packages.insert(id, key_package);
        Ok(())
    }

    fn get(&self, id: &[u8]) -> Result<Option<KeyPackageData>, Infallible> {
        Ok(self.lock().key_packages.get(id).cloned())
    }
}

/// The `basic` identity provider that, given the admitted devices, also refuses a member that is
/// not one of them. mls-rs asks it of every leaf it validates: an Add's KeyPackage, a Commit's
/// path and every leaf of a tree it joins.
#[derive(Clone, Default)]
struct Admission(Option<Admitted>);

#[derive(Debug)]
struct AdmissionError(mls_scenario::Error);

impl IntoAnyError for AdmissionError {
    fn into_dyn_error(self) -> Result<Box<dyn std::error::Error + Send + Sync>, Self> {
        Ok(self.0)
    }
}

fn basic<T>(result: Result<T, BasicIdentityProviderError>) -> Result<T, AdmissionError> {
    result.map_err(|error| AdmissionError(error.into()))
}

impl IdentityProvider for Admission {
    type Error = AdmissionError;

    fn validate_member(
        &self,
        signing_identity: &SigningIdentity,
        timestamp: Option<MlsTime>,
        context: MemberValidationContext<'_>,
    ) -> Result<(), AdmissionError> {
        basic(BasicIdentityProvider.validate_member(signing_identity, timestamp, context))?;
        let Some(admitted) = &self.0 else {
            return Ok(());
        };
        MlsRsDevice::leaf(signing_identity)
            .and_then(|leaf| admitted.check(&leaf))
            .map_err(AdmissionError)
    }

    fn validate_external_sender(
        &self,
        signing_identity: &SigningIdentity,
        timestamp: Option<MlsTime>,
        extensions: Option<&ExtensionList>,
    ) -> Result<(), AdmissionError> {
        basic(BasicIdentityProvider.validate_external_sender(
            signing_identity,
            timestamp,
            extensions,
        ))
    }

    fn identity(
        &self,
        signing_identity: &SigningIdentity,
        extensions: &ExtensionList,
    ) -> Result<Vec<u8>, AdmissionError> {
        basic(BasicIdentityProvider.identity(signing_identity, extensions))
    }

    fn valid_successor(
        &self,
        predecessor: &SigningIdentity,
        successor: &SigningIdentity,
        extensions: &ExtensionList,
    ) -> Result<bool, AdmissionError> {
        basic(BasicIdentityProvider.valid_successor(predecessor, successor, extensions))
    }

    fn supported_types(&self) -> Vec<CredentialType> {
        BasicIdentityProvider.supported_types()
    }
}

pub struct MlsRsDevice {
    identity: Identity,
    store: Store,
}

impl MlsRsDevice {
    fn client(&self) -> Client<impl MlsConfig> {
        self.client_with(Admission(None))
    }

    /// A client with the device's `basic` credential and signing key, `admission` and KeyPackages
    /// of the longest lifetime with `last_resort` in their capabilities. mls-rs takes the Ed25519
    /// key as the 64 bytes of the secret and the public key. Each Commit comes with the new
    /// epoch's GroupInfo, with the `ratchet_tree` and `external_pub` extensions: mls-rs makes it
    /// only for a group that allows external Commits.
    fn client_with(&self, admission: Admission) -> Client<impl MlsConfig> {
        let credential = BasicCredential::new(self.identity.device_id().to_vec());
        let public_key = SignaturePublicKey::new(self.identity.public_key().to_vec());
        let secret_key = SignatureSecretKey::new(
            [self.identity.secret_key(), self.identity.public_key()].concat(),
        );
        Client::builder()
            .crypto_provider(RustCryptoProvider::default())
            .identity_provider(admission)
            .mls_rules(
                DefaultMlsRules::new()
                    .with_commit_options(CommitOptions::new().with_allow_external_commit(true)),
            )
            .extension_type(ExtensionType::LAST_RESORT_KEY_PACKAGE)
            .key_package_lifetime(Duration::from_secs(key_packages::MAX_LIFETIME))
            .group_state_storage(self.store.clone())
            .key_package_repo(self.store.clone())
            .signing_identity(
                SigningIdentity::new(credential.into_credential(), public_key),
                secret_key,
                CIPHER_SUITE,
            )
            .build()
    }

    /// A KeyPackage with `extensions` and the lifetime from now: mls-rs's lifetime starts at the
    /// time it is given.
    fn new_key_package(&mut self, extensions: ExtensionList) -> Result<Vec<u8>> {
        let lifetime = key_packages::lifetime(key_packages::now());
        let message = self.client().generate_key_package_message(
            extensions,
            ExtensionList::new(),
            Some(MlsTime::from(lifetime.not_before)),
        )?;
        Ok(message.to_bytes()?)
    }

    fn leaf(signing_identity: &SigningIdentity) -> Result<Leaf> {
        let credential = signing_identity
            .credential
            .as_basic()
            .ok_or("not a basic credential")?;
        Ok(Leaf {
            identity: credential.identifier().to_vec(),
            signature_key: signing_identity.signature_key.as_bytes().to_vec(),
        })
    }
}

fn commit(output: CommitOutput) -> Result<Commit> {
    let welcome = match output.welcome_messages.as_slice() {
        [] => None,
        [welcome] => Some(welcome.to_bytes()?),
        _ => return Err("more than one Welcome".into()),
    };
    let group_info = output
        .external_commit_group_info
        .ok_or("a Commit without a GroupInfo")?;
    Ok(Commit {
        commit: output.commit_message.to_bytes()?,
        welcome,
        group_info: group_info.to_bytes()?,
    })
}

impl Device for MlsRsDevice {
    const LIBRARY: &'static str = "mls-rs";
    type Observer = MlsRsObserver;

    fn new(identity: &Identity) -> Result<Self> {
        Ok(Self {
            identity: identity.clone(),
            store: Store::default(),
        })
    }

    fn load(identity: &Identity, state: &[u8]) -> Result<Self> {
        Ok(Self {
            identity: identity.clone(),
            store: Store::load(state)?,
        })
    }

    fn save(&self) -> Result<Vec<u8>> {
        self.store.save()
    }

    fn key_package(&mut self) -> Result<Vec<u8>> {
        self.new_key_package(ExtensionList::new())
    }

    fn last_resort_key_package(&mut self) -> Result<Vec<u8>> {
        self.new_key_package(ExtensionList::from(vec![
            LastResortKeyPackageExt.into_extension()?,
        ]))
    }

    fn create_group(&mut self, group_id: &[u8]) -> Result<()> {
        let mut group = self.client().create_group_with_id(
            group_id.to_vec(),
            Default::default(),
            Default::default(),
            None,
        )?;
        group.write_to_storage()?;
        Ok(())
    }

    fn add(
        &mut self,
        group_id: &[u8],
        key_packages: &[Vec<u8>],
        admitted: &Admitted,
    ) -> Result<Commit> {
        // The commit checks each KeyPackage: its signatures, lifetime and ciphersuite, and with the
        // identity provider its credential.
        let mut group = self
            .client_with(Admission(Some(admitted.clone())))
            .load_group(group_id)?;
        let mut builder = group.commit_builder();
        for key_package in key_packages {
            builder = builder.add_member(MlsMessage::from_bytes(key_package)?)?;
        }
        let output = builder.build()?;
        group.write_to_storage()?;
        commit(output)
    }

    fn remove(
        &mut self,
        group_id: &[u8],
        devices: &[Vec<u8>],
        authenticated_data: &[u8],
    ) -> Result<Commit> {
        let mut group = self.client().load_group(group_id)?;
        let members = group
            .roster()
            .members()
            .into_iter()
            .map(|member| Ok((Self::leaf(&member.signing_identity)?.identity, member.index)))
            .collect::<Result<Vec<_>>>()?;
        let mut builder = group.commit_builder();
        for device in devices {
            let (_, index) = members
                .iter()
                .find(|(identity, _)| identity == device)
                .ok_or("no member with the device ID")?;
            builder = builder.remove_member(*index)?;
        }
        let output = builder
            .authenticated_data(authenticated_data.to_vec())
            .build()?;
        group.write_to_storage()?;
        commit(output)
    }

    fn empty_commit(&mut self, group_id: &[u8], authenticated_data: &[u8]) -> Result<Commit> {
        let mut group = self.client().load_group(group_id)?;
        // A Commit without proposals always has an UpdatePath.
        let output = group.commit(authenticated_data.to_vec())?;
        group.write_to_storage()?;
        commit(output)
    }

    fn merge_pending_commit(&mut self, group_id: &[u8]) -> Result<()> {
        let mut group = self.client().load_group(group_id)?;
        group.apply_pending_commit()?;
        group.write_to_storage()?;
        Ok(())
    }

    fn clear_pending_commit(&mut self, group_id: &[u8]) -> Result<()> {
        let mut group = self.client().load_group(group_id)?;
        if !group.has_pending_commit() {
            return Err("no pending Commit".into());
        }
        group.clear_pending_commit();
        group.write_to_storage()?;
        Ok(())
    }

    fn delete_group(&mut self, group_id: &[u8]) -> Result<()> {
        // mls-rs's storage trait has no delete: the store drops the group itself.
        self.store
            .lock()
            .groups
            .remove(group_id)
            .map(drop)
            .ok_or_else(|| "no such group".into())
    }

    fn join(&mut self, welcome: &[u8]) -> Result<Vec<u8>> {
        let (mut group, _) =
            self.client()
                .join_group(None, &MlsMessage::from_bytes(welcome)?, None)?;
        group.write_to_storage()?;
        Ok(group.group_id().to_vec())
    }

    fn encrypt(&mut self, group_id: &[u8], data: &[u8]) -> Result<Vec<u8>> {
        let mut group = self.client().load_group(group_id)?;
        let message = group.encrypt_application_message(data, Vec::new())?;
        group.write_to_storage()?;
        Ok(message.to_bytes()?)
    }

    fn process_with(
        &mut self,
        group_id: &[u8],
        message: &[u8],
        check: impl FnOnce(&Staged) -> Result<()>,
    ) -> Result<Received> {
        let mut group = self.client().load_group(group_id)?;
        // The leaves before the message: a Commit's sender and removed members by their index.
        let leaves = group
            .roster()
            .members()
            .into_iter()
            .map(|member| Ok((member.index, Self::leaf(&member.signing_identity)?)))
            .collect::<Result<BTreeMap<_, _>>>()?;
        let leaf = |index| -> Result<Leaf> {
            leaves
                .get(&index)
                .cloned()
                .ok_or_else(|| "no member at the leaf".into())
        };
        // mls-rs merges a Commit as it processes it, so the check comes before the new epoch is
        // written, not before the merge.
        let received = match group.process_incoming_message(MlsMessage::from_bytes(message)?)? {
            ReceivedMessage::ApplicationMessage(message) => Received::Application {
                sender: leaf(message.sender_index)?,
                data: message.data().to_vec(),
            },
            ReceivedMessage::Commit(commit) => {
                let new_epoch = match &commit.effect {
                    CommitEffect::NewEpoch(new_epoch) => new_epoch,
                    CommitEffect::Removed { new_epoch, .. } => new_epoch,
                    CommitEffect::ReInit(_) => return Err("a ReInit Commit".into()),
                };
                let (mut removed, mut added) = (Vec::new(), Vec::new());
                for proposal in new_epoch.applied_proposals() {
                    match &proposal.proposal {
                        Proposal::Remove(remove) => removed.push(leaf(remove.to_remove())?),
                        Proposal::Add(add) => added.push(Self::leaf(add.signing_identity())?),
                        _ => {}
                    }
                }
                let sender = leaf(commit.committer)?;
                check(&Staged {
                    sender: sender.clone(),
                    removed,
                    added,
                    authenticated_data: commit.authenticated_data,
                })?;
                Received::Commit { sender }
            }
            _ => return Err("an unexpected message".into()),
        };
        group.write_to_storage()?;
        Ok(received)
    }

    fn epoch(&self, group_id: &[u8]) -> Result<u64> {
        Ok(self.client().load_group(group_id)?.current_epoch())
    }

    fn members(&self, group_id: &[u8]) -> Result<Vec<Leaf>> {
        let group = self.client().load_group(group_id)?;
        group
            .roster()
            .members()
            .iter()
            .map(|member| Self::leaf(&member.signing_identity))
            .collect()
    }

    fn history_key(&self, group_id: &[u8]) -> Result<HistoryKey> {
        let key = self.client().load_group(group_id)?.export_secret(
            HISTORY_LABEL.as_bytes(),
            &[],
            size_of::<HistoryKey>(),
        )?;
        Ok(key.as_bytes().try_into()?)
    }
}

/// The config of mls-rs's party outside a group with the `basic` credential.
type ExternalConfig = WithIdentityProvider<
    BasicIdentityProvider,
    WithCryptoProvider<RustCryptoProvider, ExternalBaseConfig>,
>;

/// A party outside the group: mls-rs's `ExternalGroup`.
pub struct MlsRsObserver {
    group: ExternalGroup<ExternalConfig>,
}

impl Observer for MlsRsObserver {
    fn observe(group_info: &[u8]) -> Result<Self> {
        let client = ExternalClient::builder()
            .crypto_provider(RustCryptoProvider::default())
            .identity_provider(BasicIdentityProvider)
            .build();
        let group = client.observe_group(MlsMessage::from_bytes(group_info)?, None, None)?;
        Ok(Self { group })
    }

    fn process_commit(
        &mut self,
        commit: &[u8],
        check: impl FnOnce(&Leaf) -> Result<()>,
    ) -> Result<()> {
        // mls-rs moves the group to the new epoch as it processes the Commit: a copy processes it.
        let mut next = self.group.clone();
        let ExternalReceivedMessage::Commit(commit) =
            next.process_incoming_message(MlsMessage::from_bytes(commit)?)?
        else {
            return Err("not a Commit".into());
        };
        let sender = self.group.roster().member_with_index(commit.committer)?;
        check(&MlsRsDevice::leaf(&sender.signing_identity)?)?;
        self.group = next;
        Ok(())
    }

    fn epoch(&self) -> u64 {
        self.group.group_context().epoch
    }

    fn context(&self) -> Result<Vec<u8>> {
        Ok(self.group.group_context().mls_encode_to_vec()?)
    }

    fn members(&self) -> Result<Vec<Leaf>> {
        self.group
            .roster()
            .members()
            .iter()
            .map(|member| MlsRsDevice::leaf(&member.signing_identity))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use mls_rs::identity::{Credential, CustomCredential};
    use mls_scenario::{
        Fleet,
        scenarios::{CHANNEL, create_channel},
    };

    use super::*;

    /// The store as the device's next step loads it.
    fn reloaded(store: &Store) -> Store {
        Store::load(&store.save().unwrap()).unwrap()
    }

    #[test]
    fn keeps_the_last_prior_epochs_across_reloads() {
        let mut store = Store::default();
        for id in 0..5u8 {
            let state = GroupState {
                id: b"g".to_vec(),
                data: vec![id].into(),
            };
            let record = EpochRecord::new(id.into(), vec![id].into());
            store.write(state, vec![record], Vec::new()).unwrap();
            store = reloaded(&store);
        }

        assert_eq!(store.state(b"g").unwrap().as_deref(), Some(&vec![4]));
        assert_eq!(store.max_epoch_id(b"g").unwrap(), Some(4));
        assert_eq!(store.epoch(b"g", 1).unwrap(), None);
        for id in 2..5u8 {
            assert_eq!(
                store.epoch(b"g", id.into()).unwrap().as_deref(),
                Some(&vec![id]),
                "prior epoch {id}"
            );
        }
        assert_eq!(store.max_epoch_id(b"other").unwrap(), None);
        assert_eq!(store.epoch(b"other", 4).unwrap(), None);
    }

    #[test]
    fn joining_deletes_the_used_key_package() {
        let mut fleet = Fleet::<MlsRsDevice>::new(2).unwrap();
        // A KeyPackage the channel does not use, then the one its Welcome uses.
        fleet.step(1, |d| d.key_package()).unwrap();
        create_channel(&mut fleet, 2).unwrap();
        let store = Store::load(fleet.state(1)).unwrap();
        assert_eq!(store.lock().key_packages.len(), 1);
    }

    /// The signing identity of `identity`'s device.
    fn signing_identity(identity: &Identity) -> SigningIdentity {
        MlsRsDevice::new(identity)
            .unwrap()
            .client()
            .signing_identity()
            .unwrap()
            .0
            .clone()
    }

    #[test]
    fn the_identity_provider_admits_only_admitted_devices() {
        let mut fleet = Fleet::<MlsRsDevice>::new(1).unwrap();
        let outsider = fleet.add_device_without_certificate().unwrap();
        let (member, outsider) = (
            signing_identity(fleet.identity(0)),
            signing_identity(fleet.identity(outsider)),
        );
        let custom = SigningIdentity::new(
            Credential::Custom(CustomCredential::new(CredentialType::new(0xff00), vec![1])),
            member.signature_key.clone(),
        );
        let validate = |admission: &Admission, identity: &SigningIdentity| {
            admission
                .validate_member(identity, None, MemberValidationContext::None)
                .is_ok()
        };

        let admission = Admission(Some(fleet.admitted()));
        assert!(validate(&admission, &member));
        assert!(!validate(&admission, &outsider));
        assert!(!validate(&admission, &custom));
        assert!(validate(&Admission(None), &outsider));
        assert!(!validate(&Admission(None), &custom));
    }

    #[test]
    fn the_identity_provider_is_the_basic_one_otherwise() {
        let fleet = Fleet::<MlsRsDevice>::new(2).unwrap();
        let (a, b) = (
            signing_identity(fleet.identity(0)),
            signing_identity(fleet.identity(1)),
        );
        let custom = SigningIdentity::new(
            Credential::Custom(CustomCredential::new(CredentialType::new(0xff00), vec![1])),
            a.signature_key.clone(),
        );
        let admission = Admission(Some(fleet.admitted()));
        let extensions = ExtensionList::new();

        assert_eq!(
            admission.identity(&a, &extensions).unwrap(),
            fleet.identity(0).device_id()
        );
        assert!(admission.identity(&custom, &extensions).is_err());
        assert!(admission.valid_successor(&a, &a, &extensions).unwrap());
        assert!(!admission.valid_successor(&a, &b, &extensions).unwrap());
        assert!(admission.valid_successor(&a, &custom, &extensions).is_err());
        assert!(admission.validate_external_sender(&a, None, None).is_ok());
        assert!(
            admission
                .validate_external_sender(&custom, None, None)
                .is_err()
        );
        assert_eq!(
            admission.supported_types(),
            [BasicCredential::credential_type()]
        );
    }

    #[test]
    fn a_commit_without_adds_has_no_welcome() {
        let mut fleet = Fleet::<MlsRsDevice>::new(1).unwrap();
        fleet.step(0, |d| d.create_group(CHANNEL)).unwrap();
        let admitted = fleet.admitted();
        let commit = fleet.step(0, |d| d.add(CHANNEL, &[], &admitted)).unwrap();
        assert!(commit.welcome.is_none());
    }
}
