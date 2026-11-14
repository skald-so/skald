//! Spaces and their roster revisions on the server (`docs/protocol/membership.md`, "Roster"): a
//! signed-in account creates a space by name, the server stores its revisions in order and only
//! from its admins' sessions, and gives a space only to its members' sessions. Against a
//! `skald-server` with GitHub sign-in through the GitHub stand-in, on PostgreSQL; every response is
//! checked against the OpenAPI spec in `api/openapi.json`.
//!
//! The API, as these tests use it, each request stating the protocol version and with the
//! session's `Authorization: Bearer <token>`:
//! - `POST /spaces` of `{"name": <name>}`: creates a space the session's account owns; answers
//!   `{"space": <ID>, ...}`.
//! - `GET /spaces`: `{"spaces": [{"space": <ID>, "name": <name>}, ...]}`, the spaces the session's
//!   account gets.
//! - `POST /spaces/<ID>/revisions` of `{"revision": <bytes>}`: stores a revision.
//! - `GET /spaces/<ID>/revisions`: `{"revisions": [<bytes>, ...]}`, the stored revisions from
//!   revision 1 on, in order.
//! - `GET /spaces/<ID>/devices`: `{"devices": [{"device": <ID>, "cert": <bytes> | null}, ...]}`,
//!   the devices of the accounts the space's revisions list.
//! - `POST /spaces/<ID>/channels`, without a body: `{"channel": <ID>}`, a new channel ID for the
//!   space.
//! - `POST /account/personal-space`, without a body: `{"space": <ID>, "name": <name>, ...}`, the
//!   session's account's personal space; `201` when the request creates it, `200` after.
//!
//! Bytes are base64url without padding (RFC 4648, §5); a revision is its CBOR bytes, as is a
//! certificate. Taken: `200` or `201`. Refused, with a problem details object: `401` without a
//! valid session, another `4xx` otherwise. A path parameter may have any name in the spec.

#[path = "../common/mod.rs"]
mod common;

mod access;
mod api;
mod authority;
mod channels;
mod checks;
mod create;
mod order;
mod owned;
mod personal;
mod refusals;
mod retry;
mod rosters;
