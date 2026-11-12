//! Device certificates: `docs/protocol/membership.md`, "Device certificate" and "Signed objects".

use crate::cbor::{self, Codec, DecodeError, FieldProblem, Item, MapOut, Unknown, field_error};
use crate::id::{AccountId, DeviceId};

/// The label of a certificate's `signature`.
pub const CERT_LABEL: &str = "skald cert";
/// The label of a certificate's `proof`.
pub const PROOF_LABEL: &str = "skald proof";

/// The kind of a device, fixed in its certificate: `docs/protocol/README.md`, "Identifiers".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeviceKind {
    /// A `skald` installation.
    Client = 1,
    /// A linked browser.
    Browser = 2,
    /// A service agent.
    Service = 3,
}

/// What a [`Cert`] signs: a device's keys bound to its account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertBody {
    pub account: AccountId,
    /// Derived from `signing`.
    pub device: DeviceId,
    pub kind: DeviceKind,
    /// The Ed25519 public key.
    pub signing: [u8; 32],
    /// The X25519 public key.
    pub hpke: [u8; 32],
    /// The device that signed it; `None`, the master key.
    pub issuer: Option<DeviceId>,
    /// Seconds since the Unix epoch, for display.
    pub issued: u64,
    /// An SSH signature of the device key.
    pub ssh: Option<Vec<u8>>,
    /// Set on the device, 1 to 64 bytes, for display.
    pub name: String,
    pub unknown: Unknown,
}

/// A device certificate. It keeps its body's bytes as it got them, so checking or storing it
/// never re-encodes them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cert {
    body: CertBody,
    body_bytes: Vec<u8>,
    signature: [u8; 64],
    proof: [u8; 64],
}

/// The longest `name`, in bytes.
const NAME_MAX: usize = 64;

impl Cert {
    /// The certificate of an encoded body, its `signature` by the issuer over
    /// `["skald cert", body]` and its `proof` by the device over `["skald proof", body]`.
    ///
    /// # Errors
    /// Why `body` is not an encoded [`CertBody`].
    pub fn from_parts(
        body: Vec<u8>,
        signature: [u8; 64],
        proof: [u8; 64],
    ) -> Result<Self, DecodeError> {
        let decoded = cbor::parse(&body)
            .and_then(|item| CertBody::decode(&item))
            .map_err(|error| field_error("cert.body", FieldProblem::Embedded(Box::new(error))))?;
        Ok(Self {
            body: decoded,
            body_bytes: body,
            signature,
            proof,
        })
    }

    pub fn body(&self) -> &CertBody {
        &self.body
    }

    /// The body as the certificate holds it: the bytes both signatures cover.
    pub fn body_bytes(&self) -> &[u8] {
        &self.body_bytes
    }

    /// By the issuer, or the master key if `body().issuer` is `None`, over `["skald cert", body]`.
    pub fn signature(&self) -> &[u8; 64] {
        &self.signature
    }

    /// By the device's `signing` key over `["skald proof", body]`.
    pub fn proof(&self) -> &[u8; 64] {
        &self.proof
    }
}

impl Codec for Cert {
    fn encode(&self) -> Vec<u8> {
        cbor::array([
            cbor::bytes(&self.body_bytes),
            cbor::bytes(&self.signature),
            cbor::bytes(&self.proof),
        ])
    }

    fn decode(item: &Item<'_>) -> Result<Self, DecodeError> {
        let [body, signature, proof] = item.tuple("cert")?;
        let body = body.bytes("cert.body")?.to_vec();
        let signature = signature.fixed("cert.signature")?;
        let proof = proof.fixed("cert.proof")?;
        Self::from_parts(body, signature, proof)
    }
}

impl Codec for CertBody {
    fn encode(&self) -> Vec<u8> {
        MapOut::new(&self.unknown)
            .put(1, cbor::id(&self.account))
            .put(2, cbor::id(&self.device))
            .put(3, cbor::uint(self.kind as u64))
            .put(4, cbor::bytes(&self.signing))
            .put(5, cbor::bytes(&self.hpke))
            .put_some(6, self.issuer.as_ref().map(cbor::id))
            .put(7, cbor::uint(self.issued))
            .put_some(8, self.ssh.as_deref().map(cbor::bytes))
            .put(9, cbor::text(&self.name))
            .finish()
    }

    fn decode(item: &Item<'_>) -> Result<Self, DecodeError> {
        let mut fields = item.map("cert-body")?;
        Ok(Self {
            account: fields
                .required(1, "cert-body.account")?
                .id("cert-body.account")?,
            device: fields
                .required(2, "cert-body.device")?
                .id("cert-body.device")?,
            kind: DeviceKind::decode(fields.required(3, "cert-body.kind")?)?,
            signing: fields
                .required(4, "cert-body.signing")?
                .fixed("cert-body.signing")?,
            hpke: fields
                .required(5, "cert-body.hpke")?
                .fixed("cert-body.hpke")?,
            issuer: fields
                .take(6)
                .map(|item| item.id("cert-body.issuer"))
                .transpose()?,
            issued: fields
                .required(7, "cert-body.issued")?
                .uint("cert-body.issued")?,
            ssh: fields
                .take(8)
                .map(|item| item.bytes("cert-body.ssh").map(<[u8]>::to_vec))
                .transpose()?,
            name: name(fields.required(9, "cert-body.name")?)?,
            unknown: fields.unknown(),
        })
    }
}

/// `name`: `tstr .size (1..64)`.
fn name(item: &Item<'_>) -> Result<String, DecodeError> {
    let name = item.text("cert-body.name")?;
    if (1..=NAME_MAX).contains(&name.len()) {
        Ok(name.to_owned())
    } else {
        Err(field_error(
            "cert-body.name",
            FieldProblem::Size {
                min: 1,
                max: NAME_MAX,
                actual: name.len(),
            },
        ))
    }
}

impl DeviceKind {
    fn decode(item: &Item<'_>) -> Result<Self, DecodeError> {
        match item.uint("cert-body.kind")? {
            1 => Ok(Self::Client),
            2 => Ok(Self::Browser),
            3 => Ok(Self::Service),
            // 4 and 5 are reserved.
            other => Err(field_error("cert-body.kind", FieldProblem::Value(other))),
        }
    }
}

cbor::cbor!(Cert, CertBody);
