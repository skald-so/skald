//! The tests' anchors objects: built from their spec fields, devices by the seed of their signing
//! key, encoded by [`crate::cbor`] and signed with ed25519-dalek over `[label, body]`, apart from
//! the crates under test.

use skald_proto::{AccountId, AnchorsBody, DeviceId, SpaceId, Unknown};

use crate::cbor::{Value, bytes};
use crate::fields::signed_cbor;
use crate::fixtures::{ACCOUNT, LAPTOP, device, sign};

/// The label of an anchors object's signature.
pub const LABEL: &str = "skald anchors";

/// The spaces the tests' anchors name: two team spaces and the account's personal space.
pub const ACME: SpaceId = SpaceId::from_bytes([0x5a; 16]);
pub const GLOBEX: SpaceId = SpaceId::from_bytes([0x5b; 16]);
pub const PERSONAL: SpaceId = SpaceId::from_bytes([0x5c; 16]);

/// The spec fields of an anchors body: `account`, `device`, each anchor as `[space, revision]`
/// and, if any, `personal`.
pub fn spec_fields(
    account: AccountId,
    device: DeviceId,
    anchors: &[(SpaceId, [u8; 32])],
    personal: Option<SpaceId>,
) -> Vec<(i64, Value)> {
    let listed = anchors
        .iter()
        .map(|(space, revision)| Value::Array(vec![bytes(space.as_bytes()), bytes(revision)]))
        .collect();
    let mut fields = vec![
        (1, bytes(account.as_bytes())),
        (2, bytes(device.as_bytes())),
        (3, Value::Array(listed)),
    ];
    fields.extend(personal.map(|personal| (4, bytes(personal.as_bytes()))));
    fields
}

/// An anchors object's spec fields, its device by seed, and how it is signed.
#[derive(Debug, Clone)]
pub struct Anch {
    pub account: AccountId,
    pub device: u8,
    /// Each space and the hash of the latest revision of it the device accepted, in order.
    pub anchors: Vec<(SpaceId, [u8; 32])>,
    pub personal: Option<SpaceId>,
    /// Fields of later versions.
    pub extra: Vec<(i64, Value)>,
    /// The seed of the key that signs it.
    pub key: u8,
    /// The label its signature covers.
    pub label: &'static str,
}

impl Anch {
    /// The laptop's anchors object: `globex` and `acme`, in that order, and the personal space,
    /// which is anchored too.
    pub fn of_laptop() -> Self {
        Self {
            account: ACCOUNT,
            device: LAPTOP,
            anchors: vec![
                (GLOBEX, [0xb2; 32]),
                (ACME, [0xa1; 32]),
                (PERSONAL, [0xc3; 32]),
            ],
            personal: Some(PERSONAL),
            extra: Vec::new(),
            key: LAPTOP,
            label: LABEL,
        }
    }

    /// As [`Anch::of_laptop`], of a device that pinned no personal space.
    pub fn without_personal() -> Self {
        Self {
            anchors: vec![(ACME, [0xa1; 32])],
            personal: None,
            ..Self::of_laptop()
        }
    }

    /// As [`Anch::of_laptop`], of a device that accepted no revision yet.
    pub fn empty() -> Self {
        Self {
            anchors: Vec::new(),
            personal: None,
            ..Self::of_laptop()
        }
    }

    /// The fields of the body, as the spec numbers them.
    pub fn fields(&self) -> Vec<(i64, Value)> {
        let mut fields = spec_fields(
            self.account,
            device(self.device),
            &self.anchors,
            self.personal,
        );
        fields.extend(self.extra.iter().cloned());
        fields
    }

    /// The body, a map.
    pub fn value(&self) -> Value {
        Value::Map(self.fields())
    }

    /// The encoded body.
    pub fn body(&self) -> Vec<u8> {
        self.value().encode()
    }

    /// The signature by `key` over `[label, body]`.
    pub fn signature(&self) -> [u8; 64] {
        sign(self.key, self.label, &self.body())
    }

    /// The encoded anchors object: `[body, signature]`.
    pub fn bytes(&self) -> Vec<u8> {
        signed_cbor(&self.body(), &self.signature())
    }

    /// The body as a client builds it; only for an object without fields of later versions.
    pub fn proto_body(&self) -> AnchorsBody {
        assert!(
            self.extra.is_empty(),
            "a body with fields of later versions"
        );
        AnchorsBody {
            account: self.account,
            device: device(self.device),
            anchors: self.anchors.clone(),
            personal: self.personal,
            unknown: Unknown::default(),
        }
    }
}
