//! Signed objects and their fields, as `docs/protocol/membership.md`, "Signed objects", defines
//! them.

use ed25519_dalek::{Signature, VerifyingKey};

use crate::{Identity, Result, cbor::Value};

/// What a signature with `label` over `body` covers: the encoded `[label, body]`.
pub fn signed(label: &str, body: &[u8]) -> Vec<u8> {
    Value::Array(vec![Value::Text(label.into()), Value::Bytes(body.to_vec())]).encode()
}

/// The encoded `signed<body>` that `signer` signs with `label`.
pub fn sign(signer: &Identity, label: &str, body: Vec<u8>) -> Vec<u8> {
    let signature = signer.sign(&signed(label, &body));
    Value::Array(vec![Value::Bytes(body), Value::Bytes(signature.to_vec())]).encode()
}

/// Whether `signature` is an Ed25519 signature of `message` by `key`.
pub fn verifies(key: &[u8], message: &[u8], signature: &[u8]) -> bool {
    let (Ok(key), Ok(signature)) = (<[u8; 32]>::try_from(key), Signature::from_slice(signature))
    else {
        return false;
    };
    VerifyingKey::from_bytes(&key).is_ok_and(|key| key.verify_strict(message, &signature).is_ok())
}

/// The `body` and the 64-byte signatures of an object of the form `[body, signature, ...]` with
/// `signatures` of them.
pub fn parts(object: Value, signatures: usize) -> Result<(Vec<u8>, Vec<[u8; 64]>)> {
    let Value::Array(parts) = object else {
        return Err("signed object: not an array".into());
    };
    if parts.len() != 1 + signatures {
        return Err("signed object: not [body, signatures]".into());
    }
    let mut parts = parts.into_iter();
    let Some(Value::Bytes(body)) = parts.next() else {
        return Err("signed object: a body that is not a byte string".into());
    };
    let signatures = parts
        .map(|part| match part {
            Value::Bytes(signature) => signature
                .try_into()
                .map_err(|_| "signed object: a signature of a wrong size".into()),
            _ => Err("signed object: a signature that is not a byte string".into()),
        })
        .collect::<Result<_>>()?;
    Ok((body, signatures))
}

/// A `uint`, such as a `timestamp`.
pub fn uint(value: Value) -> Result<u64> {
    match value {
        Value::Uint(n) => Ok(n),
        _ => Err("not an unsigned integer".into()),
    }
}

/// An `id`: a 16-byte string.
pub fn id(value: Value) -> Result<[u8; 16]> {
    match value {
        Value::Bytes(bytes) => Ok(bytes.try_into().map_err(|_| "an id of a wrong size")?),
        _ => Err("an id that is not a byte string".into()),
    }
}

/// A `[+ id]`.
pub fn ids(value: Value) -> Result<Vec<[u8; 16]>> {
    match value {
        Value::Array(items) if !items.is_empty() => items.into_iter().map(id).collect(),
        _ => Err("not a list of ids".into()),
    }
}

/// A `bstr .size 32`, such as a public key.
pub fn key(value: Value) -> Result<[u8; 32]> {
    match value {
        Value::Bytes(bytes) => Ok(bytes.try_into().map_err(|_| "a key of a wrong size")?),
        _ => Err("a key that is not a byte string".into()),
    }
}

/// The entries of a map with integer keys: `* int => any` admits fields of later versions.
pub fn fields(body: &[u8]) -> Result<Vec<(u64, Value)>> {
    let Value::Map(entries) = Value::decode(body)? else {
        return Err("body: not a map".into());
    };
    let mut fields = Vec::new();
    for (key, value) in entries {
        match key {
            Value::Uint(key) => fields.push((key, value)),
            Value::Nint(_) => {} // a field of a later version
            _ => return Err("body: a key that is not an integer".into()),
        }
    }
    Ok(fields)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(value: &[u8]) -> Value {
        Value::Bytes(value.to_vec())
    }

    #[test]
    fn signs_the_label_and_the_body() {
        let signer = Identity::generate();
        let object = sign(&signer, "skald test", vec![0xa0]);
        assert_eq!(signed("skald test", &[0xa0]), b"\x82\x6askald test\x41\xa0");

        let (body, signatures) = parts(Value::decode(&object).unwrap(), 1).unwrap();
        assert_eq!(body, [0xa0]);
        let message = signed("skald test", &body);
        assert!(verifies(&signer.public_key(), &message, &signatures[0]));
        assert!(!verifies(
            &signer.public_key(),
            &signed("skald other", &body),
            &signatures[0]
        ));
        assert!(!verifies(
            &Identity::generate().public_key(),
            &message,
            &signatures[0]
        ));
        assert!(!verifies(
            &signer.public_key()[..31],
            &message,
            &signatures[0]
        ));
        assert!(!verifies(
            &signer.public_key(),
            &message,
            &signatures[0][..63]
        ));
        // Not a point on the curve.
        assert!(!verifies(&[0xff; 32], &message, &signatures[0]));
    }

    #[test]
    fn splits_a_body_and_its_signatures() {
        let object = Value::Array(vec![bytes(b"body"), bytes(&[1; 64]), bytes(&[2; 64])]);
        assert_eq!(
            parts(object.clone(), 2).unwrap(),
            (b"body".to_vec(), vec![[1; 64], [2; 64]])
        );
        assert!(parts(object, 1).is_err(), "more signatures than expected");
        for (object, why) in [
            (bytes(b"body"), "not an array"),
            (Value::Array(vec![bytes(b"body")]), "no signature"),
            (
                Value::Array(vec![Value::Uint(1), bytes(&[1; 64])]),
                "a body that is not bytes",
            ),
            (
                Value::Array(vec![bytes(b"body"), bytes(&[1; 63])]),
                "a short signature",
            ),
            (
                Value::Array(vec![bytes(b"body"), Value::Uint(1)]),
                "a signature that is not bytes",
            ),
        ] {
            assert!(parts(object, 1).is_err(), "{why}");
        }
    }

    #[test]
    fn reads_fields() {
        assert_eq!(uint(Value::Uint(7)).unwrap(), 7);
        assert!(uint(bytes(b"7")).is_err());
        assert_eq!(id(bytes(&[3; 16])).unwrap(), [3; 16]);
        assert!(id(bytes(&[3; 15])).is_err());
        assert!(id(Value::Uint(3)).is_err());
        assert_eq!(
            ids(Value::Array(vec![bytes(&[1; 16]), bytes(&[2; 16])])).unwrap(),
            [[1; 16], [2; 16]]
        );
        assert!(ids(Value::Array(Vec::new())).is_err());
        assert!(ids(Value::Array(vec![Value::Uint(1)])).is_err());
        assert!(ids(bytes(&[1; 16])).is_err());
        assert_eq!(key(bytes(&[4; 32])).unwrap(), [4; 32]);
        assert!(key(bytes(&[4; 33])).is_err());
        assert!(key(Value::Uint(4)).is_err());
    }

    #[test]
    fn reads_integer_keys_and_skips_negative_ones() {
        let body = Value::Map(vec![
            (Value::Uint(1), Value::Uint(10)),
            (Value::Uint(9), Value::Text("x".into())),
            (Value::Nint(0), Value::Simple(21)),
        ])
        .encode();
        assert_eq!(
            fields(&body).unwrap(),
            [(1, Value::Uint(10)), (9, Value::Text("x".into()))]
        );
        assert!(fields(&Value::Array(Vec::new()).encode()).is_err());
        let text_key = Value::Map(vec![(Value::Text("a".into()), Value::Uint(1))]).encode();
        assert!(fields(&text_key).is_err());
    }
}
