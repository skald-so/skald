//! Skald's MLS prototype on OpenMLS with its RustCrypto provider.

use std::{collections::HashMap, sync::RwLock};

use mls_scenario::{
    Admitted, Commit, Device, HISTORY_LABEL, HistoryKey, Identity, Joining, Leaf, Observer,
    Received, Result, Staged, key_packages,
    state::{Reader, Writer},
    wire,
};
use openmls::{
    group::ProposalStore,
    messages::group_info::GroupInfo,
    prelude::{
        tls_codec::{Deserialize as _, Serialize as _},
        *,
    },
};
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
    node_leaf(key_package.leaf_node())
}

fn node_leaf(leaf_node: &LeafNode) -> Result<Leaf> {
    Ok(Leaf {
        identity: BasicCredential::try_from(leaf_node.credential().clone())?
            .identity()
            .to_vec(),
        signature_key: leaf_node.signature_key().as_slice().to_vec(),
    })
}

/// RFC 9420, §12.2: an external Commit removes at most one leaf, the joiner's old one, with the
/// joiner's `identity`. OpenMLS leaves this check to the application.
fn check_external_removes(joiner: &Leaf, removed: &[Leaf]) -> Result<()> {
    match removed {
        [] => Ok(()),
        [old] if old.identity == joiner.identity => Ok(()),
        [_] => Err("an external Commit that removes another device".into()),
        _ => Err("an external Commit with more than one Remove".into()),
    }
}

/// A Commit with its Welcome and the GroupInfo OpenMLS makes with it when the group's config has
/// `use_ratchet_tree_extension`: with the `ratchet_tree` and `external_pub` extensions.
fn commit(
    commit: MlsMessageOut,
    welcome: Option<MlsMessageOut>,
    group_info: Option<GroupInfo>,
) -> Result<Commit> {
    let group_info = group_info.ok_or("a Commit without a GroupInfo")?;
    Ok(Commit {
        commit: commit.to_bytes()?,
        welcome: welcome.map(|welcome| welcome.to_bytes()).transpose()?,
        group_info: MlsMessageOut::from(group_info).to_bytes()?,
    })
}

fn message(bytes: &[u8]) -> Result<MlsMessageBodyIn> {
    Ok(MlsMessageIn::tls_deserialize_exact(bytes)?.extract())
}

/// The config of a group the device joins, from a Welcome or by external Commit.
fn join_config() -> MlsGroupJoinConfig {
    MlsGroupJoinConfig::builder()
        .wire_format_policy(WIRE_FORMAT_POLICY)
        .use_ratchet_tree_extension(true)
        .build()
}

impl Device for OpenMlsDevice {
    const LIBRARY: &'static str = "OpenMLS";
    type Observer = OpenMlsObserver;

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
        // `add_members` would force an UpdatePath; a Commit of only Adds needs none.
        let (added, welcome, group_info) =
            group.add_members_without_update(&self.provider, &self.signer(), &key_packages)?;
        commit(added, Some(welcome), group_info)
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
        let (removal, welcome, group_info) =
            group.remove_members(&self.provider, &self.signer(), &leaves)?;
        commit(removal, welcome, group_info)
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
        let (empty, _, group_info) = bundle.into_contents();
        commit(empty, welcome, group_info)
    }

    fn merge_pending_commit(&mut self, group_id: &[u8]) -> Result<()> {
        let mut group = self.group(group_id)?;
        if group.pending_commit().is_none() {
            return Err("no pending Commit".into());
        }
        group.merge_pending_commit(&self.provider)?;
        Ok(())
    }

    fn clear_pending_commit(&mut self, group_id: &[u8]) -> Result<()> {
        let mut group = self.group(group_id)?;
        if group.pending_commit().is_none() {
            return Err("no pending Commit".into());
        }
        group.clear_pending_commit(self.provider.storage())?;
        Ok(())
    }

    fn delete_group(&mut self, group_id: &[u8]) -> Result<()> {
        self.group(group_id)?.delete(self.provider.storage())?;
        Ok(())
    }

    fn join_with(
        &mut self,
        welcome: &[u8],
        check: impl FnOnce(&Joining) -> Result<()>,
    ) -> Result<Vec<u8>> {
        let MlsMessageBodyIn::Welcome(welcome) = message(welcome)? else {
            return Err("not a Welcome".into());
        };
        // A staged Welcome gives its signer's leaf and the members before the device joins.
        let staged =
            StagedWelcome::new_from_welcome(&self.provider, &join_config(), welcome, None)?;
        check(&Joining {
            signer: node_leaf(staged.welcome_sender()?)?,
            members: staged.members().map(leaf).collect::<Result<_>>()?,
        })?;
        let group = staged.into_group(&self.provider)?;
        Ok(group.group_id().as_slice().to_vec())
    }

    fn join_external(
        &mut self,
        group_info: &[u8],
        check: impl FnOnce(&Joining) -> Result<()>,
    ) -> Result<Commit> {
        // `PublicGroup` verifies the GroupInfo and gives its members; OpenMLS exposes no
        // GroupInfo's signer, so its leaf index comes from the encoding.
        let observed = OpenMlsObserver::observe(group_info)?;
        let signer = LeafNodeIndex::new(wire::group_info(group_info)?.signer);
        check(&Joining {
            signer: node_leaf(observed.group.leaf(signer).ok_or("no member at the leaf")?)?,
            members: observed.members()?,
        })?;
        let MlsMessageBodyIn::GroupInfo(group_info) = message(group_info)? else {
            return Err("not a GroupInfo".into());
        };
        // The builder itself removes a leaf with the device's signature key. The new group
        // replaces the device's old state of it in the storage.
        let (mut group, bundle) = MlsGroup::external_commit_builder()
            .with_config(join_config())
            .build_group(&self.provider, group_info, self.credential())?
            .load_psks(self.provider.storage())?
            .build(
                self.provider.rand(),
                self.provider.crypto(),
                &self.signer(),
                |_| true,
            )?
            .finalize(&self.provider)?;
        // OpenMLS cannot drop a pending external Commit; the device merges it at once.
        group.merge_pending_commit(&self.provider)?;
        let (external, _, group_info) = bundle.into_contents();
        commit(external, None, group_info)
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
        let member = match *processed.sender() {
            Sender::Member(index) => Some(Self::leaf(&group, index)?),
            Sender::NewMemberCommit => None,
            _ => return Err("a message from a non-member".into()),
        };
        let authenticated_data = processed.aad().to_vec();
        match processed.into_content() {
            ProcessedMessageContent::ApplicationMessage(message) => Ok(Received::Application {
                sender: member.ok_or("an application message from a new member")?,
                data: message.into_bytes(),
            }),
            ProcessedMessageContent::StagedCommitMessage(commit) => {
                // The Commit is staged: the group stays in its epoch until the merge.
                let removed = commit
                    .remove_proposals()
                    .map(|remove| Self::leaf(&group, remove.remove_proposal().removed()))
                    .collect::<Result<Vec<_>>>()?;
                let mut added = commit
                    .add_proposals()
                    .map(|add| key_package_leaf(add.add_proposal().key_package()))
                    .collect::<Result<Vec<_>>>()?;
                let sender = match member {
                    Some(sender) => sender,
                    // An external Commit: the joiner's new leaf is its UpdatePath's.
                    None => {
                        let joiner = node_leaf(
                            commit
                                .update_path_leaf_node()
                                .ok_or("an external Commit without a path")?,
                        )?;
                        check_external_removes(&joiner, &removed)?;
                        added.push(joiner.clone());
                        joiner
                    }
                };
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

/// A party outside the group: OpenMLS's `PublicGroup`, with a storage of its own.
pub struct OpenMlsObserver {
    crypto: RustCrypto,
    storage: MemoryStorage,
    group: PublicGroup,
}

impl Observer for OpenMlsObserver {
    fn observe(group_info: &[u8]) -> Result<Self> {
        let MlsMessageBodyIn::GroupInfo(group_info) = message(group_info)? else {
            return Err("not a GroupInfo".into());
        };
        let tree = group_info
            .extensions()
            .ratchet_tree()
            .ok_or("a GroupInfo without ratchet_tree")?
            .ratchet_tree()
            .clone();
        let (crypto, storage) = (RustCrypto::default(), MemoryStorage::default());
        let (group, _) =
            PublicGroup::from_external(&crypto, &storage, tree, group_info, ProposalStore::new())?;
        Ok(Self {
            crypto,
            storage,
            group,
        })
    }

    fn process_commit(
        &mut self,
        commit: &[u8],
        check: impl FnOnce(&Leaf) -> Result<()>,
    ) -> Result<()> {
        let message = MlsMessageIn::tls_deserialize_exact(commit)?.try_into_protocol_message()?;
        let processed = self.group.process_message(&self.crypto, message)?;
        let sender = processed.sender().clone();
        let ProcessedMessageContent::StagedCommitMessage(staged) = processed.into_content() else {
            return Err("not a Commit".into());
        };
        let leaf = match sender {
            Sender::Member(index) => {
                node_leaf(self.group.leaf(index).ok_or("no member at the leaf")?)?
            }
            // An external Commit: the joiner's new leaf is its UpdatePath's.
            Sender::NewMemberCommit => {
                let joiner = node_leaf(
                    staged
                        .update_path_leaf_node()
                        .ok_or("an external Commit without a path")?,
                )?;
                let removed = staged
                    .remove_proposals()
                    .map(|remove| {
                        node_leaf(
                            self.group
                                .leaf(remove.remove_proposal().removed())
                                .ok_or("no member at the leaf")?,
                        )
                    })
                    .collect::<Result<Vec<_>>>()?;
                check_external_removes(&joiner, &removed)?;
                joiner
            }
            _ => return Err("a Commit from a non-member".into()),
        };
        check(&leaf)?;
        self.group.merge_commit(&self.storage, *staged)?;
        Ok(())
    }

    fn epoch(&self) -> u64 {
        self.group.group_context().epoch().as_u64()
    }

    fn context(&self) -> Result<Vec<u8>> {
        Ok(self.group.group_context().tls_serialize_detached()?)
    }

    fn members(&self) -> Result<Vec<Leaf>> {
        self.group.members().map(leaf).collect()
    }
}

#[cfg(test)]
mod tests {
    use mls_scenario::{
        Refusal,
        scenarios::{CHANNEL, Network, assert_group, leaf},
    };

    use super::*;

    #[test]
    fn external_commit_removes_only_the_joiners_old_leaf() {
        let joiner = leaf(&Identity::generate());
        let other = leaf(&Identity::generate());
        assert!(check_external_removes(&joiner, &[]).is_ok());
        assert!(check_external_removes(&joiner, std::slice::from_ref(&joiner)).is_ok());
        let refusal = |removed: &[Leaf]| {
            check_external_removes(&joiner, removed)
                .unwrap_err()
                .to_string()
        };
        assert_eq!(
            refusal(std::slice::from_ref(&other)),
            "an external Commit that removes another device"
        );
        for removed in [[joiner.clone(), joiner.clone()], [joiner.clone(), other]] {
            assert_eq!(
                refusal(&removed),
                "an external Commit with more than one Remove"
            );
        }
    }

    /// OpenMLS itself accepts an external Commit that removes another device; the Delivery
    /// Service and the members refuse it. OpenMLS's builder removes only a leaf with the joiner's
    /// signature key, so this joiner has device 1's signing key and another device ID.
    #[test]
    fn external_commit_that_removes_another_device_is_refused() {
        let mut network = Network::<OpenMlsDevice>::channel(3);
        let group_info = network.service.group_info(CHANNEL).unwrap().to_vec();
        let MlsMessageBodyIn::GroupInfo(group_info) = message(&group_info).unwrap() else {
            panic!("not a GroupInfo");
        };
        let victim = network.fleet.identity(1).clone();
        let joiner = Identity::generate().device_id();
        let provider = Provider::default();
        let credential = CredentialWithKey {
            credential: BasicCredential::new(joiner.to_vec()).into(),
            signature_key: victim.public_key().to_vec().into(),
        };
        let (_, bundle) = MlsGroup::external_commit_builder()
            .with_config(join_config())
            .build_group(&provider, group_info, credential)
            .unwrap()
            .load_psks(provider.storage())
            .unwrap()
            .build(
                provider.rand(),
                provider.crypto(),
                &DeviceSigner(&victim),
                |_| true,
            )
            .unwrap()
            .finalize(&provider)
            .unwrap();
        let commit = bundle.into_commit().to_bytes().unwrap();
        assert_eq!(
            wire::public_commit(&commit).unwrap().removed,
            [1],
            "the external Commit removes device 1"
        );

        assert_eq!(
            network.service.send(&joiner, &commit),
            Err(Refusal::NotAMember),
            "the service"
        );
        assert_eq!(
            network.service.epoch(CHANNEL),
            Some(1),
            "the service's epoch"
        );
        for member in [0, 2] {
            assert!(
                network
                    .fleet
                    .step(member, |d| d.process(CHANNEL, &commit))
                    .is_err(),
                "device {member} processed the external Commit"
            );
        }
        assert_group(&mut network.fleet, 0..3, 1, 0..3);
    }
}
