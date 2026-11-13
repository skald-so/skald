//! Signed objects: `docs/protocol/membership.md`, "Signed objects".

use crate::cbor::{self, Cbor, DecodeError, FieldProblem, field_error};

/// The body of a [`Signed`] object.
pub trait SignedBody: Cbor {
    /// The label its signature covers, such as `skald roster`.
    const LABEL: &'static str;
    /// The names of the object, its body and its signature in decoding errors.
    const FIELDS: [&'static str; 3];
}

/// A signed object, `[body: bstr .cbor body, signature: bstr .size 64]`, its signature over
/// `[label, body]`. It keeps its body's bytes as it got them, so checking or storing it never
/// re-encodes them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signed<B> {
    body: B,
    body_bytes: Vec<u8>,
    signature: [u8; 64],
}

impl<B: SignedBody> Signed<B> {
    /// The object of an encoded body and its signature over `[label, body]`.
    ///
    /// # Errors
    /// Why `body` is not an encoded `B`.
    pub fn from_parts(body: Vec<u8>, signature: [u8; 64]) -> Result<Self, DecodeError> {
        let decoded = B::from_cbor(&body)
            .map_err(|error| field_error(B::FIELDS[1], FieldProblem::Embedded(Box::new(error))))?;
        Ok(Self {
            body: decoded,
            body_bytes: body,
            signature,
        })
    }

    pub fn body(&self) -> &B {
        &self.body
    }

    /// The body as the object holds it: the bytes its signature covers.
    pub fn body_bytes(&self) -> &[u8] {
        &self.body_bytes
    }

    /// By the signer over `[label, body]`.
    pub fn signature(&self) -> &[u8; 64] {
        &self.signature
    }
}

impl<B: SignedBody> Cbor for Signed<B> {
    fn to_cbor(&self) -> Vec<u8> {
        cbor::array([cbor::bytes(&self.body_bytes), cbor::bytes(&self.signature)])
    }

    fn from_cbor(bytes: &[u8]) -> Result<Self, DecodeError> {
        let [object, body, signature] = B::FIELDS;
        let item = cbor::parse(bytes)?;
        let [body_item, signature_item] = item.tuple(object)?;
        Self::from_parts(
            body_item.bytes(body)?.to_vec(),
            signature_item.fixed(signature)?,
        )
    }
}
