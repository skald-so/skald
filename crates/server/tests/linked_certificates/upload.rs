//! Storing the certificate of a device that a linked device of the account issued: `PUT
//! /devices/{device}/cert`, and the certificates the tests issue. Needs only the common helpers, so
//! that any test binary of the server includes it.

use std::sync::OnceLock;

use serde_json::{Value, json};
use skald_proto::{AccountId, DeviceId, DeviceKind};

use crate::common::devices::{Device, b64, cert_of};
use crate::common::spec::{operation_violations, spec};
use crate::common::{Response, send};

/// The path of the operation in the spec.
pub const CERT_PATH: &str = "/devices/{device}/cert";

/// The problem of a certificate over the devices limit.
pub const DEVICES_LIMIT: &str = "https://skald.so/problems/devices-limit";

/// The spec, read once.
fn the_spec() -> &'static Value {
    static SPEC: OnceLock<Value> = OnceLock::new();
    SPEC.get_or_init(spec)
}

/// `PUT /devices/<device>/cert` of `request` on the server at `base`, as the session `token`;
/// checked against the spec.
pub fn put_cert_request(base: &str, token: &str, device: DeviceId, request: &Value) -> Response {
    let bearer = format!("Bearer {token}");
    let version = skald_proto::PROTOCOL_VERSION.to_string();
    let headers = [
        ("content-type", "application/json"),
        ("authorization", bearer.as_str()),
        (skald_proto::VERSION_HEADER, version.as_str()),
    ];
    let body = serde_json::to_vec(request).expect("JSON");
    let url = format!("{base}/devices/{device}/cert");
    let response = send("put", &url, &headers, &body);
    let violations = operation_violations(the_spec(), "put", CERT_PATH, &response);
    assert_eq!(
        violations,
        Vec::<String>::new(),
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    response
}

/// `PUT /devices/<device>/cert` of the certificate `cert` on the server at `base`, as the session
/// `token`.
pub fn put_cert(base: &str, token: &str, device: DeviceId, cert: &[u8]) -> Response {
    put_cert_request(base, token, device, &json!({ "cert": b64(cert) }))
}

/// The certificate of `device` as a device of `kind` on `account`, its body as `device` registered
/// itself, issued by the device `issuer`: signed with `issuer`'s key, the proof with `device`'s.
pub fn issued_by(
    device: &Device,
    account: AccountId,
    kind: DeviceKind,
    issuer: &Device,
) -> Vec<u8> {
    let mut body = device.body_as(account, kind);
    body.issuer = Some(issuer.id);
    cert_of(&body, &issuer.signing.key, &device.signing.key)
}

/// Checks that `response` is the answer to a certificate the server stored: `201`, or `200` when
/// it held it already.
#[track_caller]
pub fn assert_stored(response: &Response, what: &str) {
    assert!(
        matches!(response.status, 200 | 201),
        "{what}: stored, not {} {}",
        response.status,
        String::from_utf8_lossy(&response.body)
    );
}

/// Checks that `response` refuses a certificate as over the devices limit: `403` with its problem.
#[track_caller]
pub fn assert_over_limit(response: &Response, what: &str) {
    assert_problem(response, 403, DEVICES_LIMIT, what);
}

/// Checks that `response` refuses with `status` and the problem `problem`; `what` names the case.
#[track_caller]
pub fn assert_problem(response: &Response, status: u16, problem: &str, what: &str) {
    let body: Value = serde_json::from_slice(&response.body).unwrap_or(Value::Null);
    assert_eq!(
        (response.status, body["type"].as_str()),
        (status, Some(problem)),
        "{what}: {}",
        String::from_utf8_lossy(&response.body)
    );
    assert!(
        response
            .content_type
            .starts_with("application/problem+json"),
        "{what}: {}",
        response.content_type
    );
}
