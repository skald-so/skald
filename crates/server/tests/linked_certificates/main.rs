//! Certificates of devices linked by another device (`docs/protocol/membership.md`, "Linking a
//! device" and "Link code", step 4 of the new device): a session of the account stores the
//! certificate of a device of the account registered without one, issued by a `client` device of
//! the account. The server takes it only after checks 1 and 2 of "Device checks" against the
//! account's master key and the certificates it holds, and only if it matches the device's
//! registration, as it does for the first device's certificate. Once it is stored the device's link
//! code run ends. With `--devices-limit`, a certificate that would give the account more devices
//! than the limit is refused, also when certificates arrive at once; a registration does not count
//! against it. Against a `skald-server` with GitHub sign-in through the stand-in, as in the spaces
//! tests; every HTTP response is checked against the OpenAPI spec in `api/openapi.json`.
//!
//! The interface, as these tests use it, each request stating the protocol version with a
//! session's token:
//! - `PUT /devices/{device}/cert` of `{"cert": <bytes>}`, the complete certificate of `device`:
//!   stored with `201`; the same certificate again, a retry after a lost answer, with `200`. The
//!   account's `GET /devices` then gives it as sent.
//! - Refused, with a problem details object and nothing stored: `401` without a valid session; with
//!   `403` and the problem `https://skald.so/problems/devices-limit` over the devices limit; with
//!   another `4xx` for a malformed body or certificate, one that fails check 1 or 2, one whose
//!   account, device, kind, keys or `name` differ from the device's registration, a device the
//!   session's account has not registered, and another certificate of a device that holds one.
//!
//! Bytes are base64url without padding (RFC 4648, §5). `devices`: one row per device, as the tests
//! of the cloud limits write them; these tests write devices the API does not register, such as a
//! `browser` device or one certified by another master key.

#[path = "../common/mod.rs"]
mod common;

// The spaces tests' accounts and their requests; this binary uses some of them.
#[allow(dead_code)]
#[path = "../spaces/api.rs"]
mod api;

// The link code relay's accounts and messages; this binary uses some of them.
#[allow(dead_code)]
#[path = "../link_relay/kit.rs"]
mod kit;

#[allow(dead_code)]
mod upload;

mod accounts;

mod limit;
mod refusals;
mod storing;
