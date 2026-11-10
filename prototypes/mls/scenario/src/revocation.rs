//! Revocations as `docs/protocol/membership.md`, "Revocation", defines them, and the encoded
//! `[+ revocation]` a Remove or an empty Commit carries in its `authenticated_data` ("Group
//! changes").
//!
//! Whether a revocation counts comes from the certificates and master keys of a [`Roster`]. No
//! scenario has a stolen device, so a master-key revocation voids nothing here.

use crate::{
    DeviceId, Identity, Result,
    cbor::Value,
    certificate::Kind,
    object::{self, signed, verifies},
    roster::Roster,
};

/// A 16-byte account ID.
pub type AccountId = [u8; 16];

/// The signature label of a revocation.
pub const LABEL: &str = "skald revocation";

/// The fields of `revocation-body`.
const ACCOUNT: u64 = 1;
const DEVICE: u64 = 2;
const KEEP: u64 = 3;
const SIGNER: u64 = 4;
const TIMESTAMP: u64 = 5;
const ALSO: u64 = 6;

/// A revocation of `device` that the device `signer` signs: the encoded
/// `signed<revocation-body>`.
pub fn revoke(
    signer: &Identity,
    account: &AccountId,
    device: &DeviceId,
    timestamp: u64,
) -> Vec<u8> {
    let body = Value::Map(vec![
        (Value::Uint(ACCOUNT), Value::Bytes(account.to_vec())),
        (Value::Uint(DEVICE), Value::Bytes(device.to_vec())),
        (
            Value::Uint(SIGNER),
            Value::Bytes(signer.device_id().to_vec()),
        ),
        (Value::Uint(TIMESTAMP), Value::Uint(timestamp)),
    ])
    .encode();
    object::sign(signer, LABEL, body)
}

/// The `authenticated_data` of a Commit with the revocations: the encoded `[+ revocation]`.
pub fn list(revocations: &[Vec<u8>]) -> Result<Vec<u8>> {
    if revocations.is_empty() {
        return Err("a revocation list holds at least one".into());
    }
    let items = revocations
        .iter()
        .map(|revocation| Value::decode(revocation))
        .collect::<Result<_>>()?;
    Ok(Value::Array(items).encode())
}

/// What a member device reads from a Commit's `authenticated_data` before it merges the Commit:
/// the devices that the counting revocations revoke, each one's `device` and `also` in turn.
///
/// Fails unless `authenticated_data` is an encoded `[+ revocation]`; one that does not count is
/// ignored.
pub fn revoked(authenticated_data: &[u8], roster: &Roster) -> Result<Vec<DeviceId>> {
    let Value::Array(revocations) = Value::decode(authenticated_data)? else {
        return Err("authenticated_data: not an array of revocations".into());
    };
    if revocations.is_empty() {
        return Err("authenticated_data: no revocations".into());
    }
    let mut revoked = Vec::new();
    for revocation in revocations {
        if let Some(counts) = counting(revocation, roster)? {
            revoked.push(counts.device);
            revoked.extend(counts.also);
        }
    }
    Ok(revoked)
}

/// A decoded `revocation-body`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Revocation {
    pub account: AccountId,
    pub device: DeviceId,
    pub keep: Vec<DeviceId>,
    /// Absent: the master key.
    pub signer: Option<DeviceId>,
    pub also: Vec<DeviceId>,
}

impl Revocation {
    /// Whether it names `device` in `device` or `also`.
    pub fn names(&self, device: &DeviceId) -> bool {
        self.device == *device || self.also.contains(device)
    }
}

/// The revocation if it counts: its signature verifies with the master key `roster` pins for its
/// `account`, or it has no `also` and its signature verifies with the certificate of `signer`, a
/// `client` or `browser` device of `account` whose certificate passes checks 1 and 2 of "Device
/// checks". Fails unless `revocation` is a `signed<revocation-body>`.
pub fn counting(revocation: Value, roster: &Roster) -> Result<Option<Revocation>> {
    let (body, signatures) = object::parts(revocation, 1)?;
    let revocation = decode(&body)?;
    let key = match revocation.signer {
        None => roster.master(&revocation.account),
        Some(signer) if revocation.also.is_empty() => roster.chain(&signer).and_then(|chain| {
            let cert = &chain[0];
            (cert.account == revocation.account
                && matches!(cert.kind, Kind::Client | Kind::Browser))
            .then_some(cert.signing)
        }),
        Some(_) => None,
    };
    let counts = key.is_some_and(|key| verifies(&key, &signed(LABEL, &body), &signatures[0]));
    Ok(counts.then_some(revocation))
}

fn decode(body: &[u8]) -> Result<Revocation> {
    let (mut account, mut device, mut keep, mut signer, mut timestamp, mut also) =
        (None, None, Vec::new(), None, None, Vec::new());
    for (key, value) in object::fields(body)? {
        match key {
            ACCOUNT => account = Some(object::id(value)?),
            DEVICE => device = Some(object::id(value)?),
            KEEP => keep = object::ids(value)?,
            SIGNER => signer = Some(object::id(value)?),
            TIMESTAMP => timestamp = Some(object::uint(value)?),
            ALSO => also = object::ids(value)?,
            _ => {} // a field of a later version
        }
    }
    match (account, device, timestamp) {
        (Some(account), Some(device), Some(_)) => Ok(Revocation {
            account,
            device,
            keep,
            signer,
            also,
        }),
        _ => Err("revocation-body: a required field is missing".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::certificate::{Body, issue};

    const ACCOUNT_ID: AccountId = [0xac; 16];
    const OTHER_ID: AccountId = [0xad; 16];

    fn body(entries: Vec<(Value, Value)>) -> Vec<u8> {
        Value::Map(entries).encode()
    }

    /// A revocation of `body` signed by `signer`.
    fn signed_by(signer: &Identity, body: Vec<u8>) -> Vec<u8> {
        object::sign(signer, LABEL, body)
    }

    fn bytes(value: &[u8]) -> Value {
        Value::Bytes(value.to_vec())
    }

    fn fields(signer: &Identity, device: &DeviceId) -> Vec<(Value, Value)> {
        vec![
            (Value::Uint(ACCOUNT), bytes(&ACCOUNT_ID)),
            (Value::Uint(DEVICE), bytes(device)),
            (Value::Uint(SIGNER), bytes(&signer.device_id())),
            (Value::Uint(TIMESTAMP), Value::Uint(1_700_000_000)),
        ]
    }

    /// The account's master key, its roster and a device of `kind` certified by the master key.
    struct Fixture {
        master: Identity,
        roster: Roster,
    }

    impl Fixture {
        fn new() -> Self {
            let master = Identity::generate();
            let mut roster = Roster::default();
            roster.pin(ACCOUNT_ID, master.public_key());
            Self { master, roster }
        }

        fn device(&mut self, account: AccountId, kind: Kind) -> Identity {
            let device = Identity::generate();
            let body = Body::new(account, &device, kind, None);
            self.roster
                .add_certificate(&issue(&body, &self.master, &device))
                .unwrap();
            device
        }
    }

    #[test]
    fn encodes_a_signed_revocation_body() {
        let signer = Identity::generate();
        let device = [0xde; 16];
        let revocation = revoke(&signer, &ACCOUNT_ID, &device, 1_700_000_000);

        let mut want_body = vec![0xa4, 0x01, 0x50];
        want_body.extend_from_slice(&ACCOUNT_ID);
        want_body.extend_from_slice(&[0x02, 0x50]);
        want_body.extend_from_slice(&device);
        want_body.extend_from_slice(&[0x04, 0x50]);
        want_body.extend_from_slice(&signer.device_id());
        want_body.extend_from_slice(&[0x05, 0x1a, 0x65, 0x53, 0xf1, 0x00]);
        let mut want_signed = b"\x82\x70skald revocation\x58\x3d".to_vec();
        want_signed.extend_from_slice(&want_body);

        let mut want = vec![0x82, 0x58, 0x3d];
        want.extend_from_slice(&want_body);
        want.extend_from_slice(&[0x58, 0x40]);
        want.extend_from_slice(&signer.sign(&want_signed));
        assert_eq!(revocation, want);
    }

    #[test]
    fn counts_revocations_by_client_and_browser_devices_and_the_master_key() {
        let mut fixture = Fixture::new();
        let client = fixture.device(ACCOUNT_ID, Kind::Client);
        let browser = fixture.device(ACCOUNT_ID, Kind::Browser);
        let mut by_master = fields(&client, &[3; 16]);
        by_master.remove(2);
        by_master.push((Value::Uint(ALSO), Value::Array(vec![bytes(&[4; 16])])));
        let ad = list(&[
            revoke(&client, &ACCOUNT_ID, &[1; 16], 1),
            revoke(&browser, &ACCOUNT_ID, &[2; 16], 2),
            signed_by(&fixture.master, body(by_master)),
        ])
        .unwrap();
        assert_eq!(ad[0], 0x83);
        assert_eq!(
            revoked(&ad, &fixture.roster).unwrap(),
            [[1; 16], [2; 16], [3; 16], [4; 16]]
        );
    }

    #[test]
    fn ignores_revocations_that_do_not_count() {
        let mut fixture = Fixture::new();
        let signer = fixture.device(ACCOUNT_ID, Kind::Client);
        let service = fixture.device(ACCOUNT_ID, Kind::Service);
        let custodian = fixture.device(ACCOUNT_ID, Kind::Custodian);
        let foreign = fixture.device(OTHER_ID, Kind::Client);
        fixture.roster.pin(OTHER_ID, fixture.master.public_key());
        let outsider = Identity::generate();
        let device = [7; 16];
        let counts = revoke(&signer, &ACCOUNT_ID, &device, 1);

        let mut forged = Value::decode(&counts).unwrap();
        let Value::Array(parts) = &mut forged else {
            unreachable!()
        };
        parts[1] = bytes(&[0; 64]);
        let mut master_key = fields(&signer, &device);
        master_key.remove(2);
        let mut with_also = fields(&signer, &device);
        with_also.push((Value::Uint(ALSO), Value::Array(vec![bytes(&[8; 16])])));
        // The same body signed by another key than its signer's.
        let mut impostor = Value::decode(&counts).unwrap();
        let Value::Array(parts) = &mut impostor else {
            unreachable!()
        };
        let Value::Bytes(counted) = parts[0].clone() else {
            unreachable!()
        };
        parts[1] = bytes(&outsider.sign(&signed(LABEL, &counted)));

        for (revocation, why) in [
            (revoke(&signer, &[0; 16], &device, 1), "another account"),
            (
                revoke(&foreign, &ACCOUNT_ID, &device, 1),
                "a signer of another account",
            ),
            (
                revoke(&outsider, &ACCOUNT_ID, &device, 1),
                "a signer without a certificate",
            ),
            (
                revoke(&service, &ACCOUNT_ID, &device, 1),
                "a service signer",
            ),
            (
                revoke(&custodian, &ACCOUNT_ID, &device, 1),
                "a custodian signer",
            ),
            (forged.encode(), "a signature that does not verify"),
            (impostor.encode(), "a signature by another key"),
            (
                signed_by(&signer, body(master_key)),
                "no signer, but not signed by the master key",
            ),
            (
                signed_by(&signer, body(with_also)),
                "`also` without the master key",
            ),
        ] {
            let ad = list(&[revocation, counts.clone()]).unwrap();
            assert_eq!(revoked(&ad, &fixture.roster).unwrap(), [device], "{why}");
        }
        let unpinned = Roster::default();
        let ad = list(&[counts]).unwrap();
        assert!(
            revoked(&ad, &unpinned).unwrap().is_empty(),
            "no certificates"
        );
    }

    #[test]
    fn reads_keep_and_fields_of_later_versions() {
        let mut fixture = Fixture::new();
        let signer = fixture.device(ACCOUNT_ID, Kind::Client);
        let device = [7; 16];
        let mut entries = fields(&signer, &device);
        entries.insert(2, (Value::Uint(KEEP), Value::Array(vec![bytes(&[9; 16])])));
        entries.push((Value::Uint(7), Value::Text("later".into())));
        entries.push((Value::Nint(0), Value::Simple(21)));
        let revocation = signed_by(&signer, body(entries));
        assert_eq!(
            counting(Value::decode(&revocation).unwrap(), &fixture.roster).unwrap(),
            Some(Revocation {
                account: ACCOUNT_ID,
                device,
                keep: vec![[9; 16]],
                signer: Some(signer.device_id()),
                also: Vec::new(),
            })
        );
        let ad = list(&[revocation]).unwrap();
        assert_eq!(revoked(&ad, &fixture.roster).unwrap(), [device]);
    }

    #[test]
    fn names_its_device_and_also() {
        let revocation = Revocation {
            account: ACCOUNT_ID,
            device: [1; 16],
            keep: vec![[2; 16]],
            signer: None,
            also: vec![[3; 16]],
        };
        assert!(revocation.names(&[1; 16]));
        assert!(revocation.names(&[3; 16]));
        assert!(!revocation.names(&[2; 16]));
    }

    #[test]
    fn rejects_authenticated_data_that_is_not_a_revocation_list() {
        let mut fixture = Fixture::new();
        let signer = fixture.device(ACCOUNT_ID, Kind::Client);
        let device = [7; 16];
        let counts = revoke(&signer, &ACCOUNT_ID, &device, 1);
        let malformed = |change: fn(&mut Vec<(Value, Value)>)| {
            let mut entries = fields(&signer, &device);
            change(&mut entries);
            Value::Array(vec![
                Value::decode(&signed_by(&signer, body(entries))).unwrap(),
            ])
            .encode()
        };

        let Value::Array(parts) = Value::decode(&counts).unwrap() else {
            unreachable!()
        };
        let counted_body = parts[0].clone();

        assert!(list(&[]).is_err());
        assert!(list(&[vec![0x82]]).is_err());
        for (ad, why) in [
            (Vec::new(), "nothing"),
            (Value::Array(Vec::new()).encode(), "an empty list"),
            (counts.clone(), "a revocation, not a list"),
            (Value::Uint(1).encode(), "not an array"),
            (
                Value::Array(vec![Value::Uint(1)]).encode(),
                "a revocation that is not an array",
            ),
            (
                Value::Array(vec![Value::Array(vec![bytes(b"body")])]).encode(),
                "a revocation without a signature",
            ),
            (
                Value::Array(vec![Value::Array(vec![counted_body, bytes(&[0; 63])])]).encode(),
                "a short signature",
            ),
            (
                Value::Array(vec![Value::Array(vec![bytes(&[0x01]), bytes(&[0; 64])])]).encode(),
                "a body that is not a map",
            ),
            (malformed(|e| drop(e.remove(0))), "no account"),
            (malformed(|e| drop(e.remove(1))), "no device"),
            (malformed(|e| drop(e.remove(3))), "no timestamp"),
            (malformed(|e| e[1].1 = bytes(&[1; 15])), "a short device ID"),
            (
                malformed(|e| e[0].1 = Value::Uint(1)),
                "an account that is not bytes",
            ),
            (
                malformed(|e| e[2].1 = Value::Uint(1)),
                "a signer that is not bytes",
            ),
            (
                malformed(|e| e[3].1 = bytes(b"now")),
                "a timestamp that is not a uint",
            ),
            (
                malformed(|e| e.insert(2, (Value::Uint(KEEP), Value::Array(Vec::new())))),
                "an empty keep",
            ),
            (
                malformed(|e| e.insert(2, (Value::Uint(KEEP), Value::Array(vec![Value::Uint(1)])))),
                "a keep of non-ids",
            ),
            (
                malformed(|e| e.push((Value::Uint(ALSO), bytes(&[1; 16])))),
                "an also that is not a list",
            ),
            (
                malformed(|e| e.push((Value::Text("x".into()), Value::Uint(1)))),
                "a key that is not an integer",
            ),
        ] {
            assert!(revoked(&ad, &fixture.roster).is_err(), "{why}");
        }
    }
}
