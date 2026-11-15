//! A channel seen from outside its group, as a linked browser or the server sees it: from its
//! GroupInfo (`docs/protocol/README.md`, "Envelope"; `docs/protocol/membership.md`, "Wraps"), then
//! from its Commits, on mls-rs's `ExternalGroup`, without group secrets. The server saves the view
//! in its database and loads it again, so that every instance and a restart go on from it
//! ("Commit ordering").

use mls_rs::{
    WireFormat,
    extension::{MlsCodecExtension as _, built_in::ExternalPubExt},
    external_client::{
        ExternalClient, ExternalGroup, ExternalReceivedMessage, ExternalSnapshot,
        builder::{ExternalBaseConfig, WithCryptoProvider, WithIdentityProvider},
    },
    group::CommitEffect,
    mls_rs_codec::MlsDecode as _,
};
use mls_rs_crypto_rustcrypto::RustCryptoProvider;
use skald_proto::ChannelId;

use super::{
    MemberDevice, MlsError, key_package, member_device, members_of, message,
    profile::{Profile, SUITE},
};

/// The config of mls-rs's party outside a group, with the profile's identity provider and
/// ciphersuite; spelled out, as a struct field cannot hold an `impl` type.
type Config =
    WithIdentityProvider<Profile, WithCryptoProvider<RustCryptoProvider, ExternalBaseConfig>>;

/// mls-rs's party outside a group, with the profile's identity provider and ciphersuite.
fn client() -> ExternalClient<Config> {
    ExternalClient::builder()
        .crypto_provider(RustCryptoProvider::with_enabled_cipher_suites(vec![SUITE]))
        .identity_provider(Profile)
        .build()
}

/// A channel's epoch and members, as a party outside its group sees them.
#[derive(Clone)]
pub struct ChannelView {
    group: ExternalGroup<Config>,
    /// The leaf index of [`ChannelView::signer`].
    signer_index: u32,
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
        let group = client().observe_group(group_info, None, None)?;
        Self::of(group, signer)
    }

    /// As [`ChannelView::observe`], for a GroupInfo a member device uploads for others to join
    /// from ("MLS profile"): refused also without the `external_pub` extension.
    ///
    /// # Errors
    /// As [`ChannelView::observe`]; [`MlsError::NotJoinable`] without `external_pub`.
    pub fn observe_joinable(channel: ChannelId, group_info: &[u8]) -> Result<Self, MlsError> {
        let message = message(group_info, WireFormat::GroupInfo)?;
        let info = message.as_group_info().ok_or(MlsError::Malformed)?;
        if !info
            .extensions()
            .has_extension(ExternalPubExt::extension_type())
        {
            return Err(MlsError::NotJoinable);
        }
        Self::observe(channel, group_info)
    }

    /// The view's bytes, for [`ChannelView::load`]: the signer's leaf index as 4 bytes,
    /// big-endian, then mls-rs's snapshot of the group with its tree.
    ///
    /// # Errors
    /// [`MlsError::Mls`] if mls-rs fails to encode the snapshot.
    pub fn save(&self) -> Result<Vec<u8>, MlsError> {
        let snapshot = self.group.snapshot().to_bytes()?;
        Ok([self.signer_index.to_be_bytes().as_slice(), &snapshot].concat())
    }

    /// The view of `channel` that `saved`, bytes [`ChannelView::save`] gave, holds.
    ///
    /// # Errors
    /// [`MlsError::Malformed`] if `saved` is not a saved view; [`MlsError::OtherChannel`] if it is
    /// one of another channel; [`MlsError::Lifetime`] or [`MlsError::Mls`] as
    /// [`ChannelView::observe`] gives them for its tree.
    pub fn load(channel: ChannelId, saved: &[u8]) -> Result<Self, MlsError> {
        let (signer, mut snapshot) = saved.split_first_chunk::<4>().ok_or(MlsError::Malformed)?;
        let decoded =
            ExternalSnapshot::mls_decode(&mut snapshot).map_err(|_| MlsError::Malformed)?;
        if !snapshot.is_empty() {
            return Err(MlsError::Malformed);
        }
        if decoded.context().group_id != channel.as_bytes() {
            return Err(MlsError::OtherChannel);
        }
        let group = client().load_group(decoded)?;
        Self::of(group, u32::from_be_bytes(*signer))
    }

    /// Whether `group_info`, a GroupInfo in an `MLSMessage`, has the GroupContext of the view's
    /// epoch: of its group and epoch, with its tree and transcript. `false` for anything else; the
    /// GroupInfo's signature is not checked.
    pub fn has_context(&self, group_info: &[u8]) -> bool {
        message(group_info, WireFormat::GroupInfo)
            .ok()
            .and_then(|message| message.into_group_info())
            .is_some_and(|info| info.group_context() == self.group.group_context())
    }

    /// The view of the next epoch from `commit`, a Commit of the view's epoch in a PublicMessage;
    /// the view stays as it was, for the caller to keep or drop the new one. Refused where a
    /// member refuses it, such as a Commit of another epoch or group or one whose signature does
    /// not verify with its sender leaf's key, but for what takes the group's secrets, its
    /// membership tag, its confirmation tag and the UpdatePath's encrypted path secrets, and for
    /// whether an Add's KeyPackage lifetime holds, which takes a time.
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

    fn of(group: ExternalGroup<Config>, signer_index: u32) -> Result<Self, MlsError> {
        key_package::check_spans(&group.exported_tree())?;
        let (signer, members) = {
            let roster = group.roster();
            let signer = roster.member_with_index(signer_index)?.signing_identity;
            (member_device(&signer)?, members_of(&roster)?)
        };
        Ok(Self {
            group,
            signer_index,
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
