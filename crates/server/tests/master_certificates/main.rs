//! Certificates of devices linked with the recovery key (`docs/protocol/membership.md`, "Linking a
//! device", "With the recovery key"): a session of an account that has a master key stores the
//! certificate of a device of the account registered without one, issued by the master key, with
//! the checks and the devices limit of a certificate a linked device issued. With `--devices-limit`,
//! a registration at the limit is taken, and a certificate over it is refused. Against a
//! `skald-server` with GitHub sign-in through the stand-in, as in the spaces tests; every HTTP
//! response is checked against the OpenAPI spec in `api/openapi.json`.
//!
//! The interface, as these tests use it, each request stating the protocol version with a
//! session's token:
//! - `PUT /devices/{device}/cert` of `{"cert": <bytes>}`, the complete certificate of `device`, its
//!   `issuer` absent: stored with `201`; the same certificate again with `200`. The account's
//!   `GET /devices` then gives it as sent.
//! - Refused, with a problem details object and nothing stored: with `403` and the problem
//!   `https://skald.so/problems/devices-limit` over the devices limit; with `400` and the problem
//!   `https://skald.so/problems/invalid-certificate` for one whose signature does not verify with
//!   the account's master key.
//!
//! Bytes are base64url without padding (RFC 4648, §5).

#[path = "../common/mod.rs"]
mod common;

// The spaces tests' accounts and their requests; this binary uses some of them.
#[allow(dead_code)]
#[path = "../spaces/api.rs"]
mod api;

mod limit;
mod refusals;
mod storing;

use serde_json::{Value, json};
use skald_proto::DeviceKind;

use api::Person;
use common::Response;
use common::device_flow::text;
use common::devices::{Device, Key, b64, devices_of};

/// The problems of a certificate over the devices limit and of an invalid one.
pub const DEVICES_LIMIT: &str = "https://skald.so/problems/devices-limit";
pub const INVALID_CERTIFICATE: &str = "https://skald.so/problems/invalid-certificate";

/// The certificate of `device` on `person`'s account, as it registered itself, issued by `master`,
/// the account's master key or another.
pub fn issued_by_master(person: &Person, device: &Device, master: &Key) -> Vec<u8> {
    device.cert_as(person.account, DeviceKind::Client, master)
}

/// `PUT /devices/<device>/cert` of `cert` from `person`'s session; checked against the spec.
pub fn put_cert(person: &Person, device: &Device, cert: &[u8]) -> Response {
    let path = format!("/devices/{}/cert", device.id);
    person
        .session
        .call("put", &path, Some(&json!({ "cert": b64(cert) })))
}

/// Registers `device` on `person`'s account from its session; fails unless the server takes it.
pub fn register(person: &Person, device: &Device) {
    let response = person
        .session
        .call("post", "/devices", Some(&device.registration()));
    assert!(
        matches!(response.status, 200 | 201),
        "the registration of {}: {}",
        device.name,
        text(&response)
    );
}

/// The certificate the server gives `person`'s session for `device`, if any.
pub fn held(person: &Person, device: &Device) -> Option<Vec<u8>> {
    devices_of(&person.session.call("get", "/devices", None))
        .remove(&device.id.to_string())
        .flatten()
}

/// Checks that `response` refuses with `status` and the problem `problem`; `what` names the case.
#[track_caller]
pub fn assert_problem(response: &Response, status: u16, problem: &str, what: &str) {
    let body: Value = serde_json::from_slice(&response.body).unwrap_or(Value::Null);
    assert_eq!(
        (response.status, body["type"].as_str()),
        (status, Some(problem)),
        "{what}: {}",
        text(response)
    );
    assert!(
        response
            .content_type
            .starts_with("application/problem+json"),
        "{what}: {}",
        response.content_type
    );
}
