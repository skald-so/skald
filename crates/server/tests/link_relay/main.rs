//! The link code relay (`docs/protocol/membership.md`, "Link code"): the server relays a CPace run
//! between sessions of one account without seeing the code. A run opens with a hello that names a
//! device of the account registered without a certificate; the server gives the account's latest
//! hello, and relays each later message by its `session`, as the bytes it got, across instances. A
//! run ends on a `link-abort` from either side, once its device has a certificate and at the code's
//! expiry; a device's new hello replaces its open run. While a run is open, the account's sessions
//! get its device's registered `name` and kind. Against a `skald-server` with GitHub sign-in through
//! the stand-in, as in the spaces tests; every HTTP response is checked against the OpenAPI spec in
//! `api/openapi.json`.
//!
//! The interfaces, as these tests use them, each request stating the protocol version with a
//! session's token:
//! - `POST /account/links` of `{"device": <ID>, "hello": <bytes>}`: a `link-hello` naming the new
//!   device; taken with `201`, which opens the device's run and replaces its open one.
//! - `GET /account/links/hello`: `{"hello": <bytes> | null}`, the latest hello of the account's
//!   open runs.
//! - `POST /account/links/messages` of `{"message": <bytes>}`: a later message of the run its
//!   `session` names, `link-reply`, `link-request`, `link-grant` or `link-abort`; taken with `201`.
//! - `GET /account/links/messages/{session}`, `session` in base64url without padding:
//!   `{"messages": [<bytes>, ...]}`, the run's later messages, in the order the server took them.
//! - `GET /account/links/waiting`: `{"devices": [{"device": <ID>, "kind": "client", "name":
//!   <name>}, ...]}`, the account's devices with an open run.
//! - `--link-code-lifetime <SECONDS>`: how long after its hello a run ends; 5 minutes unset.
//!
//! Bytes are base64url without padding (RFC 4648, §5); a message is its CBOR bytes. Refused, with a
//! problem details object: `401` without a valid session, another `4xx` otherwise, such as for a
//! message that does not decode or names a run the server does not hold for the session's account.
//! A refused message is not stored or relayed. `devices`: one row per device, as the tests of the
//! cloud limits write them; these tests give a waiting device its certificate there.

#[path = "../common/mod.rs"]
mod common;

// The spaces tests' accounts and their requests; this binary uses some of them.
#[allow(dead_code)]
#[path = "../spaces/api.rs"]
mod api;

mod kit;

mod ends;
mod refusals;
mod runs;
mod storage;
mod waiting;
