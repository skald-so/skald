//! Signing revocations and counting them: `docs/protocol/membership.md`, "Revocation", and
//! "Device checks", checks 3 and 4.

use skald_proto::{
    AccountId, Cbor, Cert, DecodeError, DeviceId, DeviceKind, REVOCATION_LABEL, Revocation,
    RevocationBody,
};

use crate::cert::{DeviceCheckError, check_chain};
use crate::sign::{SigningKey, verify};

/// The revocation of `body`, signed by `key`: the signing key of the device in `body.signer`, or
/// the master key when `signer` is `None`.
///
/// # Errors
/// Why `body` breaks the spec's CDDL.
pub fn sign_revocation(body: &RevocationBody, key: &SigningKey) -> Result<Revocation, DecodeError> {
    let bytes = body.to_cbor();
    let signature = key.sign(REVOCATION_LABEL, &[&bytes]);
    Revocation::from_parts(bytes, signature)
}

/// The valid certificate of `device` among `certs` against the pinned `master` key, with the
/// `revocations` the server returns with them: checks 1 to 4 of "Device checks".
///
/// # Errors
/// [`check_device`](crate::check_device)'s error when check 1 or 2 fails, revoked or not;
/// otherwise [`DeviceCheckError::Revoked`] when check 3 or 4 does.
pub fn check_valid_device<'c>(
    master: &[u8; 32],
    certs: &'c [Cert],
    revocations: &[Revocation],
    device: DeviceId,
) -> Result<&'c Cert, DeviceCheckError> {
    let (cert, chain) = check_chain(master, certs, device)?;
    let counting = counting(master, certs, revocations, cert.body().account);
    unrevoked(&counting, &chain)?;
    Ok(cert)
}

/// Whether `revocation` counts, against `master` and `certs` of its account, beside the
/// `revocations` stored before it: what a server checks before it stores one.
#[must_use]
pub fn counts(
    master: &[u8; 32],
    certs: &[Cert],
    revocations: &[Revocation],
    revocation: &Revocation,
) -> bool {
    let all: Vec<Revocation> = revocations.iter().chain([revocation]).cloned().collect();
    let new = all.last().expect("the revocation");
    counting(master, certs, &all, new.body().account)
        .iter()
        .any(|counting| std::ptr::eq(*counting, new))
}

/// The devices of `account` among `certs` that checks 3 and 4 of "Device checks" revoke against
/// `master` and `revocations`, without checks 1 and 2: for a server, which made those when it took
/// each certificate. A device's chain follows the `issuer` of each certificate among `certs` up to
/// one without `issuer`, one missing or a repeated device.
#[must_use]
pub fn revoked_devices(
    master: &[u8; 32],
    certs: &[Cert],
    revocations: &[Revocation],
    account: AccountId,
) -> Vec<DeviceId> {
    let counting = counting(master, certs, revocations, account);
    let mut revoked: Vec<DeviceId> = certs
        .iter()
        .map(Cert::body)
        .filter(|body| body.account == account)
        .map(|body| body.device)
        .filter(|&device| unrevoked(&counting, &issuers(certs, device)).is_err())
        .collect();
    revoked.sort_unstable();
    revoked.dedup();
    revoked
}

/// `device`, then each issuer up that the `issuer` of its certificate among `certs` names, unchecked.
fn issuers(certs: &[Cert], device: DeviceId) -> Vec<DeviceId> {
    let mut chain = vec![device];
    let issuer_of = |device: DeviceId| {
        certs
            .iter()
            .map(Cert::body)
            .find(|body| body.device == device)
            .and_then(|body| body.issuer)
    };
    while let Some(issuer) = issuer_of(chain[chain.len() - 1]) {
        if chain.contains(&issuer) {
            break;
        }
        chain.push(issuer);
    }
    chain
}

/// Checks 3 and 4 of "Device checks" for the device of `chain`, the device then each issuer up,
/// with the `counting` revocations of its account.
fn unrevoked(counting: &[&Revocation], chain: &[DeviceId]) -> Result<(), DeviceCheckError> {
    let device = chain[0];
    // Check 3.
    if counting
        .iter()
        .any(|revocation| names(revocation.body(), device))
    {
        return Err(DeviceCheckError::Revoked { device });
    }
    // Check 4: each issuer up the chain, and the device below it.
    for pair in chain.windows(2) {
        let [below, issuer] = [pair[0], pair[1]];
        let kept = counting
            .iter()
            .map(|revocation| revocation.body())
            .filter(|revocation| names(revocation, issuer))
            .all(|revocation| lists(revocation.keep.as_deref(), below));
        if !kept {
            return Err(DeviceCheckError::Revoked { device: issuer });
        }
    }
    Ok(())
}

/// The counting revocations of `account`. The key `signer` names alone verifies one: without it,
/// the master key; with it, the certificate of that `client` or `browser` device of the account,
/// valid by checks 1 and 2, and then the revocation has no `also`. A master-key revocation always
/// counts and voids those of the devices it marks as stolen.
fn counting<'r>(
    master: &[u8; 32],
    certs: &[Cert],
    revocations: &'r [Revocation],
    account: AccountId,
) -> Vec<&'r Revocation> {
    let mut by_master = Vec::new();
    // Each revocation by a device, with its signer's chain.
    let mut by_devices = Vec::new();
    for revocation in revocations {
        let body = revocation.body();
        if body.account != account {
            continue;
        }
        let verifies = |key: &[u8; 32]| {
            verify(
                key,
                REVOCATION_LABEL,
                &[revocation.body_bytes()],
                revocation.signature(),
            )
            .is_ok()
        };
        match body.signer {
            None if verifies(master) => by_master.push(revocation),
            None => {}
            Some(signer) if body.also.is_none() => {
                let Ok((cert, chain)) = check_chain(master, certs, signer) else {
                    continue;
                };
                let signer_body = cert.body();
                if signer_body.account == account
                    && matches!(signer_body.kind, DeviceKind::Client | DeviceKind::Browser)
                    && verifies(&signer_body.signing)
                {
                    by_devices.push((revocation, chain));
                }
            }
            Some(_) => {}
        }
    }
    let voided = |chain: &[DeviceId]| {
        by_master
            .iter()
            .any(|stolen| marks_stolen(stolen.body(), chain))
    };
    let counting_by_devices: Vec<_> = by_devices
        .iter()
        .filter(|(_, chain)| !voided(chain))
        .map(|(revocation, _)| *revocation)
        .collect();
    by_master.extend(counting_by_devices);
    by_master
}

/// Whether master-key revocation `stolen` marks as stolen the device of `chain`, the device then
/// each issuer up: its chain passes through `stolen.device` other than through a device in `keep`.
fn marks_stolen(stolen: &RevocationBody, chain: &[DeviceId]) -> bool {
    chain
        .iter()
        .position(|&device| device == stolen.device)
        .is_some_and(|at| {
            !chain[..at]
                .iter()
                .any(|&device| lists(stolen.keep.as_deref(), device))
        })
}

/// Whether `revocation` names `device` in `device` or `also`.
fn names(revocation: &RevocationBody, device: DeviceId) -> bool {
    revocation.device == device || lists(revocation.also.as_deref(), device)
}

fn lists(devices: Option<&[DeviceId]>, device: DeviceId) -> bool {
    devices.is_some_and(|devices| devices.contains(&device))
}
