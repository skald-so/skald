//! A session of a device by its own key: the challenge it signs, the request that stores its
//! certificate and gives it a session, and the token in the answer. Needs only the common helpers
//! and the spaces tests' requests, so that the tests of link runs without a session include it.

use serde_json::{Value, json};
use skald_crypto::SigningKey;
use skald_proto::DeviceId;

use crate::api::Session;
use crate::common::Response;
use crate::common::device_flow::{DeviceFlow, json, text};
use crate::common::devices::{Device, b64, unb64};

/// The label of the signature in a request for a device's session.
pub const LABEL: &str = "skald device session";

/// `POST /devices/<device>/challenge` from `nobody`, requests without a session.
pub fn challenge_response(nobody: &Session, device: DeviceId) -> Response {
    nobody.call("post", &format!("/devices/{device}/challenge"), None)
}

/// A fresh challenge for `device`; the server must give it.
#[track_caller]
pub fn challenge(nobody: &Session, device: DeviceId) -> Vec<u8> {
    let response = challenge_response(nobody, device);
    assert_eq!(response.status, 200, "a challenge: {}", text(&response));
    let body = json(&response);
    let challenge = unb64(
        body["challenge"]
            .as_str()
            .unwrap_or_else(|| panic!("no challenge in {body}")),
    );
    assert_eq!(challenge.len(), 32, "{body}");
    challenge
}

/// The signature by `key` over `challenge` and `origin`, the server's public address.
pub fn signature(key: &SigningKey, challenge: &[u8], origin: &str) -> [u8; 64] {
    key.sign(LABEL, &[challenge, origin.as_bytes()])
}

/// The body of a request for a session of `device` on `challenge`, signed by `key` over `origin`,
/// with the complete certificate `cert` if given.
pub fn body_by(key: &SigningKey, challenge: &[u8], origin: &str, cert: Option<&[u8]>) -> Value {
    let mut body = json!({
        "challenge": b64(challenge),
        "signature": b64(&signature(key, challenge, origin)),
    });
    if let Some(cert) = cert {
        body["cert"] = json!(b64(cert));
    }
    body
}

/// `POST /devices/<device>/session` of `body` from `nobody`, requests without a session.
pub fn ask(nobody: &Session, device: DeviceId, body: &Value) -> Response {
    nobody.call("post", &format!("/devices/{device}/session"), Some(body))
}

/// A request for a session of `device`, as its machine makes it on `flow`'s server: a fresh
/// challenge from `nobody`, signed with the device's key, and its complete certificate `cert`.
pub fn sign_in_by_key(
    flow: &DeviceFlow,
    nobody: &Session,
    device: &Device,
    cert: &[u8],
) -> Response {
    let challenge = challenge(nobody, device.id);
    let body = body_by(
        &device.signing.key,
        &challenge,
        &flow.stand.public,
        Some(cert),
    );
    ask(nobody, device.id, &body)
}

/// The session token in `response`, which must give a session: `200` or `201`.
#[track_caller]
pub fn token_of(response: &Response) -> String {
    assert!(
        matches!(response.status, 200 | 201),
        "a session: {}",
        text(response)
    );
    let body = json(response);
    body["token"]
        .as_str()
        .unwrap_or_else(|| panic!("no token in {body}"))
        .to_owned()
}
