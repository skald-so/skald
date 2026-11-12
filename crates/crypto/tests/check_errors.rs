//! The reason `check_device` gives for each check that fails, `issue` with a body the spec's CDDL
//! forbids, and what keys and errors show.

use skald_crypto::{
    ChainProblem, DeviceCheckError, SignatureError, SigningKey, check_device, issue,
};
use skald_proto::{
    AccountId, Cert, CertBody, DecodeError, DeviceId, DeviceKind, FieldProblem, Unknown,
};

const MASTER: u8 = 7;

fn signing_key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn device_id(seed: u8) -> DeviceId {
    DeviceId::from_signing_key(&signing_key(seed).public_key())
}

/// A body for device `seed` on account `[account; 16]`, issued by device `issuer` or the master key.
fn body_for(seed: u8, kind: DeviceKind, issuer: Option<u8>, account: u8) -> CertBody {
    CertBody {
        account: AccountId::from_bytes([account; 16]),
        device: device_id(seed),
        kind,
        signing: signing_key(seed).public_key(),
        hpke: [0; 32],
        issuer: issuer.map(device_id),
        issued: 0,
        ssh: None,
        name: "Test".to_owned(),
        unknown: Unknown::default(),
    }
}

/// The certificate of [`body_for`], issued with [`issue`].
fn issued(seed: u8, kind: DeviceKind, issuer: Option<u8>, account: u8) -> Cert {
    let body = body_for(seed, kind, issuer, account);
    issue(
        &body,
        &signing_key(issuer.unwrap_or(MASTER)),
        &signing_key(seed),
    )
    .unwrap()
}

fn chain(seed: u8, problem: ChainProblem) -> Result<Cert, DeviceCheckError> {
    Err(DeviceCheckError::Chain {
        device: device_id(seed),
        problem,
    })
}

#[test]
fn each_failed_check_has_its_reason() {
    let master = signing_key(MASTER).public_key();
    let other_master = signing_key(MASTER + 1).public_key();
    let check = |certs: &[Cert], seed| check_device(&master, certs, device_id(seed)).cloned();
    let first = issued(1, DeviceKind::Client, None, 1);
    let second = issued(2, DeviceKind::Client, Some(1), 1);

    assert_eq!(check(&[], 1), Err(DeviceCheckError::NoCertificate));
    let other_body = issued(1, DeviceKind::Client, None, 2);
    assert_eq!(
        check(&[first.clone(), other_body.clone()], 1),
        Err(DeviceCheckError::TwoBodies)
    );
    assert_eq!(
        check(&[issued(1, DeviceKind::Browser, None, 1)], 1),
        chain(1, ChainProblem::MasterKind)
    );
    assert_eq!(
        check_device(&other_master, std::slice::from_ref(&first), device_id(1)).cloned(),
        chain(1, ChainProblem::Signature)
    );
    assert_eq!(
        check_device(
            &other_master,
            &[first.clone(), second.clone()],
            device_id(2)
        )
        .cloned(),
        chain(1, ChainProblem::Signature)
    );
    assert_eq!(
        check(&[issued(1, DeviceKind::Client, Some(1), 1)], 1),
        chain(1, ChainProblem::Repeats)
    );
    assert_eq!(
        check(std::slice::from_ref(&second), 2),
        chain(1, ChainProblem::NoCertificate)
    );
    assert_eq!(
        check(&[first.clone(), other_body, second.clone()], 2),
        chain(1, ChainProblem::TwoBodies)
    );
    assert_eq!(
        check(
            &[
                issued(1, DeviceKind::Service, Some(3), 1),
                issued(3, DeviceKind::Client, None, 1),
                issued(2, DeviceKind::Client, Some(1), 1)
            ],
            2
        ),
        chain(2, ChainProblem::IssuerKind)
    );
    assert_eq!(
        check(
            &[first.clone(), issued(2, DeviceKind::Client, Some(1), 2)],
            2
        ),
        chain(2, ChainProblem::OtherAccount)
    );
    assert_eq!(check(&[first, second.clone()], 2), Ok(second));
}

#[test]
fn a_body_the_cddl_forbids_is_not_issued() {
    for name in [String::new(), "a".repeat(65)] {
        let body = CertBody {
            name: name.clone(),
            ..body_for(1, DeviceKind::Client, None, 1)
        };
        let error = issue(&body, &signing_key(MASTER), &signing_key(1)).unwrap_err();
        let DecodeError::Field {
            field: "cert.body",
            problem: FieldProblem::Embedded(inner),
        } = error
        else {
            panic!("{error:?}");
        };
        assert_eq!(
            *inner,
            DecodeError::Field {
                field: "cert-body.name",
                problem: FieldProblem::Size {
                    min: 1,
                    max: 64,
                    actual: name.len(),
                },
            }
        );
    }
}

#[test]
fn errors_explain_themselves() {
    let device = device_id(1);
    let chain = |problem| DeviceCheckError::Chain { device, problem }.to_string();
    for (problem, text) in [
        (
            ChainProblem::NoCertificate,
            "no certificate passes the proof check",
        ),
        (
            ChainProblem::TwoBodies,
            "certificates with different bodies",
        ),
        (
            ChainProblem::Signature,
            "no signature verifies with its issuer's key",
        ),
        (
            ChainProblem::Repeats,
            "its issuer repeats a device of the chain",
        ),
        (
            ChainProblem::IssuerKind,
            "its issuer is not a client device",
        ),
        (
            ChainProblem::MasterKind,
            "the master key issues only client devices",
        ),
        (
            ChainProblem::OtherAccount,
            "its issuer is on another account",
        ),
    ] {
        assert_eq!(
            chain(problem),
            format!("the certificate of {device}: {text}")
        );
    }
    assert_eq!(
        DeviceCheckError::NoCertificate.to_string(),
        "no certificate of the device passes the proof check"
    );
    assert_eq!(
        DeviceCheckError::TwoBodies.to_string(),
        "the device has certificates with different bodies"
    );
    assert_eq!(SignatureError.to_string(), "the signature does not verify");
}

#[test]
fn a_signing_key_never_shows_its_bytes() {
    assert_eq!(format!("{:?}", signing_key(1)), "SigningKey(..)");
    assert_eq!(format!("{:#?}", SigningKey::generate()), "SigningKey(..)");
}
