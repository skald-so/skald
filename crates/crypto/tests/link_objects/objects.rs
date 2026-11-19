//! The tests' link objects, requests and grants: built from their spec fields, devices by the seed
//! of their signing key, encoded by [`crate::cbor`] and signed with ed25519-dalek over
//! `[label, body]`, apart from the crates under test.

use skald_proto::{DeviceKind, LinkBody, SpaceId, Unknown};

use crate::cbor::{Value, bytes, text};
use crate::fields::signed_cbor;
use crate::fixtures::{DESKTOP, LAPTOP, device, public, sign};

/// The label of a link object's signature.
pub const LINK: &str = "skald link";
/// The label of a certificate's `signature`.
pub const CERT: &str = "skald cert";

/// The spaces the issuing device accepted revisions of: two team spaces and the account's personal
/// space.
pub const INITECH: SpaceId = SpaceId::from_bytes([0x6a; 16]);
pub const UMBRELLA: SpaceId = SpaceId::from_bytes([0x6b; 16]);
pub const OWN: SpaceId = SpaceId::from_bytes([0x6c; 16]);

/// A link object for the device of seed `new`, by the issuing device of seed `issuer`.
#[derive(Debug, Clone)]
pub struct Linked {
    pub new: u8,
    /// The device the body names as `issuer`.
    pub issuer: u8,
    /// The revisions the new device starts from, in the body's order.
    pub starts: Vec<(SpaceId, [u8; 32])>,
    pub personal: Option<SpaceId>,
    /// Fields of later versions.
    pub later: Vec<(i64, Value)>,
    /// The seed of the key that signs it, and the label its signature covers.
    pub signer: u8,
    pub label: &'static str,
}

impl Linked {
    /// The laptop's link object for the desktop: `umbrella`, then `initech`, then the personal
    /// space, which is anchored too, and the personal space.
    pub fn desktop() -> Self {
        Linked {
            new: DESKTOP,
            issuer: LAPTOP,
            starts: vec![
                (UMBRELLA, [0xd4; 32]),
                (INITECH, [0xe5; 32]),
                (OWN, [0xf6; 32]),
            ],
            personal: Some(OWN),
            later: Vec::new(),
            signer: LAPTOP,
            label: LINK,
        }
    }

    /// As [`Linked::desktop`], by an issuing device that pinned no personal space.
    pub fn no_personal() -> Self {
        Linked {
            starts: vec![(INITECH, [0xe5; 32])],
            personal: None,
            ..Self::desktop()
        }
    }

    /// As [`Linked::desktop`], by an issuing device that accepted no revision yet.
    pub fn nothing_accepted() -> Self {
        Linked {
            starts: Vec::new(),
            personal: None,
            ..Self::desktop()
        }
    }

    /// The body's fields, numbered as the spec numbers them, fields of later versions last.
    pub fn fields(&self) -> Vec<(i64, Value)> {
        let mut fields = vec![
            (1, bytes(device(self.new).as_bytes())),
            (2, bytes(device(self.issuer).as_bytes())),
            (3, starts_value(&self.starts)),
        ];
        if let Some(personal) = self.personal {
            fields.push((4, bytes(personal.as_bytes())));
        }
        fields.extend(self.later.clone());
        fields
    }

    /// The encoded body.
    pub fn body(&self) -> Vec<u8> {
        Value::Map(self.fields()).encode()
    }

    /// The signature by `signer` over `[label, body]`.
    pub fn signature(&self) -> [u8; 64] {
        sign(self.signer, self.label, &self.body())
    }

    /// The encoded link object.
    pub fn encoded(&self) -> Vec<u8> {
        signed_cbor(&self.body(), &self.signature())
    }

    /// The link object as an item of another object: `[body, signature]`.
    pub fn value(&self) -> Value {
        Value::Array(vec![bytes(&self.body()), bytes(&self.signature())])
    }

    /// The body as a client builds it; only for one without fields of later versions.
    pub fn proto(&self) -> LinkBody {
        assert!(self.later.is_empty(), "a body with later fields");
        LinkBody {
            device: device(self.new),
            issuer: device(self.issuer),
            anchors: self.starts.clone(),
            personal: self.personal,
            unknown: Unknown::default(),
        }
    }
}

/// The anchors of a link body: `[space, revision]` each, in order.
pub fn starts_value(starts: &[(SpaceId, [u8; 32])]) -> Value {
    Value::Array(
        starts
            .iter()
            .map(|(space, hash)| Value::Array(vec![bytes(space.as_bytes()), bytes(hash)]))
            .collect(),
    )
}

/// The desktop's request: its kind, keys and `name`.
pub fn request_fields() -> Vec<(i64, Value)> {
    vec![
        (1, Value::Uint(DeviceKind::Client as u64)),
        (2, bytes(&public(DESKTOP))),
        (3, bytes(&[0x3d; 32])),
        (4, text("Desktop at the office")),
    ]
}

/// The certificate body the laptop issues the desktop on its account, fields as the spec numbers
/// them.
pub fn cert_body_fields() -> Vec<(i64, Value)> {
    vec![
        (1, bytes(crate::fixtures::ACCOUNT.as_bytes())),
        (2, bytes(device(DESKTOP).as_bytes())),
        (3, Value::Uint(DeviceKind::Client as u64)),
        (4, bytes(&public(DESKTOP))),
        (5, bytes(&[0x3d; 32])),
        (6, bytes(device(LAPTOP).as_bytes())),
        (7, Value::Uint(1_800_000_500)),
        (9, text("Desktop at the office")),
    ]
}

/// A grant of the laptop to the desktop: the master key, `cert`, an encoded certificate body, the
/// laptop's signature over it and the link object `link`; fields of later versions last.
pub fn grant_fields(cert: &[u8], link: &Linked, later: &[(i64, Value)]) -> Vec<(i64, Value)> {
    let mut fields = vec![
        (1, bytes(&public(crate::fixtures::MASTER))),
        (2, bytes(cert)),
        (3, bytes(&sign(LAPTOP, CERT, cert))),
        (4, link.value()),
    ];
    fields.extend(later.iter().cloned());
    fields
}
