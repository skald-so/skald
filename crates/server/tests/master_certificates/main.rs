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

// The link code relay's accounts and messages, which the helpers of the tests of linked devices'
// certificates use.
#[allow(dead_code)]
#[path = "../link_relay/kit.rs"]
mod kit;

// The tests of linked devices' certificates: storing a certificate, registering a device and the
// certificates the account's devices hold; this binary uses some of them.
#[allow(dead_code, unused_imports)]
#[path = "../linked_certificates/accounts.rs"]
mod accounts;
#[allow(dead_code)]
#[path = "../linked_certificates/upload.rs"]
mod upload;

mod limit;
mod refusals;
mod storing;

use skald_proto::DeviceKind;

use api::Person;
use common::devices::{Device, Key};

/// The problem of an invalid certificate.
pub const INVALID_CERTIFICATE: &str = "https://skald.so/problems/invalid-certificate";

/// The certificate of `device` on `person`'s account, as it registered itself, issued by `master`,
/// the account's master key or another.
pub fn issued_by_master(person: &Person, device: &Device, master: &Key) -> Vec<u8> {
    device.cert_as(person.account, DeviceKind::Client, master)
}
