//! KeyPackages as `docs/protocol/membership.md`, "KeyPackages", says: the lifetime of a device's
//! own, and the checks a device runs on another's before it adds it.

use mls_rs::{
    CipherSuite, MlsMessage, ProtocolVersion,
    crypto::HpkePublicKey,
    group::{ExportedTree, LeafNode, LeafNodeSource, Node},
    mls_rs_codec::{MlsDecode as _, MlsEncode as _},
};
use skald_proto::Cert;

use super::{MlsError, profile};

/// A KeyPackage's lifetime starts an hour before the caller's time, for clocks that are behind.
const MARGIN: u64 = 3_600;
/// The lifetime of a device's own KeyPackages, and the longest one it adds: 60 days.
pub(super) const LIFETIME: u64 = 60 * 24 * 3_600;

/// A new KeyPackage of the device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyPackage {
    /// The KeyPackage in an `MLSMessage`.
    pub message: Vec<u8>,
    /// The end of its lifetime, its `not_after`, in seconds since the Unix epoch: the caller
    /// replaces it 7 days before.
    pub expires: u64,
}

impl AsRef<[u8]> for KeyPackage {
    fn as_ref(&self) -> &[u8] {
        &self.message
    }
}

/// The start of the lifetime of a KeyPackage made at the caller's time `now`.
///
/// # Errors
/// [`MlsError::Time`] if its end is past the last second `u64` holds.
pub(super) fn lifetime_start(now: u64) -> Result<u64, MlsError> {
    let start = now.saturating_sub(MARGIN);
    match start.checked_add(LIFETIME) {
        Some(_) => Ok(start),
        None => Err(MlsError::Time),
    }
}

/// Checks the KeyPackage in `message` before an Add at the caller's time `now`: ciphersuite
/// `0x0001`, a lifetime that holds at `now` and spans at most [`LIFETIME`], and a leaf that keeps
/// to the profile with the device ID and signing key of one of the `admitted` certificates. mls-rs
/// verifies its signatures when it commits the Add.
///
/// # Errors
/// [`MlsError::Malformed`] if `message` holds no KeyPackage; [`MlsError::Profile`] if its
/// ciphersuite or leaf breaks the profile; [`MlsError::Lifetime`] if its lifetime fails;
/// [`MlsError::NotAdmitted`] if no certificate of `admitted` is its device's.
pub(super) fn check(message: &MlsMessage, admitted: &[Cert], now: u64) -> Result<(), MlsError> {
    let package = message.as_key_package().ok_or(MlsError::Malformed)?;
    if package.cipher_suite() != profile::SUITE {
        return Err(MlsError::Profile);
    }
    let leaf = leaf_node(
        &package
            .mls_encode_to_vec()
            .map_err(mls_rs::error::MlsError::from)?,
    )?;
    let (not_before, not_after) = lifetime(&leaf).ok_or(MlsError::Malformed)?;
    if now < not_before || now > not_after || !spans_at_most_a_lifetime(&leaf) {
        return Err(MlsError::Lifetime);
    }
    let (device, signing) =
        profile::device_of(package.signing_identity()).map_err(|_| MlsError::Profile)?;
    let admits = |cert: &Cert| cert.body().device == device && cert.body().signing == signing;
    if admitted.iter().any(admits) {
        Ok(())
    } else {
        Err(MlsError::NotAdmitted(device))
    }
}

/// The leaf node of the KeyPackage `encoded`, which mls-rs keeps private: the leaf follows the
/// version, the ciphersuite and the init key.
fn leaf_node(mut encoded: &[u8]) -> Result<LeafNode, MlsError> {
    let reader = &mut encoded;
    ProtocolVersion::mls_decode(reader)
        .and_then(|_| CipherSuite::mls_decode(reader))
        .and_then(|_| HpkePublicKey::mls_decode(reader))
        .and_then(|_| LeafNode::mls_decode(reader))
        .map_err(|_| MlsError::Malformed)
}

/// The `not_before` and `not_after` of `leaf`, if it has a lifetime: a KeyPackage's leaf, or a
/// member's leaf that its KeyPackage's leaf still is.
fn lifetime(leaf: &LeafNode) -> Option<(u64, u64)> {
    match &leaf.leaf_node_source {
        LeafNodeSource::KeyPackage(lifetime) => Some((
            lifetime.not_before.seconds_since_epoch(),
            lifetime.not_after.seconds_since_epoch(),
        )),
        _ => None,
    }
}

/// Whether the lifetime of `leaf`, if it has one, spans at most [`LIFETIME`], whether or not it
/// holds.
fn spans_at_most_a_lifetime(leaf: &LeafNode) -> bool {
    lifetime(leaf)
        .is_none_or(|(not_before, not_after)| not_after.saturating_sub(not_before) <= LIFETIME)
}

/// Checks that the lifetime of each leaf of `tree` that has one spans at most [`LIFETIME`], as
/// RFC 9420, §7.2, requires a bound; whether it still holds is not checked.
///
/// # Errors
/// [`MlsError::Lifetime`] if one spans more.
pub(super) fn check_spans(tree: &ExportedTree<'_>) -> Result<(), MlsError> {
    let spans = tree.nodes().iter().flatten().all(|node| match node {
        Node::Leaf(leaf) => spans_at_most_a_lifetime(leaf),
        Node::Parent(_) => true,
    });
    if spans {
        Ok(())
    } else {
        Err(MlsError::Lifetime)
    }
}
