//! Revocations as `docs/protocol/membership.md`, "Revocation", defines them, and the encoded
//! `[+ revocation]` a Remove or an empty Commit carries in its `authenticated_data` ("Group
//! changes").
//!
//! The prototypes have no certificates yet: a member leaf of the group stands in for the
//! certificate of a revocation's `signer`, a `client` device of the one account all devices share.

use ed25519_dalek::{Signature, VerifyingKey};

use crate::{DeviceId, Identity, Leaf, Result, cbor::Value};

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
    let signature = signer.sign(&signed(&body));
    Value::Array(vec![Value::Bytes(body), Value::Bytes(signature.to_vec())]).encode()
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
/// the devices that the counting revocations revoke, in their order. `members` are the leaves of
/// the group's current epoch.
///
/// Fails unless `authenticated_data` is an encoded `[+ revocation]`. A revocation counts if its
/// `account` is `account`, it has no `also`, which needs the master key, and its signature verifies
/// with the key of its `signer`'s leaf in `members`; one that does not count is ignored.
pub fn revoked(
    authenticated_data: &[u8],
    account: &AccountId,
    members: &[Leaf],
) -> Result<Vec<DeviceId>> {
    let Value::Array(revocations) = Value::decode(authenticated_data)? else {
        return Err("authenticated_data: not an array of revocations".into());
    };
    if revocations.is_empty() {
        return Err("authenticated_data: no revocations".into());
    }
    let mut revoked = Vec::new();
    for revocation in revocations {
        let (body, signature) = signed_body(revocation)?;
        let fields = Fields::decode(&body)?;
        if fields.account != *account || fields.also {
            continue;
        }
        let Some(signer) = fields.signer else {
            continue;
        };
        let Some(leaf) = members.iter().find(|leaf| leaf.identity == signer) else {
            continue;
        };
        if verifies(&leaf.signature_key, &signed(&body), &signature) {
            revoked.push(fields.device);
        }
    }
    Ok(revoked)
}

/// What a revocation's signature covers: the encoded `[label, body]`.
fn signed(body: &[u8]) -> Vec<u8> {
    Value::Array(vec![Value::Text(LABEL.into()), Value::Bytes(body.to_vec())]).encode()
}

fn signed_body(revocation: Value) -> Result<(Vec<u8>, Vec<u8>)> {
    let Value::Array(parts) = revocation else {
        return Err("revocation: not an array".into());
    };
    match <[Value; 2]>::try_from(parts) {
        Ok([Value::Bytes(body), Value::Bytes(signature)]) if signature.len() == 64 => {
            Ok((body, signature))
        }
        _ => Err("revocation: not [body, signature]".into()),
    }
}

fn verifies(key: &[u8], message: &[u8], signature: &[u8]) -> bool {
    let (Ok(key), Ok(signature)) = (<[u8; 32]>::try_from(key), Signature::from_slice(signature))
    else {
        return false;
    };
    VerifyingKey::from_bytes(&key).is_ok_and(|key| key.verify_strict(message, &signature).is_ok())
}

/// The fields of a `revocation-body` that decide whether it counts here.
struct Fields {
    account: AccountId,
    device: DeviceId,
    signer: Option<DeviceId>,
    also: bool,
}

impl Fields {
    fn decode(body: &[u8]) -> Result<Self> {
        let Value::Map(entries) = Value::decode(body)? else {
            return Err("revocation-body: not a map".into());
        };
        let (mut account, mut device, mut signer, mut timestamp, mut also) =
            (None, None, None, None, false);
        for (key, value) in entries {
            match (key, value) {
                (Value::Uint(ACCOUNT), value) => account = Some(id(value)?),
                (Value::Uint(DEVICE), value) => device = Some(id(value)?),
                (Value::Uint(KEEP), value) => ids(value)?,
                (Value::Uint(SIGNER), value) => signer = Some(id(value)?),
                (Value::Uint(TIMESTAMP), value) => timestamp = Some(uint(value)?),
                (Value::Uint(ALSO), value) => {
                    ids(value)?;
                    also = true;
                }
                (Value::Uint(_) | Value::Nint(_), _) => {} // a field of a later version
                _ => return Err("revocation-body: a key that is not an integer".into()),
            }
        }
        match (account, device, timestamp) {
            (Some(account), Some(device), Some(_)) => Ok(Self {
                account,
                device,
                signer,
                also,
            }),
            _ => Err("revocation-body: a required field is missing".into()),
        }
    }
}

/// A `uint`, such as a `timestamp`.
fn uint(value: Value) -> Result<u64> {
    match value {
        Value::Uint(n) => Ok(n),
        _ => Err("not an unsigned integer".into()),
    }
}

/// An `id`: a 16-byte string.
fn id(value: Value) -> Result<[u8; 16]> {
    match value {
        Value::Bytes(bytes) => Ok(bytes.try_into().map_err(|_| "an id of a wrong size")?),
        _ => Err("an id that is not a byte string".into()),
    }
}

/// A `[+ id]`.
fn ids(value: Value) -> Result<()> {
    match value {
        Value::Array(items) if !items.is_empty() => items.into_iter().try_for_each(|item| {
            id(item)?;
            Ok(())
        }),
        _ => Err("not a list of ids".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenarios::leaf;

    const ACCOUNT_ID: AccountId = [0xac; 16];

    fn body(entries: Vec<(Value, Value)>) -> Vec<u8> {
        Value::Map(entries).encode()
    }

    /// A revocation of `body` signed by `signer`.
    fn signed_by(signer: &Identity, body: Vec<u8>) -> Vec<u8> {
        let signature = signer.sign(&signed(&body));
        Value::Array(vec![Value::Bytes(body), Value::Bytes(signature.to_vec())]).encode()
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
    fn counts_revocations_signed_by_a_member() {
        let (signer, other) = (Identity::generate(), Identity::generate());
        let members = [leaf(&other), leaf(&signer)];
        let ad = list(&[
            revoke(&signer, &ACCOUNT_ID, &[1; 16], 1),
            revoke(&signer, &ACCOUNT_ID, &[2; 16], 2),
        ])
        .unwrap();
        assert_eq!(ad[0], 0x82);
        assert_eq!(
            revoked(&ad, &ACCOUNT_ID, &members).unwrap(),
            [[1; 16], [2; 16]]
        );
    }

    #[test]
    fn ignores_revocations_that_do_not_count() {
        let (signer, outsider) = (Identity::generate(), Identity::generate());
        let members = [leaf(&signer)];
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
        parts[1] = bytes(&outsider.sign(&signed(&counted)));

        for (revocation, why) in [
            (revoke(&signer, &[0; 16], &device, 1), "another account"),
            (
                revoke(&outsider, &ACCOUNT_ID, &device, 1),
                "a signer outside the group",
            ),
            (forged.encode(), "a signature that does not verify"),
            (impostor.encode(), "a signature by another key"),
            (
                signed_by(&signer, body(master_key)),
                "no signer: the master key",
            ),
            (
                signed_by(&signer, body(with_also)),
                "`also` without the master key",
            ),
        ] {
            let ad = list(&[revocation, counts.clone()]).unwrap();
            assert_eq!(
                revoked(&ad, &ACCOUNT_ID, &members).unwrap(),
                [device],
                "{why}"
            );
        }
    }

    #[test]
    fn reads_keep_and_fields_of_later_versions() {
        let signer = Identity::generate();
        let device = [7; 16];
        let mut entries = fields(&signer, &device);
        entries.insert(2, (Value::Uint(KEEP), Value::Array(vec![bytes(&[9; 16])])));
        entries.push((Value::Uint(7), Value::Text("later".into())));
        entries.push((Value::Nint(0), Value::Simple(21)));
        let ad = list(&[signed_by(&signer, body(entries))]).unwrap();
        assert_eq!(
            revoked(&ad, &ACCOUNT_ID, &[leaf(&signer)]).unwrap(),
            [device]
        );
    }

    #[test]
    fn rejects_authenticated_data_that_is_not_a_revocation_list() {
        let signer = Identity::generate();
        let members = [leaf(&signer)];
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
            assert!(revoked(&ad, &ACCOUNT_ID, &members).is_err(), "{why}");
        }
    }
}
