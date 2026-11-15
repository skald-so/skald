//! KeyPackages on the server (`docs/protocol/membership.md`, "KeyPackages"): it takes a device's
//! KeyPackages from a session of the device's account, only for its `client` device with a
//! certificate that is not revoked, and only if they pass the checks: the MLS signature, a
//! lifetime that holds and spans at most 60 days, ciphersuite `0x0001`, and the device's
//! `identity` and `signature_key`. It counts the unused ones that expire in more than 7 days and
//! says whether the last-resort one is to be replaced. It hands out each other KeyPackage once and
//! deletes it, the last-resort one only when no other remains, and no expired one; only to
//! sessions of accounts that share a space with the device, and none of a revoked device. Two
//! requests at once never get one KeyPackage, also through two instances. Against a `skald-server`
//! with GitHub sign-in through the GitHub stand-in, on PostgreSQL; the tests' devices make their
//! KeyPackages with the client's own code, and those that break the checks with an MLS client of
//! the tests' own.
//!
//! The API, each request stating the protocol version and each response checked against the
//! OpenAPI spec in `api/openapi.json`; bytes are base64url without padding, a KeyPackage in an
//! `MLSMessage`:
//! - `POST /devices/{device}/key-packages` of `{"key_packages": [<bytes>, ...], "last_resort":
//!   <bytes>}`, either field left out: uploads KeyPackages of the device and its last-resort one,
//!   which replaces the one it had. Taken: `200` or `201`. Refused: a `4xx` with a problem details
//!   object.
//! - `GET /devices/{device}/key-packages`: `{"unused": <count>, "replace_last_resort": <bool>}`,
//!   the count of the device's unused KeyPackages that expire in more than 7 days, and whether its
//!   last-resort one is missing, was handed out or expires within 7 days.
//! - `POST /key-packages` of `{"devices": [<ID>, ...]}`: hands out one KeyPackage for each device
//!   asked for that has one for the session, `200` with `{"key_packages": [{"device": <ID>,
//!   "key_package": <bytes>}, ...]}`; a device without one is left out.
//! - `POST /account/revocations` of `{"revocation": <bytes>}`: stores a revocation of a device of
//!   the session's account. Taken: `200` or `201`.
//!
//! The database: the table `key_packages`, whose `timestamptz` columns hold when each KeyPackage
//! expires; the tests move them back to expire KeyPackages.

#[path = "../common/mod.rs"]
mod common;

// The spaces tests' accounts and revisions; this binary uses some of them.
#[allow(dead_code)]
#[path = "../spaces/api.rs"]
mod api;
#[allow(dead_code)]
#[path = "../spaces/rosters.rs"]
mod rosters;

mod answers;
mod fixture;
mod foreign;

mod handing_out;
mod instances;
mod refusals;
mod revoked;
mod taking;
