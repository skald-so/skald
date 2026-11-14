//! A channel seen from outside its group, as a linked browser or the server sees it: from its
//! GroupInfo (`docs/protocol/README.md`, "Envelope"; `docs/protocol/membership.md`, "Wraps"), then
//! from its Commits, on mls-rs's `ExternalGroup`, without group secrets.

use mls_rs::{
    WireFormat,
    external_client::{
        ExternalClient, ExternalGroup, ExternalReceivedMessage,
        builder::{ExternalBaseConfig, WithCryptoProvider, WithIdentityProvider},
    },
    group::CommitEffect,
};
use mls_rs_crypto_rustcrypto::RustCryptoProvider;
use skald_proto::ChannelId;

use super::{
    MemberDevice, MlsError, key_package, member_device, members_of, message,
    profile::{Profile, SUITE},
};

/// The config of mls-rs's party outside a group, with the profile's identity provider and
/// ciphersuite; spelled out, as mls-rs keeps the trait of `impl` configs private.
type Config =
    WithIdentityProvider<Profile, WithCryptoProvider<RustCryptoProvider, ExternalBaseConfig>>;

/// A channel's epoch and members, as a party outside its group sees them.
#[derive(Clone)]
pub struct ChannelView {
    group: ExternalGroup<Config>,
    /// The GroupInfo's signer, or the followed Commit's sender.
    signer: MemberDevice,
    members: Vec<MemberDevice>,
}

impl ChannelView {
    /// The view of `channel` from `group_info`, a GroupInfo in an `MLSMessage` with the
    /// `ratchet_tree` extension: refused unless its `group_id` is `channel`, its tree checks, every
    /// leaf keeps to the profile and spans at most a KeyPackage's lifetime, and its signature
    /// verifies with its signer leaf's key, a member's.
    ///
    /// # Errors
    /// [`MlsError::Malformed`] if `group_info` is not a GroupInfo; [`MlsError::OtherChannel`] if it
    /// is not of `channel`; [`MlsError::Lifetime`] if a leaf's lifetime spans more than 60 days;
    /// [`MlsError::Mls`] if mls-rs refuses it, such as a tree or a signature that does not check
    /// or a leaf that breaks the profile.
    pub fn observe(channel: ChannelId, group_info: &[u8]) -> Result<Self, MlsError> {
        let group_info = message(group_info, WireFormat::GroupInfo)?;
        let info = group_info.as_group_info().ok_or(MlsError::Malformed)?;
        if info.group_context().group_id != channel.as_bytes() {
            return Err(MlsError::OtherChannel);
        }
        let signer = info.sender();
        let group = ExternalClient::builder()
            .crypto_provider(RustCryptoProvider::with_enabled_cipher_suites(vec![SUITE]))
            .identity_provider(Profile)
            .build()
            .observe_group(group_info, None, None)?;
        Self::of(group, signer)
    }

    /// The view of the next epoch from `commit`, a Commit of the view's epoch in a PublicMessage;
    /// the view stays as it was, for the caller to keep or drop the new one. Refused as a member
    /// refuses it, such as a Commit of another epoch or group or one whose signature does not
    /// verify with its sender leaf's key; unchecked are its membership tag, which takes the
    /// group's secrets, and whether an Add's KeyPackage lifetime holds, which takes a time.
    ///
    /// # Errors
    /// [`MlsError::Malformed`] if `commit` is not a Commit in a PublicMessage;
    /// [`MlsError::Lifetime`] if a leaf's lifetime spans more than 60 days after it;
    /// [`MlsError::Mls`] if mls-rs refuses it.
    pub fn follow(&self, commit: &[u8]) -> Result<Self, MlsError> {
        let commit = message(commit, WireFormat::PublicMessage)?;
        // mls-rs moves the group to the new epoch as it processes the Commit: a copy processes it.
        let mut group = self.group.clone();
        let ExternalReceivedMessage::Commit(description) =
            group.process_incoming_message(commit)?
        else {
            return Err(MlsError::Malformed);
        };
        if let CommitEffect::ReInit(_) = description.effect {
            return Err(MlsError::Malformed);
        }
        // An external Commit's sender is the joiner's new leaf, which only the new epoch has; a
        // member's keeps its leaf.
        Self::of(group, description.committer)
    }

    fn of(group: ExternalGroup<Config>, signer: u32) -> Result<Self, MlsError> {
        key_package::check_spans(&group.exported_tree())?;
        let signer = member_device(&group.roster().member_with_index(signer)?.signing_identity)?;
        let members = members_of(&group.roster())?;
        Ok(Self {
            group,
            signer,
            members,
        })
    }

    /// The channel's epoch.
    pub fn epoch(&self) -> u64 {
        self.group.group_context().epoch
    }

    /// The member that signed the GroupInfo, or sent the Commit the view followed.
    pub fn signer(&self) -> MemberDevice {
        self.signer
    }

    /// Every member device, in leaf order.
    pub fn members(&self) -> &[MemberDevice] {
        &self.members
    }
}

impl std::fmt::Debug for ChannelView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChannelView")
            .field("epoch", &self.epoch())
            .field("signer", &self.signer)
            .field("members", &self.members)
            .finish()
    }
}
