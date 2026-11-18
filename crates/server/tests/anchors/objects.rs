//! What the tests share: anchors objects the tests encode byte by byte and sign with a device's key,
//! and the requests that store and list them.

use serde_json::{Value, json};
use skald_crypto::SigningKey;
use skald_proto::{AccountId, DeviceId, SpaceId};

use crate::api::{Person, Session, assert_refused};
use crate::common::cbor::{self, array_of, byte_item};
use crate::common::devices::{Device, b64, unb64};
use crate::common::{Response, device_flow};

/// Stores an anchors object: `POST`; the account's anchors objects: `GET`.
pub const ANCHORS: &str = "/account/anchors";
/// The problem of an anchors object the server does not store.
pub const INVALID_ANCHORS: &str = "https://skald.so/problems/invalid-anchors";
/// The label of an anchors object's signature.
pub const LABEL: &str = "skald anchors";

/// The spaces the tests' anchors objects name; the server does not check them.
pub const ACME: SpaceId = SpaceId::from_bytes([0x5a; 16]);
pub const PERSONAL: SpaceId = SpaceId::from_bytes([0x5c; 16]);

/// The encoded body `{1: account, 2: device, 3: anchors, ? 4: personal}`, each anchor a space and
/// the bytes of its revision's hash.
pub fn body(
    account: AccountId,
    device: DeviceId,
    anchors: &[(SpaceId, &[u8])],
    personal: Option<SpaceId>,
) -> Vec<u8> {
    let listed: Vec<Vec<u8>> = anchors
        .iter()
        .map(|(space, revision)| array_of(&[byte_item(space.as_bytes()), byte_item(revision)]))
        .collect();
    let mut fields = vec![
        (1, byte_item(account.as_bytes())),
        (2, byte_item(device.as_bytes())),
        (3, array_of(&listed)),
    ];
    if let Some(personal) = personal {
        fields.push((4, byte_item(personal.as_bytes())));
    }
    map(&fields)
}

/// The encoded map of `fields`, given in the order of their keys.
pub fn map(fields: &[(u64, Vec<u8>)]) -> Vec<u8> {
    let mut encoded = cbor::head(5, u64::try_from(fields.len()).expect("a length of 64 bits"));
    for (key, value) in fields {
        encoded.extend(cbor::uint(*key));
        encoded.extend_from_slice(value);
    }
    encoded
}

/// The encoded anchors object of `body` and the signature of `key` over `[label, body]`.
pub fn signed_as(body: &[u8], key: &SigningKey, label: &str) -> Vec<u8> {
    array_of(&[byte_item(body), byte_item(&key.sign(label, &[body]))])
}

/// The encoded anchors object of `body`, signed by `key` over `["skald anchors", body]`.
pub fn signed(body: &[u8], key: &SigningKey) -> Vec<u8> {
    signed_as(body, key, LABEL)
}

impl Person {
    /// The body of an anchors object of `device` on this person's account, naming `acme` at the
    /// revision whose hash is `revision` repeated, and the personal space.
    pub fn anchors_body(&self, device: &Device, revision: u8) -> Vec<u8> {
        body(
            self.account,
            device.id,
            &[(ACME, &[revision; 32]), (PERSONAL, &[0xc3; 32])],
            Some(PERSONAL),
        )
    }

    /// The anchors object of `device` on this person's account, signed by the device, as
    /// [`Person::anchors_body`] gives its body.
    pub fn anchors_of(&self, device: &Device, revision: u8) -> Vec<u8> {
        signed(&self.anchors_body(device, revision), &device.signing.key)
    }
}

impl Session {
    /// `POST /account/anchors` of `anchors`'s bytes.
    pub fn store_anchors(&self, anchors: &[u8]) -> Response {
        self.store_anchors_body(&json!({ "anchors": b64(anchors) }))
    }

    /// `POST /account/anchors` of `body`.
    pub fn store_anchors_body(&self, body: &Value) -> Response {
        self.call("post", ANCHORS, Some(body))
    }

    /// The anchors objects `GET /account/anchors` gives this session's account, each as its bytes,
    /// sorted.
    pub fn anchors(&self) -> Vec<Vec<u8>> {
        let response = self.call("get", ANCHORS, None);
        assert_eq!(
            response.status,
            200,
            "GET {ANCHORS}: {}",
            device_flow::text(&response)
        );
        let listed: Vec<String> = serde_json::from_value(
            device_flow::json(&response)["anchors"].clone(),
        )
        .unwrap_or_else(|err| panic!("GET {ANCHORS}: {err}: {}", device_flow::text(&response)));
        let mut objects: Vec<Vec<u8>> = listed.iter().map(|object| unb64(object)).collect();
        objects.sort();
        objects
    }
}

/// Checks that `response` refuses an anchors object: `400` with the problem `invalid-anchors`;
/// `what` names the case.
#[track_caller]
pub fn assert_invalid(response: &Response, what: &str) {
    assert_refused(response, what);
    assert_eq!(
        (response.status, device_flow::json(response)["type"].clone()),
        (400, Value::from(INVALID_ANCHORS)),
        "{what}: {}",
        device_flow::text(response)
    );
}

/// Checks that the server stored `anchors` from `session` as new: `201`.
#[track_caller]
pub fn assert_stored(session: &Session, anchors: &[u8], what: &str) {
    let response = session.store_anchors(anchors);
    assert_eq!(
        response.status,
        201,
        "{what}: {}",
        device_flow::text(&response)
    );
}
