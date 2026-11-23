//! Invites and acceptances on the server (`docs/protocol/membership.md`, "Invite"): the server
//! issues invite IDs and stores invites only for sessions of the space's admins, each signed by a
//! certified, unrevoked `client` device of the session's account, of that space with an ID issued
//! for it, expiring within 30 days, naming channels of the latest revision with its default channel
//! first. It gives an invite to a device session that names its ID, with its space's revisions and
//! devices, until the invite has an acceptance; then only to the account of that acceptance, whose
//! sessions also get the space as a member's do. It keeps the first acceptance of an invite, from
//! a session of the account it names, with that account's master key, signed by the invite key,
//! before `expires`, and refuses every other. At an acceptance it sends `space-notice` to the
//! admins' devices, on every instance, and gives the waiting acceptance, with its account's devices
//! and revocations, only to admins' sessions. An invite ends when it expires or a stored revision
//! lists a member with it; the server then gives neither it nor its acceptance. Against a
//! `skald-server` as in the tests of storing sent envelopes, whose fixture and frames these tests
//! share; every HTTP response is checked against the OpenAPI spec in `api/openapi.json`.
//!
//! The interfaces, as these tests use them, each request stating the protocol version, with a
//! device session's `Authorization: Bearer <token>`:
//! - `POST /spaces/{space}/invites`, without a body: `201` with `{"invite": <ID>, "web": <URL>}`, a
//!   new invite ID for the space and the server's web address.
//! - `PUT /spaces/{space}/invites/{invite}` of `{"invite": <bytes>}`: stores the invite with that
//!   ID; `201` once stored, `200` for the same bytes again.
//! - `GET /invites/{invite}`: `200` with `{"invite": <bytes>, "revisions": [<bytes>, ...],
//!   "devices": [{"device": <ID>, "cert": <bytes> | null}, ...], "revocations": [<bytes>, ...],
//!   ...}`: the invite as the server got it, and its space's revisions, devices and revocations as
//!   `GET /spaces/{space}/revisions` and `GET /spaces/{space}/devices` give them to a member.
//! - `PUT /invites/{invite}/acceptance` of `{"acceptance": <bytes>}`: stores the invite's
//!   acceptance; `201` once stored, `200` for the same bytes again.
//! - `GET /spaces/{space}/acceptances`: `200` with `{"acceptances": [{"invite": <bytes>,
//!   "acceptance": <bytes>, "devices": [{"device": <ID>, "cert": <bytes> | null}, ...],
//!   "revocations": [<bytes>, ...]}, ...]}`: the space's waiting acceptances, each with its invite
//!   and its account's devices and revocations.
//!   A session of an account that is no admin of the space gets none: a refusal or an empty list.
//! - `space-notice = [9, space: id]` ("Frames"), as in the tests of server instances.
//!
//! Bytes are base64url without padding; an invite, an acceptance and a revision are their CBOR
//! bytes. Refused, with a problem details object and nothing stored: `401` without a session or with
//! a service token, another `4xx` otherwise.

// Each module shared with other test binaries is used in part.
#![allow(dead_code)]

#[path = "../common/mod.rs"]
mod common;

// The connections of the tests of server instances, and the service tokens of the tests of service
// tokens.
#[path = "../service_tokens/tokens.rs"]
mod tokens;
#[path = "../server_instances/watch.rs"]
mod watch;
// The revocations and certified devices of the tests of revocations.
#[path = "../revocations/kit.rs"]
mod kit;
// The spaces tests' accounts and revisions.
#[path = "../spaces/api.rs"]
mod api;
#[path = "../spaces/rosters.rs"]
mod rosters;
// The fixture and frames of the tests of storing sent envelopes, the entries of the tests of
// delivering a mailbox and the proofs of the tests of the mailbox over HTTPS.
#[path = "../mailbox/fixture.rs"]
mod fixture;
#[path = "../mailbox_https/https.rs"]
mod https;
#[path = "../mailbox_delivery/link.rs"]
mod link;
#[path = "../mailbox/wire.rs"]
mod wire;

mod scene;

mod accepting;
mod ending;
mod issuing;
mod reading;
mod waiting;
