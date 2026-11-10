//! Skald's MLS prototype on mls-rs with its RustCrypto provider.

use std::{
    collections::BTreeMap,
    convert::Infallible,
    sync::{Arc, Mutex, MutexGuard},
};

use mls_rs::{
    CipherSuite, Client, GroupStateStorage, KeyPackageStorage, MlsMessage,
    client_builder::MlsConfig,
    crypto::{SignaturePublicKey, SignatureSecretKey},
    group::ReceivedMessage,
    identity::{
        SigningIdentity,
        basic::{BasicCredential, BasicIdentityProvider},
    },
    mls_rs_codec::{MlsDecode, MlsEncode},
    storage_provider::KeyPackageData,
};
use mls_rs_core::group::{EpochRecord, GroupState};
use mls_rs_crypto_rustcrypto::RustCryptoProvider;
use mls_scenario::{
    Commit, Device, Identity, Leaf, Received, Result,
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

pub struct MlsRsDevice {
    identity: Identity,
    store: Store,
}

impl MlsRsDevice {
    /// A client with the device's `basic` credential and signing key. mls-rs takes the Ed25519 key
    /// as the 64 bytes of the secret and the public key.
    fn client(&self) -> Client<impl MlsConfig> {
        let credential = BasicCredential::new(self.identity.device_id().to_vec());
        let public_key = SignaturePublicKey::new(self.identity.public_key().to_vec());
        let secret_key = SignatureSecretKey::new(
            [self.identity.secret_key(), self.identity.public_key()].concat(),
        );
        Client::builder()
            .crypto_provider(RustCryptoProvider::default())
            .identity_provider(BasicIdentityProvider)
            .group_state_storage(self.store.clone())
            .key_package_repo(self.store.clone())
            .signing_identity(
                SigningIdentity::new(credential.into_credential(), public_key),
                secret_key,
                CIPHER_SUITE,
            )
            .build()
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

impl Device for MlsRsDevice {
    const LIBRARY: &'static str = "mls-rs";

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
        let message = self.client().generate_key_package_message(
            Default::default(),
            Default::default(),
            None,
        )?;
        Ok(message.to_bytes()?)
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

    fn add(&mut self, group_id: &[u8], key_packages: &[Vec<u8>]) -> Result<Commit> {
        let mut group = self.client().load_group(group_id)?;
        let mut builder = group.commit_builder();
        for key_package in key_packages {
            builder = builder.add_member(MlsMessage::from_bytes(key_package)?)?;
        }
        let output = builder.build()?;
        group.write_to_storage()?;
        let welcome = match output.welcome_messages.as_slice() {
            [] => None,
            [welcome] => Some(welcome.to_bytes()?),
            _ => return Err("more than one Welcome".into()),
        };
        Ok(Commit {
            commit: output.commit_message.to_bytes()?,
            welcome,
        })
    }

    fn merge_pending_commit(&mut self, group_id: &[u8]) -> Result<()> {
        let mut group = self.client().load_group(group_id)?;
        group.apply_pending_commit()?;
        group.write_to_storage()?;
        Ok(())
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

    fn process(&mut self, group_id: &[u8], message: &[u8]) -> Result<Received> {
        let mut group = self.client().load_group(group_id)?;
        let received = group.process_incoming_message(MlsMessage::from_bytes(message)?)?;
        let sender_index = match &received {
            ReceivedMessage::ApplicationMessage(message) => message.sender_index,
            ReceivedMessage::Commit(commit) => commit.committer,
            _ => return Err("an unexpected message".into()),
        };
        let member = group
            .member_at_index(sender_index)
            .ok_or("no member at the sender's leaf")?;
        let sender = Self::leaf(&member.signing_identity)?;
        group.write_to_storage()?;
        Ok(match received {
            ReceivedMessage::ApplicationMessage(message) => Received::Application {
                sender,
                data: message.data().to_vec(),
            },
            _ => Received::Commit { sender },
        })
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
}
