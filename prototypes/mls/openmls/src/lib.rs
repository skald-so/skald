//! Skald's MLS prototype on OpenMLS with its RustCrypto provider.

use std::{collections::HashMap, sync::RwLock};

use mls_scenario::{
    Commit, Device, Identity, Leaf, Received, Result,
    state::{Reader, Writer},
};
use openmls::prelude::{tls_codec::Deserialize as _, *};
use openmls_memory_storage::MemoryStorage;
use openmls_rust_crypto::RustCrypto;
use openmls_traits::{
    OpenMlsProvider,
    signatures::{Signer, SignerError},
    types::SignatureScheme,
};

const CIPHERSUITE: Ciphersuite = Ciphersuite::MLS_128_DHKEMX25519_AES128GCM_SHA256_Ed25519;

/// Handshake messages are PublicMessages; OpenMLS always sends application messages as
/// PrivateMessages.
const WIRE_FORMAT_POLICY: WireFormatPolicy = PURE_PLAINTEXT_WIRE_FORMAT_POLICY;

/// The RustCrypto provider with a storage the device saves and loads: `OpenMlsRustCrypto` cannot
/// take a loaded storage.
#[derive(Default)]
struct Provider {
    crypto: RustCrypto,
    storage: MemoryStorage,
}

impl OpenMlsProvider for Provider {
    type CryptoProvider = RustCrypto;
    type RandProvider = RustCrypto;
    type StorageProvider = MemoryStorage;

    fn storage(&self) -> &MemoryStorage {
        &self.storage
    }

    fn crypto(&self) -> &RustCrypto {
        &self.crypto
    }

    fn rand(&self) -> &RustCrypto {
        &self.crypto
    }
}

/// Signs with the device's key, which stays outside OpenMLS's storage.
struct DeviceSigner<'a>(&'a Identity);

impl Signer for DeviceSigner<'_> {
    fn sign(&self, payload: &[u8]) -> Result<Vec<u8>, SignerError> {
        Ok(self.0.sign(payload).to_vec())
    }

    fn signature_scheme(&self) -> SignatureScheme {
        SignatureScheme::ED25519
    }
}

pub struct OpenMlsDevice {
    identity: Identity,
    provider: Provider,
}

impl OpenMlsDevice {
    fn signer(&self) -> DeviceSigner<'_> {
        DeviceSigner(&self.identity)
    }

    /// The `basic` credential with the device ID and the device's signing key.
    fn credential(&self) -> CredentialWithKey {
        CredentialWithKey {
            credential: BasicCredential::new(self.identity.device_id().to_vec()).into(),
            signature_key: self.identity.public_key().to_vec().into(),
        }
    }

    fn group(&self, group_id: &[u8]) -> Result<MlsGroup> {
        MlsGroup::load(self.provider.storage(), &GroupId::from_slice(group_id))?
            .ok_or_else(|| "no such group".into())
    }

    fn leaf(group: &MlsGroup, index: LeafNodeIndex) -> Result<Leaf> {
        let member = group.member_at(index).ok_or("no member at the leaf")?;
        leaf(member)
    }
}

fn leaf(member: Member) -> Result<Leaf> {
    Ok(Leaf {
        identity: BasicCredential::try_from(member.credential)?
            .identity()
            .to_vec(),
        signature_key: member.signature_key,
    })
}

fn message(bytes: &[u8]) -> Result<MlsMessageBodyIn> {
    Ok(MlsMessageIn::tls_deserialize_exact(bytes)?.extract())
}

impl Device for OpenMlsDevice {
    const LIBRARY: &'static str = "OpenMLS";

    fn new(identity: &Identity) -> Result<Self> {
        Ok(Self {
            identity: identity.clone(),
            provider: Provider::default(),
        })
    }

    fn load(identity: &Identity, state: &[u8]) -> Result<Self> {
        let mut reader = Reader::new(state);
        let mut values = HashMap::new();
        for _ in 0..reader.u64()? {
            let key = reader.bytes()?.to_vec();
            values.insert(key, reader.bytes()?.to_vec());
        }
        reader.finish()?;
        Ok(Self {
            identity: identity.clone(),
            provider: Provider {
                crypto: RustCrypto::default(),
                storage: MemoryStorage {
                    values: RwLock::new(values),
                },
            },
        })
    }

    fn save(&self) -> Result<Vec<u8>> {
        let values = self
            .provider
            .storage
            .values
            .read()
            .map_err(|_| "storage lock poisoned")?;
        let mut entries: Vec<_> = values.iter().collect();
        entries.sort();
        let mut writer = Writer::default();
        writer.u64(entries.len() as u64);
        for (key, value) in entries {
            writer.bytes(key).bytes(value);
        }
        Ok(writer.finish())
    }

    fn key_package(&mut self) -> Result<Vec<u8>> {
        let bundle = KeyPackage::builder().build(
            CIPHERSUITE,
            &self.provider,
            &self.signer(),
            self.credential(),
        )?;
        Ok(MlsMessageOut::from(bundle.key_package().clone()).to_bytes()?)
    }

    fn create_group(&mut self, group_id: &[u8]) -> Result<()> {
        let config = MlsGroupCreateConfig::builder()
            .ciphersuite(CIPHERSUITE)
            .wire_format_policy(WIRE_FORMAT_POLICY)
            .use_ratchet_tree_extension(true)
            .build();
        MlsGroup::new_with_group_id(
            &self.provider,
            &self.signer(),
            &config,
            GroupId::from_slice(group_id),
            self.credential(),
        )?;
        Ok(())
    }

    fn add(&mut self, group_id: &[u8], key_packages: &[Vec<u8>]) -> Result<Commit> {
        let key_packages = key_packages
            .iter()
            .map(|bytes| match message(bytes)? {
                MlsMessageBodyIn::KeyPackage(key_package) => {
                    Ok(key_package.validate(self.provider.crypto(), ProtocolVersion::Mls10)?)
                }
                _ => Err("not a KeyPackage".into()),
            })
            .collect::<Result<Vec<_>>>()?;
        let mut group = self.group(group_id)?;
        let (commit, welcome, _) =
            group.add_members(&self.provider, &self.signer(), &key_packages)?;
        Ok(Commit {
            commit: commit.to_bytes()?,
            welcome: Some(welcome.to_bytes()?),
        })
    }

    fn merge_pending_commit(&mut self, group_id: &[u8]) -> Result<()> {
        let mut group = self.group(group_id)?;
        if group.pending_commit().is_none() {
            return Err("no pending Commit".into());
        }
        group.merge_pending_commit(&self.provider)?;
        Ok(())
    }

    fn join(&mut self, welcome: &[u8]) -> Result<Vec<u8>> {
        let MlsMessageBodyIn::Welcome(welcome) = message(welcome)? else {
            return Err("not a Welcome".into());
        };
        let config = MlsGroupJoinConfig::builder()
            .wire_format_policy(WIRE_FORMAT_POLICY)
            .use_ratchet_tree_extension(true)
            .build();
        let group = StagedWelcome::new_from_welcome(&self.provider, &config, welcome, None)?
            .into_group(&self.provider)?;
        Ok(group.group_id().as_slice().to_vec())
    }

    fn encrypt(&mut self, group_id: &[u8], data: &[u8]) -> Result<Vec<u8>> {
        let mut group = self.group(group_id)?;
        let message = group.create_message(&self.provider, &self.signer(), data)?;
        Ok(message.to_bytes()?)
    }

    fn process(&mut self, group_id: &[u8], bytes: &[u8]) -> Result<Received> {
        let message = MlsMessageIn::tls_deserialize_exact(bytes)?.try_into_protocol_message()?;
        let mut group = self.group(group_id)?;
        let processed = group.process_message(&self.provider, message)?;
        let Sender::Member(index) = *processed.sender() else {
            return Err("a message from a non-member".into());
        };
        let sender = Self::leaf(&group, index)?;
        match processed.into_content() {
            ProcessedMessageContent::ApplicationMessage(message) => Ok(Received::Application {
                sender,
                data: message.into_bytes(),
            }),
            ProcessedMessageContent::StagedCommitMessage(commit) => {
                group.merge_staged_commit(&self.provider, *commit)?;
                Ok(Received::Commit { sender })
            }
            _ => Err("an unexpected message".into()),
        }
    }

    fn epoch(&self, group_id: &[u8]) -> Result<u64> {
        Ok(self.group(group_id)?.epoch().as_u64())
    }

    fn members(&self, group_id: &[u8]) -> Result<Vec<Leaf>> {
        self.group(group_id)?.members().map(leaf).collect()
    }
}
