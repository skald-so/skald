//! Device certificates as `docs/protocol/membership.md`, "Device certificate", defines them.

use crate::{
    DeviceId, Identity, Result,
    cbor::Value,
    identity::device_id,
    object::{self, signed, verifies},
    revocation::AccountId,
};

/// The label of a certificate's `signature`, by its issuer.
pub const LABEL: &str = "skald cert";
/// The label of a certificate's `proof`, by its device.
pub const PROOF_LABEL: &str = "skald proof";

/// The `issued` of the scenarios' certificates.
pub const ISSUED: u64 = 1_770_000_000;

/// A `device-kind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Client = 1,
    Browser = 2,
    Service = 3,
    Custodian = 4,
    Auditor = 5,
}

impl Kind {
    fn decode(value: u64) -> Result<Self> {
        Ok(match value {
            1 => Kind::Client,
            2 => Kind::Browser,
            3 => Kind::Service,
            4 => Kind::Custodian,
            5 => Kind::Auditor,
            _ => return Err(format!("cert-body: device kind {value}").into()),
        })
    }
}

/// A `cert-body`, without `ssh`, which nothing here checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Body {
    pub account: AccountId,
    pub device: DeviceId,
    pub kind: Kind,
    pub signing: [u8; 32],
    pub hpke: [u8; 32],
    /// Absent: the master key.
    pub issuer: Option<DeviceId>,
    pub issued: u64,
    pub name: String,
}

/// The fields of `cert-body`.
const ACCOUNT: u64 = 1;
const DEVICE: u64 = 2;
const KIND: u64 = 3;
const SIGNING: u64 = 4;
const HPKE: u64 = 5;
const ISSUER: u64 = 6;
const TIMESTAMP: u64 = 7;
const SSH: u64 = 8;
const NAME: u64 = 9;

impl Body {
    /// The body of a certificate for `device` of `account`, issued by the device `issuer` or, if
    /// none, by the master key. Its `hpke` key stands in for one: nothing here uses it.
    pub fn new(
        account: AccountId,
        device: &Identity,
        kind: Kind,
        issuer: Option<DeviceId>,
    ) -> Self {
        Self {
            account,
            device: device.device_id(),
            kind,
            signing: device.public_key(),
            hpke: [0x5e; 32],
            issuer,
            issued: ISSUED,
            name: "device".into(),
        }
    }

    /// The deterministic CBOR of the body.
    pub fn encode(&self) -> Vec<u8> {
        let bytes = |value: &[u8]| Value::Bytes(value.to_vec());
        let mut entries = vec![
            (Value::Uint(ACCOUNT), bytes(&self.account)),
            (Value::Uint(DEVICE), bytes(&self.device)),
            (Value::Uint(KIND), Value::Uint(self.kind as u64)),
            (Value::Uint(SIGNING), bytes(&self.signing)),
            (Value::Uint(HPKE), bytes(&self.hpke)),
        ];
        if let Some(issuer) = &self.issuer {
            entries.push((Value::Uint(ISSUER), bytes(issuer)));
        }
        entries.push((Value::Uint(TIMESTAMP), Value::Uint(self.issued)));
        entries.push((Value::Uint(NAME), Value::Text(self.name.clone())));
        Value::Map(entries).encode()
    }

    fn decode(body: &[u8]) -> Result<Self> {
        let (mut account, mut device, mut kind, mut signing, mut hpke) =
            (None, None, None, None, None);
        let (mut issuer, mut issued, mut name) = (None, None, None);
        for (key, value) in object::fields(body)? {
            match key {
                ACCOUNT => account = Some(object::id(value)?),
                DEVICE => device = Some(object::id(value)?),
                KIND => kind = Some(Kind::decode(object::uint(value)?)?),
                SIGNING => signing = Some(object::key(value)?),
                HPKE => hpke = Some(object::key(value)?),
                ISSUER => issuer = Some(object::id(value)?),
                TIMESTAMP => issued = Some(object::uint(value)?),
                SSH if !matches!(value, Value::Bytes(_)) => {
                    return Err("cert-body: an ssh signature that is not bytes".into());
                }
                NAME => match value {
                    Value::Text(text) if (1..=64).contains(&text.len()) => name = Some(text),
                    _ => return Err("cert-body: a name that is not 1 to 64 bytes of text".into()),
                },
                _ => {} // `ssh`, or a field of a later version
            }
        }
        match (account, device, kind, signing, hpke, issued, name) {
            (
                Some(account),
                Some(device),
                Some(kind),
                Some(signing),
                Some(hpke),
                Some(issued),
                Some(name),
            ) => Ok(Self {
                account,
                device,
                kind,
                signing,
                hpke,
                issuer,
                issued,
                name,
            }),
            _ => Err("cert-body: a required field is missing".into()),
        }
    }
}

/// The encoded `cert` of `body`: `issuer`, the master key or the issuing device's key, signs it,
/// and the device's key `device` signs its `proof`.
pub fn issue(body: &Body, issuer: &Identity, device: &Identity) -> Vec<u8> {
    let encoded = body.encode();
    let signature = issuer.sign(&signed(LABEL, &encoded));
    let proof = device.sign(&signed(PROOF_LABEL, &encoded));
    Value::Array(vec![
        Value::Bytes(encoded),
        Value::Bytes(signature.to_vec()),
        Value::Bytes(proof.to_vec()),
    ])
    .encode()
}

/// A decoded certificate. Decoding checks its form, not its signatures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Certificate {
    pub body: Body,
    encoded: Vec<u8>,
    signature: [u8; 64],
    proof: [u8; 64],
}

impl Certificate {
    pub fn decode(cert: &[u8]) -> Result<Self> {
        let (encoded, signatures) = object::parts(Value::decode(cert)?, 2)?;
        Ok(Self {
            body: Body::decode(&encoded)?,
            encoded,
            signature: signatures[0],
            proof: signatures[1],
        })
    }

    /// The proof check: `device` is the ID derived from `signing`, and `proof` verifies with
    /// `signing`.
    pub fn passes_proof_check(&self) -> bool {
        self.body.device == device_id(&self.body.signing)
            && verifies(
                &self.body.signing,
                &signed(PROOF_LABEL, &self.encoded),
                &self.proof,
            )
    }

    /// Whether `signature` verifies with `key`, the issuer's.
    pub fn signed_by(&self, key: &[u8; 32]) -> bool {
        verifies(key, &signed(LABEL, &self.encoded), &self.signature)
    }

    /// Whether both certificates have the same encoded `body`.
    pub fn same_body(&self, other: &Certificate) -> bool {
        self.encoded == other.encoded
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACCOUNT_ID: AccountId = [0xac; 16];

    fn bytes(value: &[u8]) -> Value {
        Value::Bytes(value.to_vec())
    }

    #[test]
    fn encodes_a_certificate() {
        let (master, device) = (Identity::generate(), Identity::generate());
        let body = Body::new(ACCOUNT_ID, &device, Kind::Client, None);
        let mut want_body = vec![0xa7, 0x01, 0x50];
        want_body.extend_from_slice(&ACCOUNT_ID);
        want_body.extend_from_slice(&[0x02, 0x50]);
        want_body.extend_from_slice(&device.device_id());
        want_body.extend_from_slice(&[0x03, 0x01, 0x04, 0x58, 0x20]);
        want_body.extend_from_slice(&device.public_key());
        want_body.extend_from_slice(&[0x05, 0x58, 0x20]);
        want_body.extend_from_slice(&[0x5e; 32]);
        want_body.extend_from_slice(&[0x07, 0x1a, 0x69, 0x80, 0x0e, 0x80]);
        want_body.extend_from_slice(b"\x09\x66device");
        assert_eq!(body.encode(), want_body);

        let issuer = [9; 16];
        let issued = Body::new(ACCOUNT_ID, &device, Kind::Browser, Some(issuer)).encode();
        assert_eq!(issued[0], 0xa8);
        assert!(
            issued
                .windows(19)
                .any(|window| window == [&[0x06, 0x50][..], &issuer, &[0x07]].concat()),
            "the issuer comes before the timestamp"
        );

        let cert = issue(&body, &master, &device);
        let mut want = vec![0x83, 0x58, want_body.len() as u8];
        want.extend_from_slice(&want_body);
        want.extend_from_slice(&[0x58, 0x40]);
        want.extend_from_slice(&master.sign(&signed(LABEL, &want_body)));
        want.extend_from_slice(&[0x58, 0x40]);
        want.extend_from_slice(&device.sign(&signed(PROOF_LABEL, &want_body)));
        assert_eq!(cert, want);
    }

    #[test]
    fn decodes_what_it_issues() {
        let (master, device) = (Identity::generate(), Identity::generate());
        for (kind, issuer) in [
            (Kind::Client, None),
            (Kind::Browser, Some([1; 16])),
            (Kind::Service, None),
            (Kind::Custodian, None),
            (Kind::Auditor, None),
        ] {
            let body = Body::new(ACCOUNT_ID, &device, kind, issuer);
            let cert = Certificate::decode(&issue(&body, &master, &device)).unwrap();
            assert_eq!(cert.body, body);
            assert!(cert.passes_proof_check(), "{kind:?}");
            assert!(cert.signed_by(&master.public_key()), "{kind:?}");
            assert!(!cert.signed_by(&device.public_key()), "{kind:?}");
        }
    }

    #[test]
    fn the_proof_check_needs_the_derived_id_and_the_devices_proof() {
        let (master, device, other) = (
            Identity::generate(),
            Identity::generate(),
            Identity::generate(),
        );
        let body = Body::new(ACCOUNT_ID, &device, Kind::Client, None);
        let proof_by_other = Certificate::decode(&issue(&body, &master, &other)).unwrap();
        assert!(
            !proof_by_other.passes_proof_check(),
            "a proof by another key"
        );
        assert!(proof_by_other.signed_by(&master.public_key()));

        let mut other_id = body.clone();
        other_id.device = other.device_id();
        let cert = Certificate::decode(&issue(&other_id, &master, &device)).unwrap();
        assert!(!cert.passes_proof_check(), "an ID not derived from signing");

        // The proof covers the body, not the issuer's signature.
        let a = Certificate::decode(&issue(&body, &master, &device)).unwrap();
        let b = Certificate::decode(&issue(&body, &other, &device)).unwrap();
        assert!(a.same_body(&b) && b.passes_proof_check());
        let renamed = Body {
            name: "other".into(),
            ..body.clone()
        };
        let c = Certificate::decode(&issue(&renamed, &master, &device)).unwrap();
        assert!(!a.same_body(&c) && c.passes_proof_check());
    }

    #[test]
    fn reads_ssh_and_fields_of_later_versions() {
        let device = Identity::generate();
        let body = Body::new(ACCOUNT_ID, &device, Kind::Client, None);
        let Value::Map(mut entries) = Value::decode(&body.encode()).unwrap() else {
            unreachable!()
        };
        entries.insert(6, (Value::Uint(SSH), bytes(b"sshsig")));
        entries.push((Value::Uint(10), Value::Text("later".into())));
        entries.push((Value::Nint(0), Value::Simple(21)));
        let encoded = Value::Map(entries).encode();
        let cert = Value::Array(vec![bytes(&encoded), bytes(&[0; 64]), bytes(&[0; 64])]);
        assert_eq!(Certificate::decode(&cert.encode()).unwrap().body, body);
    }

    #[test]
    fn rejects_malformed_certificates() {
        let device = Identity::generate();
        let body = Body::new(ACCOUNT_ID, &device, Kind::Client, None);
        let Value::Map(entries) = Value::decode(&body.encode()).unwrap() else {
            unreachable!()
        };
        let cert = |entries: Vec<(Value, Value)>| {
            Value::Array(vec![
                bytes(&Value::Map(entries).encode()),
                bytes(&[0; 64]),
                bytes(&[0; 64]),
            ])
            .encode()
        };
        let changed = |at: usize, value: Value| {
            let mut entries = entries.clone();
            entries[at].1 = value;
            cert(entries)
        };
        let without = |at: usize| {
            let mut entries = entries.clone();
            entries.remove(at);
            cert(entries)
        };
        let with = |at: usize, key: u64, value: Value| {
            let mut entries = entries.clone();
            entries.insert(at, (Value::Uint(key), value));
            cert(entries)
        };
        let mut cases = vec![
            (
                Value::Array(vec![bytes(&body.encode()), bytes(&[0; 64])]).encode(),
                "no proof".to_string(),
            ),
            (changed(0, bytes(&[1; 15])), "a short account".into()),
            (
                changed(1, Value::Uint(1)),
                "a device that is not bytes".into(),
            ),
            (changed(2, Value::Uint(0)), "device kind 0".into()),
            (changed(2, Value::Uint(6)), "device kind 6".into()),
            (changed(2, bytes(b"1")), "a kind that is not a uint".into()),
            (changed(3, bytes(&[1; 31])), "a short signing key".into()),
            (changed(4, bytes(&[1; 33])), "a long hpke key".into()),
            (
                changed(5, bytes(b"now")),
                "issued that is not a uint".into(),
            ),
            (
                changed(6, Value::Text(String::new())),
                "an empty name".into(),
            ),
            (
                changed(6, Value::Text("n".repeat(65))),
                "a name of 65 bytes".into(),
            ),
            (changed(6, bytes(b"name")), "a name that is not text".into()),
            (with(5, ISSUER, bytes(&[1; 17])), "a long issuer".into()),
            (with(6, SSH, Value::Uint(1)), "ssh that is not bytes".into()),
        ];
        for (at, field) in [
            "account", "device", "kind", "signing", "hpke", "issued", "name",
        ]
        .iter()
        .enumerate()
        {
            cases.push((without(at), format!("no {field}")));
        }
        for (cert, why) in cases {
            assert!(Certificate::decode(&cert).is_err(), "{why}");
        }
        assert!(Certificate::decode(&cert(entries.clone())).is_ok());
        let longest = changed(6, Value::Text("n".repeat(64)));
        assert!(Certificate::decode(&longest).is_ok(), "a name of 64 bytes");
    }
}
