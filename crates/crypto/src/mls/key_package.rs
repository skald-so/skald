//! KeyPackages as `docs/protocol/membership.md`, "KeyPackages", says: the lifetime of a device's
//! own, and the checks a device runs on another's before it adds it.

use mls_rs::{
    CipherSuite, MlsMessage, ProtocolVersion,
    crypto::HpkePublicKey,
    group::{LeafNode, LeafNodeSource},
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
    let (not_before, not_after) = lifetime(
        &package
            .mls_encode_to_vec()
            .map_err(mls_rs::error::MlsError::from)?,
    )?;
    if now < not_before || now > not_after || not_after - not_before > LIFETIME {
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

/// The `not_before` and `not_after` of the KeyPackage `encoded`, from its leaf node, which mls-rs
/// keeps private: the leaf follows the version, the ciphersuite and the init key.
fn lifetime(mut encoded: &[u8]) -> Result<(u64, u64), MlsError> {
    let reader = &mut encoded;
    let leaf = ProtocolVersion::mls_decode(reader)
        .and_then(|_| CipherSuite::mls_decode(reader))
        .and_then(|_| HpkePublicKey::mls_decode(reader))
        .and_then(|_| LeafNode::mls_decode(reader))
        .map_err(|_| MlsError::Malformed)?;
    match leaf.leaf_node_source {
        LeafNodeSource::KeyPackage(lifetime) => Ok((
            lifetime.not_before.seconds_since_epoch(),
            lifetime.not_after.seconds_since_epoch(),
        )),
        _ => Err(MlsError::Malformed),
    }
}
