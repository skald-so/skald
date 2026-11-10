//! Skald's MLS prototype on OpenMLS with its RustCrypto provider.

use std::{collections::HashMap, sync::RwLock};

use mls_scenario::{
    Admitted, Commit, Device, HISTORY_LABEL, HistoryKey, Identity, Leaf, Received, Result, Staged,
    key_packages,
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

    /// A KeyPackage with the lifetime from now and `last_resort` in its leaf's capabilities:
    /// `KeyPackageIn::validate` refuses a KeyPackage with an extension its capabilities lack.
    fn new_key_package(&mut self, last_resort: bool) -> Result<Vec<u8>> {
        let lifetime = key_packages::lifetime(key_packages::now());
        let capabilities =
            Capabilities::new(None, None, Some(&[ExtensionType::LastResort]), None, None);
        let mut builder = KeyPackage::builder()
            .key_package_lifetime(Lifetime::init(lifetime.not_before, lifetime.not_after))
            .leaf_node_capabilities(capabilities);
        if last_resort {
            builder = builder.mark_as_last_resort();
        }
        let bundle = builder.build(
            CIPHERSUITE,
            &self.provider,
            &self.signer(),
            self.credential(),
        )?;
        Ok(MlsMessageOut::from(bundle.key_package().clone()).to_bytes()?)
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

/// The leaf a KeyPackage gives its device.
fn key_package_leaf(key_package: &KeyPackage) -> Result<Leaf> {
    let leaf_node = key_package.leaf_node();
    Ok(Leaf {
        identity: BasicCredential::try_from(leaf_node.credential().clone())?
            .identity()
            .to_vec(),
        signature_key: leaf_node.signature_key().as_slice().to_vec(),
    })
}

fn commit(commit: MlsMessageOut, welcome: Option<MlsMessageOut>) -> Result<Commit> {
    Ok(Commit {
        commit: commit.to_bytes()?,
        welcome: welcome.map(|welcome| welcome.to_bytes()).transpose()?,
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
        self.new_key_package(false)
    }

    fn last_resort_key_package(&mut self) -> Result<Vec<u8>> {
        self.new_key_package(true)
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

    fn add(
        &mut self,
        group_id: &[u8],
        key_packages: &[Vec<u8>],
        admitted: &Admitted,
    ) -> Result<Commit> {
        // `validate` checks the signatures and the lifetime, and the Commit the ciphersuite; the
        // credential is the device's to check.
        let key_packages = key_packages
            .iter()
            .map(|bytes| {
                let MlsMessageBodyIn::KeyPackage(key_package) = message(bytes)? else {
                    return Err("not a KeyPackage".into());
                };
                let key_package =
                    key_package.validate(self.provider.crypto(), ProtocolVersion::Mls10)?;
                admitted.check(&key_package_leaf(&key_package)?)?;
                Ok(key_package)
            })
            .collect::<Result<Vec<_>>>()?;
        let mut group = self.group(group_id)?;
        let (added, welcome, _) =
            group.add_members(&self.provider, &self.signer(), &key_packages)?;
        commit(added, Some(welcome))
    }

    fn remove(
        &mut self,
        group_id: &[u8],
        devices: &[Vec<u8>],
        authenticated_data: &[u8],
    ) -> Result<Commit> {
        let mut group = self.group(group_id)?;
        let members = group
            .members()
            .map(|member| Ok((leaf(member.clone())?.identity, member.index)))
            .collect::<Result<Vec<_>>>()?;
        let leaves = devices
            .iter()
            .map(|device| {
                members
                    .iter()
                    .find(|(identity, _)| identity == device)
                    .map(|(_, index)| *index)
                    .ok_or_else(|| "no member with the device ID".into())
            })
            .collect::<Result<Vec<_>>>()?;
        // The AAD of the next message only: OpenMLS resets it after the Commit.
        group.set_aad(authenticated_data.to_vec());
        let (removal, welcome, _) =
            group.remove_members(&self.provider, &self.signer(), &leaves)?;
        commit(removal, welcome)
    }

    fn empty_commit(&mut self, group_id: &[u8], authenticated_data: &[u8]) -> Result<Commit> {
        let mut group = self.group(group_id)?;
        group.set_aad(authenticated_data.to_vec());
        // With no proposals, OpenMLS's self-update is an empty Commit with an UpdatePath.
        let bundle = group.self_update(
            &self.provider,
            &self.signer(),
            LeafNodeParameters::default(),
        )?;
        let welcome = bundle.to_welcome_msg();
        commit(bundle.into_commit(), welcome)
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

    fn process_with(
        &mut self,
        group_id: &[u8],
        bytes: &[u8],
        check: impl FnOnce(&Staged) -> Result<()>,
    ) -> Result<Received> {
        let message = MlsMessageIn::tls_deserialize_exact(bytes)?.try_into_protocol_message()?;
        let mut group = self.group(group_id)?;
        let processed = group.process_message(&self.provider, message)?;
        let Sender::Member(index) = *processed.sender() else {
            return Err("a message from a non-member".into());
        };
        let sender = Self::leaf(&group, index)?;
        let authenticated_data = processed.aad().to_vec();
        match processed.into_content() {
            ProcessedMessageContent::ApplicationMessage(message) => Ok(Received::Application {
                sender,
                data: message.into_bytes(),
            }),
            ProcessedMessageContent::StagedCommitMessage(commit) => {
                // The Commit is staged: the group stays in its epoch until the merge.
                let removed = commit
                    .remove_proposals()
                    .map(|remove| Self::leaf(&group, remove.remove_proposal().removed()))
                    .collect::<Result<_>>()?;
                let added = commit
                    .add_proposals()
                    .map(|add| key_package_leaf(add.add_proposal().key_package()))
                    .collect::<Result<_>>()?;
                check(&Staged {
                    sender: sender.clone(),
                    removed,
                    added,
                    authenticated_data,
                })?;
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

    fn history_key(&self, group_id: &[u8]) -> Result<HistoryKey> {
        let key = self.group(group_id)?.export_secret(
            self.provider.crypto(),
            HISTORY_LABEL,
            &[],
            size_of::<HistoryKey>(),
        )?;
        Ok(key.as_slice().try_into()?)
    }
}
