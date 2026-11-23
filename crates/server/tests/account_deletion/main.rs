//! Account deletion on the server: a session with a recent sign-in deletes its account, never while
//! the account is the only admin of a space with other members. At once, on every instance, the
//! account loses its sessions, service tokens, link runs, pending device codes and sign-in methods;
//! its username is free; each of its devices is served as a revoked one; the spaces it alone was
//! in are deleted with their channels; the spaces it shared stay, keep it in their revisions and
//! mark it deleted, and their devices get a `space-notice`. A space's other members are the accounts
//! of its latest revision but the account itself and those already deleted. Against a
//! `skald-server` on PostgreSQL with the GitHub stand-in, its accounts, spaces, revisions and
//! devices as in the tests of revocations on the server; every response is checked against the
//! OpenAPI spec in `api/openapi.json`.
//!
//! The interfaces, as these tests use them, each request stating the protocol version:
//! - `GET /account/deletion`, with a session of the account: `200` with `{"deletes": [<space>,
//!   ...], "blocks": [<space>, ...]}`, each `<space>` an object with the space's ID as `"space"`:
//!   `deletes`, the spaces where the account is the only member, its personal space included;
//!   `blocks`, those where it is the only admin and there are other members.
//! - `DELETE /account`: deletes the session's account, `204` without a body. Only with a web
//!   session's cookie from a page of the server, as its `Origin` says, or with a device session's
//!   `Authorization: Bearer <token>`, and only with a sign-in more recent than
//!   `--link-sign-in-age`, 300 seconds unset. Refused, with a problem details object and nothing
//!   deleted: `409` with the problem `https://skald.so/problems/deletion-blocked` and `"spaces":
//!   [<space>, ...]`, as `blocks` has them, while a space blocks it; the problem
//!   `https://skald.so/problems/sign-in-too-old` for an older sign-in; another `4xx` for a request
//!   from another site or without `Origin`, a service token or no session.
//! - The connections of the account's sessions close, on every instance, within a second, with code
//!   1008 and the reason `account-deleted`.
//! - `GET /spaces/{space}/members`: a member whose account is deleted has `"deleted": true`; no
//!   other member has it true.
//! - `GET /spaces/{space}/devices`: besides `devices`, `revocations` and `records`, which still
//!   give a deleted account's devices, certificates and revocations, `"deleted": [<account ID>,
//!   ...]`, the listed devices' accounts that are deleted, each once; left out, it is empty.
//! - A revision that leaves a space with other members whose every admin is deleted is refused with
//!   a `4xx` and a problem details object.
//! - `POST /devices/{device}/session` answers a deleted account's device with `403
//!   account-deleted` once the signature by the key in its server-held certificate verifies, and
//!   `403 device-revoked` for a new certificate it would need to store. `POST /links` answers
//!   `403 device-revoked` and the link runs its devices opened without a session end. So too with
//!   `devices.revoked` false in the database, as a request racing the deletion could leave it.
//! - A request whose session the server read before a deletion ended it, still running then, acts
//!   for no deleted account. The tests hold a deletion while it runs by a lock on the account's
//!   `sign_in_methods` rows in the database, which it takes once it holds the account's row.
//! - `sessions.created_at` in the database is a session's sign-in.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

#[path = "../common/mod.rs"]
mod common;

// The spaces tests' accounts and revisions, the fixture and frames of the tests of storing sent
// envelopes, the connections and notices of the tests of delivering a mailbox, the proofs of the
// tests of the mailbox over HTTPS, the revocations and certified devices of the tests of
// revocations on the server, and the stage of the tests of service tokens.
#[path = "../spaces/api.rs"]
mod api;
#[path = "../mailbox/fixture.rs"]
mod fixture;
#[path = "../mailbox_https/https.rs"]
mod https;
#[path = "../revocations/kit.rs"]
mod kit;
#[path = "../mailbox_delivery/link.rs"]
mod link;
#[path = "../mailbox_delivery/listener.rs"]
mod listener;
#[path = "../spaces/rosters.rs"]
mod rosters;
#[path = "../service_tokens/tokens.rs"]
mod tokens;
#[path = "../mailbox/wire.rs"]
mod wire;

// The API calls, sessions and connections of the tests of sessions on the web.
#[path = "../web_sessions/sessions.rs"]
mod sessions;

// The link runs of the tests of the link code relay, those of a new device without a session, the
// certificates a linked device issues and a session by a device's key.
#[path = "../device_key_sessions/kit.rs"]
mod by_key;
#[path = "../link_relay/kit.rs"]
mod relay;
#[path = "../sessionless_links/kit.rs"]
mod runs;
#[path = "../linked_certificates/upload.rs"]
mod upload;

// The KeyPackages of the tests of KeyPackages on the server.
#[path = "../key_packages/answers.rs"]
mod answers;
#[path = "../key_packages/fixture.rs"]
mod packages;

// The channel's group and history uploads of the tests of a channel's history.
#[path = "../channel_history/kit.rs"]
mod history;
#[path = "../delivery_service/scene.rs"]
mod scene;

mod deletion;

mod after;
mod blocking;
mod deleting;
mod delivery;
mod held;
mod racing;
mod revoked;
mod sign_ins;
mod spaces;
mod traces;
