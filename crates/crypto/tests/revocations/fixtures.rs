//! The tests' accounts, their devices and revocations. The client under test pins the account's
//! master key; another account has a device of its own.
//!
//! The account's chains:
//!
//! ```text
//! master ─┬─ LAPTOP ─┬─ DESKTOP ─┬─ PHONE ── PHONE_BROWSER
//!         │          │           └─ TABLET
//!         │          ├─ BROWSER
//!         │          └─ SERVICE
//!         └─ RECOVERED
//! ```
//!
//! Revocations are built from their spec fields, encoded by [`crate::cbor`] and signed with
//! ed25519-dalek over `["skald revocation", body]`, apart from the crates under test.

use ed25519_dalek::Signer as _;
use skald_crypto::{DeviceCheckError, SigningKey, check_valid_device, issue};
use skald_proto::{AccountId, Cbor, Cert, CertBody, DeviceId, DeviceKind, Revocation, Unknown};

use crate::cbor::{Value, bytes, text};

/// The label of a revocation's signature.
pub const LABEL: &str = "skald revocation";
/// The time a revocation states, for display.
pub const TIMESTAMP: u64 = 1_800_000_000;

pub const ACCOUNT: AccountId = AccountId::from_bytes([0xac; 16]);
pub const OTHER_ACCOUNT: AccountId = AccountId::from_bytes([0xad; 16]);

// Keys by seed: master keys, and a key of the server's own, which no chain ends at.
pub const MASTER: u8 = 1;
pub const OTHER_MASTER: u8 = 2;
pub const SERVER_KEY: u8 = 3;

// The account's devices, by the seed of their signing key.
/// The account's first `client` device, certified by the master key.
pub const LAPTOP: u8 = 10;
/// A `client` device the laptop certified.
pub const DESKTOP: u8 = 11;
/// A `browser` device the laptop certified.
pub const BROWSER: u8 = 12;
/// A `service` device the laptop certified.
pub const SERVICE: u8 = 13;
/// A `client` device the desktop certified.
pub const PHONE: u8 = 14;
/// Another `client` device the desktop certified.
pub const TABLET: u8 = 15;
/// A `browser` device the phone certified: its chain is the phone, the desktop, the laptop.
pub const PHONE_BROWSER: u8 = 16;
/// A `client` device the master key certified, as one linked with the recovery key.
pub const RECOVERED: u8 = 17;
/// A `client` device of the account whose certificate the server's key signed: no valid one.
pub const BY_SERVER: u8 = 18;
/// A device with no certificate.
pub const UNCERTIFIED: u8 = 19;
/// The `client` device of the other account, certified by its master key.
pub const OTHER_LAPTOP: u8 = 20;
/// A `client` device of the other account certified by this account's master key.
pub const FOREIGN: u8 = 21;

/// The account's devices with a valid certificate.
pub const CERTIFIED: [u8; 9] = [
    LAPTOP,
    DESKTOP,
    BROWSER,
    SERVICE,
    PHONE,
    TABLET,
    PHONE_BROWSER,
    RECOVERED,
    OTHER_LAPTOP,
];

/// The laptop and every device under it.
pub const LAPTOP_AND_UNDER: [u8; 7] = [
    LAPTOP,
    DESKTOP,
    BROWSER,
    SERVICE,
    PHONE,
    TABLET,
    PHONE_BROWSER,
];
/// The desktop and every device under it.
pub const DESKTOP_AND_UNDER: [u8; 4] = [DESKTOP, PHONE, TABLET, PHONE_BROWSER];

/// Each certificate: its device, account, kind, the issuer device or `None` for a master key, and
/// the seed of the key that signs it.
const CERTS: [(u8, AccountId, DeviceKind, Option<u8>, u8); 11] = [
    (LAPTOP, ACCOUNT, DeviceKind::Client, None, MASTER),
    (DESKTOP, ACCOUNT, DeviceKind::Client, Some(LAPTOP), LAPTOP),
    (BROWSER, ACCOUNT, DeviceKind::Browser, Some(LAPTOP), LAPTOP),
    (SERVICE, ACCOUNT, DeviceKind::Service, Some(LAPTOP), LAPTOP),
    (PHONE, ACCOUNT, DeviceKind::Client, Some(DESKTOP), DESKTOP),
    (TABLET, ACCOUNT, DeviceKind::Client, Some(DESKTOP), DESKTOP),
    (
        PHONE_BROWSER,
        ACCOUNT,
        DeviceKind::Browser,
        Some(PHONE),
        PHONE,
    ),
    (RECOVERED, ACCOUNT, DeviceKind::Client, None, MASTER),
    (BY_SERVER, ACCOUNT, DeviceKind::Client, None, SERVER_KEY),
    (
        OTHER_LAPTOP,
        OTHER_ACCOUNT,
        DeviceKind::Client,
        None,
        OTHER_MASTER,
    ),
    (FOREIGN, OTHER_ACCOUNT, DeviceKind::Client, None, MASTER),
];

/// The tests' name of device `seed`, for failure messages.
pub fn name(seed: u8) -> &'static str {
    match seed {
        LAPTOP => "laptop",
        DESKTOP => "desktop",
        BROWSER => "browser",
        SERVICE => "service",
        PHONE => "phone",
        TABLET => "tablet",
        PHONE_BROWSER => "phone browser",
        RECOVERED => "recovered",
        BY_SERVER => "by server",
        UNCERTIFIED => "uncertified",
        OTHER_LAPTOP => "other laptop",
        FOREIGN => "foreign",
        _ => "unknown",
    }
}

fn dalek(seed: u8) -> ed25519_dalek::SigningKey {
    ed25519_dalek::SigningKey::from_bytes(&[seed; 32])
}

/// The key `seed` as the crate under test takes it.
pub fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

pub fn public(seed: u8) -> [u8; 32] {
    dalek(seed).verifying_key().to_bytes()
}

pub fn device(seed: u8) -> DeviceId {
    DeviceId::from_signing_key(&public(seed))
}

/// The signature of key `seed` over the encoded `[label, body]`.
pub fn sign(seed: u8, label: &str, body: &[u8]) -> [u8; 64] {
    let message = Value::Array(vec![text(label), bytes(body)]).encode();
    dalek(seed).sign(&message).to_bytes()
}

/// Every certificate, of both accounts, as the server returns them.
pub fn certs() -> Vec<Cert> {
    CERTS
        .iter()
        .map(|&(seed, account, kind, issuer, by)| {
            let body = CertBody {
                name: format!("{} device", name(seed)),
                issued: TIMESTAMP - 1_000,
                issuer: issuer.map(device),
                hpke: [seed ^ 0x55; 32],
                signing: public(seed),
                kind,
                device: device(seed),
                account,
                ssh: None,
                github: None,
                unknown: Unknown::default(),
            };
            issue(&body, &key(by), &key(seed)).expect("a fixture certificate")
        })
        .collect()
}

/// A revocation's spec fields, devices by seed, and how it is signed.
#[derive(Debug, Clone)]
pub struct Rev {
    pub account: AccountId,
    pub device: u8,
    pub keep: Vec<u8>,
    /// `signer`; `None`, the master key.
    pub signer: Option<u8>,
    pub also: Vec<u8>,
    /// Fields of later versions.
    pub extra: Vec<(i64, Value)>,
    /// The seed of the key that signs it.
    pub key: u8,
    /// The label its signature covers.
    pub label: &'static str,
    /// Whether one bit of its signature is flipped.
    pub broken: bool,
}

impl Rev {
    /// A revocation of `device` on the account by its device `signer`, with that device's key.
    pub fn by(signer: u8, device: u8) -> Self {
        Self {
            account: ACCOUNT,
            device,
            keep: Vec::new(),
            signer: Some(signer),
            also: Vec::new(),
            extra: Vec::new(),
            key: signer,
            label: LABEL,
            broken: false,
        }
    }

    /// A revocation of `device` on the account by its master key.
    pub fn by_master(device: u8) -> Self {
        Self {
            signer: None,
            key: MASTER,
            ..Self::by(LAPTOP, device)
        }
    }

    pub fn keep(self, devices: &[u8]) -> Self {
        Self {
            keep: devices.to_vec(),
            ..self
        }
    }

    pub fn also(self, devices: &[u8]) -> Self {
        Self {
            also: devices.to_vec(),
            ..self
        }
    }

    /// The body, a map.
    pub fn value(&self) -> Value {
        let id = |seed: u8| bytes(device(seed).as_bytes());
        let mut fields = vec![
            (1, bytes(self.account.as_bytes())),
            (2, id(self.device)),
            (5, Value::Uint(TIMESTAMP)),
        ];
        if !self.keep.is_empty() {
            fields.push((3, Value::Array(self.keep.iter().copied().map(id).collect())));
        }
        if let Some(signer) = self.signer {
            fields.push((4, id(signer)));
        }
        if !self.also.is_empty() {
            fields.push((6, Value::Array(self.also.iter().copied().map(id).collect())));
        }
        fields.extend(self.extra.iter().cloned());
        Value::Map(fields)
    }

    /// The encoded body.
    pub fn body(&self) -> Vec<u8> {
        self.value().encode()
    }

    /// The signature by `key` over `[label, body]`, with a bit flipped if `broken`.
    pub fn signature(&self) -> [u8; 64] {
        let mut signature = sign(self.key, self.label, &self.body());
        if self.broken {
            signature[17] ^= 0x04;
        }
        signature
    }

    /// The encoded revocation: `[body, signature]`.
    pub fn bytes(&self) -> Vec<u8> {
        Value::Array(vec![bytes(&self.body()), bytes(&self.signature())]).encode()
    }

    /// The revocation as the server returns it, decoded.
    pub fn revocation(&self) -> Revocation {
        Revocation::from_cbor(&self.bytes()).expect("a fixture revocation decodes")
    }
}

/// What a check of a device's certificate says of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Checks 1 to 4 pass.
    Valid,
    /// Checks 1 and 2 pass, and check 3 or 4 fails.
    Revoked,
    /// Check 1 or 2 fails: the device has no valid certificate, revoked or not.
    NoValidCertificate,
}

fn status_with(revocations: &[Revocation], seed: u8) -> Status {
    let master = public(if seed == OTHER_LAPTOP {
        OTHER_MASTER
    } else {
        MASTER
    });
    let certs = certs();
    match check_valid_device(&master, &certs, revocations, device(seed)) {
        Ok(cert) => {
            assert_eq!(cert.body().device, device(seed), "{}", name(seed));
            Status::Valid
        }
        Err(DeviceCheckError::Revoked { .. }) => Status::Revoked,
        Err(_) => Status::NoValidCertificate,
    }
}

/// What the client says of device `seed`, with its account's master key pinned, every
/// certificate and `revs`; the same whatever their order.
pub fn status(revs: &[Rev], seed: u8) -> Status {
    let mut revocations: Vec<Revocation> = revs.iter().map(Rev::revocation).collect();
    let given = status_with(&revocations, seed);
    revocations.reverse();
    assert_eq!(
        status_with(&revocations, seed),
        given,
        "{} with the revocations in reverse order",
        name(seed)
    );
    given
}

/// Checks that each device of `expected` has its status with `revs`.
pub fn assert_statuses(revs: &[Rev], expected: &[(u8, Status)]) {
    let wrong: Vec<String> = expected
        .iter()
        .filter_map(|&(seed, want)| {
            let got = status(revs, seed);
            (got != want).then(|| format!("{}: {got:?}, not {want:?}", name(seed)))
        })
        .collect();
    assert!(wrong.is_empty(), "{wrong:#?}\nwith {revs:#?}");
}

/// Checks that, with `revs`, the devices in `revoked` are revoked and every other certified
/// device is valid.
pub fn assert_revoked(revs: &[Rev], revoked: &[u8]) {
    let expected: Vec<(u8, Status)> = CERTIFIED
        .iter()
        .map(|&seed| {
            let want = if revoked.contains(&seed) {
                Status::Revoked
            } else {
                Status::Valid
            };
            (seed, want)
        })
        .collect();
    assert_statuses(revs, &expected);
}
