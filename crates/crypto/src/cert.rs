//! Issuing device certificates and checking them against a pinned master key:
//! `docs/protocol/membership.md`, "Device certificate" and "Device checks", checks 1 and 2.

use std::fmt;

use skald_proto::{
    CERT_LABEL, Cbor, Cert, CertBody, DecodeError, DeviceId, DeviceKind, PROOF_LABEL,
};

use crate::sign::{SigningKey, verify};

/// The certificate of `body`: `issuer`, the master key or an issuer device, signs it, and
/// `device`, the key of `body.signing`, proves it.
///
/// # Errors
/// Why `body` breaks the spec's CDDL, such as a `name` that is empty or longer than 64 bytes.
pub fn issue(
    body: &CertBody,
    issuer: &SigningKey,
    device: &SigningKey,
) -> Result<Cert, DecodeError> {
    let bytes = body.to_cbor();
    let signature = issuer.sign(CERT_LABEL, &[&bytes]);
    let proof = device.sign(PROOF_LABEL, &[&bytes]);
    Cert::from_parts(bytes, signature, proof)
}

/// Why a device has no valid certificate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceCheckError {
    /// No certificate of the device passes the proof check.
    NoCertificate,
    /// Certificates of the device with different bodies pass the proof check, so none is valid;
    /// the client warns its user.
    TwoBodies,
    /// The chain fails at `device`: the checked device or an issuer in its chain.
    Chain {
        device: DeviceId,
        problem: ChainProblem,
    },
}

/// How a certificate chain fails at a device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainProblem {
    /// The device is an issuer with no certificate that passes the proof check.
    NoCertificate,
    /// The device is an issuer with certificates of different bodies that pass the proof check.
    TwoBodies,
    /// No copy of the device's certificate has a `signature` that verifies with its issuer's key:
    /// check 1.
    Signature,
    /// The device's issuer is a device the chain has passed.
    Repeats,
    /// The device's issuer is not a `client` device.
    IssuerKind,
    /// The master key issued the device, which is not a `client` one.
    MasterKind,
    /// The device's issuer is on another account.
    OtherAccount,
}

impl fmt::Display for DeviceCheckError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoCertificate => {
                f.write_str("no certificate of the device passes the proof check")
            }
            Self::TwoBodies => f.write_str("the device has certificates with different bodies"),
            Self::Chain { device, problem } => write!(f, "the certificate of {device}: {problem}"),
        }
    }
}

impl fmt::Display for ChainProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NoCertificate => "no certificate passes the proof check",
            Self::TwoBodies => "certificates with different bodies",
            Self::Signature => "no signature verifies with its issuer's key",
            Self::Repeats => "its issuer repeats a device of the chain",
            Self::IssuerKind => "its issuer is not a client device",
            Self::MasterKind => "the master key issues only client devices",
            Self::OtherAccount => "its issuer is on another account",
        })
    }
}

impl std::error::Error for DeviceCheckError {}

/// The valid certificate of `device` among `certs`, as a server returns them, against the pinned
/// `master` key: the proof check and checks 1 and 2 of "Device checks". Copies with one body count
/// as one, and the one returned is a copy whose `signature` passes check 1.
///
/// # Errors
/// Why `device` has no valid certificate; [`DeviceCheckError::TwoBodies`] is for its user's
/// warning.
pub fn check_device<'c>(
    master: &[u8; 32],
    certs: &'c [Cert],
    device: DeviceId,
) -> Result<&'c Cert, DeviceCheckError> {
    let mut copies = copies_of(certs, device).map_err(|problem| match problem {
        ChainProblem::TwoBodies => DeviceCheckError::TwoBodies,
        _ => DeviceCheckError::NoCertificate,
    })?;
    let mut chain = vec![device];
    let mut found = None;
    loop {
        let first: &'c Cert = copies[0];
        let body = first.body();
        let fail = |problem| DeviceCheckError::Chain {
            device: body.device,
            problem,
        };
        // The key that signs this certificate, and the copies of its issuer's certificate.
        let (key, next) = match body.issuer {
            None if body.kind != DeviceKind::Client => return Err(fail(ChainProblem::MasterKind)),
            None => (*master, None),
            Some(issuer) => {
                if chain.contains(&issuer) {
                    return Err(fail(ChainProblem::Repeats));
                }
                chain.push(issuer);
                let issuer_copies =
                    copies_of(certs, issuer).map_err(|problem| DeviceCheckError::Chain {
                        device: issuer,
                        problem,
                    })?;
                let issuer_body = issuer_copies[0].body();
                if issuer_body.kind != DeviceKind::Client {
                    return Err(fail(ChainProblem::IssuerKind));
                }
                if issuer_body.account != body.account {
                    return Err(fail(ChainProblem::OtherAccount));
                }
                (issuer_body.signing, Some(issuer_copies))
            }
        };
        let cert = copies
            .into_iter()
            .find(|cert| verify(&key, CERT_LABEL, &[cert.body_bytes()], cert.signature()).is_ok())
            .ok_or(fail(ChainProblem::Signature))?;
        let valid = *found.get_or_insert(cert);
        match next {
            None => return Ok(valid),
            Some(next) => copies = next,
        }
    }
}

/// The copies of the one body of `device`'s certificates that pass the proof check.
fn copies_of(certs: &[Cert], device: DeviceId) -> Result<Vec<&Cert>, ChainProblem> {
    let mut passing = certs
        .iter()
        .filter(|cert| cert.body().device == device && passes_proof_check(cert));
    let first = passing.next().ok_or(ChainProblem::NoCertificate)?;
    let mut copies = vec![first];
    for cert in passing {
        if cert.body_bytes() != first.body_bytes() {
            return Err(ChainProblem::TwoBodies);
        }
        copies.push(cert);
    }
    Ok(copies)
}

/// The proof check: `device` is derived from `signing`, and `proof` verifies with `signing`.
fn passes_proof_check(cert: &Cert) -> bool {
    let body = cert.body();
    body.device.is_derived_from(&body.signing)
        && verify(
            &body.signing,
            PROOF_LABEL,
            &[cert.body_bytes()],
            cert.proof(),
        )
        .is_ok()
}
