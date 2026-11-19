//! Checking a service grant as a recipient: `docs/protocol/README.md`, "Service grant", checks 1
//! to 3.

use std::fmt;

use skald_proto::{
    AccountId, Cbor, Cert, ChannelId, DeviceId, GRANT_LABEL, Grant, Revocation, Roster,
};

use crate::roster::{admins_device, master_of};
use crate::sign::verify;

/// Why a recipient rejects the grant in content from a `service` device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrantError {
    /// Check 1: `service` is not the sending device.
    Service,
    /// Check 1: `channels` does not list the content's channel.
    Channel,
    /// Check 2: `signer` is not an admin's device in the revision, or is revoked.
    Signer(DeviceId),
    /// Check 2: the signature does not verify with the key of `signer`'s certificate.
    Signature,
    /// Check 3: `expires` has passed.
    Expired,
}

impl fmt::Display for GrantError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Service => f.write_str("its grant names another service device"),
            Self::Channel => f.write_str("its grant does not list the channel"),
            Self::Signer(device) => write!(
                f,
                "the signer {device} of its grant is no valid admin's device in the revision"
            ),
            Self::Signature => {
                f.write_str("its grant's signature does not verify with the signer's key")
            }
            Self::Expired => f.write_str("its grant has expired"),
        }
    }
}

impl std::error::Error for GrantError {}

/// Whether `grant`, in content from the device `service` in `channel`, passes checks 1 to 3
/// against `roster`, the latest revision the client of `account` accepted, with its `master` key
/// pinned, and the certificates and revocations the server returns, `certs` and `revocations`, at
/// `now`, seconds since the Unix epoch. The signer's device passes checks 1 to 4 of "Device checks"
/// against `master` for `account` and against the `master` the revision gives any other account.
///
/// # Errors
/// The first check that fails.
pub fn check_grant(
    grant: &Grant,
    (service, channel): (DeviceId, ChannelId),
    roster: &Roster,
    certs: &[Cert],
    revocations: &[Revocation],
    (account, master): (AccountId, &[u8; 32]),
    now: u64,
) -> Result<(), GrantError> {
    let body = &grant.body;
    // Check 1.
    if body.service != service {
        return Err(GrantError::Service);
    }
    if !body.channels.contains(&channel) {
        return Err(GrantError::Channel);
    }

    // Check 2.
    let revision = roster.body();
    let signer = admins_device(revision, (certs, revocations), body.signer, |member| {
        master_of(revision, (account, master), member)
    })
    .ok_or(GrantError::Signer(body.signer))?;
    verify(&signer, GRANT_LABEL, &[&body.to_cbor()], &grant.signature)
        .map_err(|_| GrantError::Signature)?;

    // Check 3.
    if now > body.expires {
        return Err(GrantError::Expired);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each reason names the check that failed.
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn each_error_says_why_the_grant_fails() {
        let signer: DeviceId = "a90f5ea9-75e5-899a-90ec-3a09f33aebdf".parse().unwrap();
        let reasons = [
            (
                GrantError::Service,
                "its grant names another service device",
            ),
            (GrantError::Channel, "its grant does not list the channel"),
            (
                GrantError::Signer(signer),
                "the signer a90f5ea9-75e5-899a-90ec-3a09f33aebdf of its grant is no valid admin's \
                 device in the revision",
            ),
            (
                GrantError::Signature,
                "its grant's signature does not verify with the signer's key",
            ),
            (GrantError::Expired, "its grant has expired"),
        ];
        for (error, reason) in reasons {
            assert_eq!(error.to_string(), reason);
        }
    }
}
