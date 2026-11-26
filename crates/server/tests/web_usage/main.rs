//! Space limits and owned spaces on the web: a session of a space's member gets the space's limits
//! and its messages this month, and a session of an account gets the spaces the account owns and
//! its devices as the devices limit counts them, each count with its limit while the server has
//! the limit's setting (`docs/dev/architecture.md`, "Cloud limits"). Without a setting, its count
//! is still given and no limit applies, as on a self-hosted server. A session of an account that is
//! not a member gets nothing of the space. Against a `skald-server` on PostgreSQL with the GitHub
//! stand-in, its accounts, revisions, revocations and sent envelopes as in the tests of the cloud
//! limits. Responses are checked against the OpenAPI spec in `api/openapi.json`.
//!
//! The API, as these tests use it, each request stating the protocol version:
//! - `GET /account/usage`, with a web session's cookie: `{"owned_spaces": {"count": <n>, "limit":
//!   <n>, "spaces": [{"space": <ID>, "name": <name>}, ...]}, "devices": {"count": <n>, "limit":
//!   <n>}}`. `owned_spaces.count` counts the spaces the account created, its personal space
//!   included also before it is created, as `--owned-spaces-limit` does; `spaces` lists those that
//!   exist, by name, and asking creates no personal space. `devices.count` counts the account's
//!   devices with a certificate, `service` and revoked devices aside, as `--devices-limit` does.
//!   Each `limit` is the setting's value, absent without the setting. Without a session, or with
//!   an unknown one: `401` with a problem details object;
//! - `GET /spaces/{space}/usage`, with a session of a member in the space's latest stored revision:
//!   `{"monthly_messages": {"count": <n>, "limit": <n>}, "message_size": {"limit": <bytes>},
//!   "mailbox_retention": {"limit": <seconds>}}`. `monthly_messages.count` counts the space's
//!   messages this calendar month in UTC as `--monthly-messages-limit` does, also without the
//!   setting: each message once whatever its envelopes, no retry and no refused one. Each `limit`
//!   is the value of `--monthly-messages-limit`, `--message-size-limit` or `--mailbox-retention`,
//!   absent without it. Another session, also of a removed member, gets a `4xx` with a problem
//!   details object, `401` without a session or with an unknown one.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

#[path = "../common/mod.rs"]
mod common;

// The API calls and web sessions of the tests of sessions on the web.
#[path = "../web_sessions/sessions.rs"]
mod sessions;

// The accounts, spaces, revisions, revocations, sent envelopes and settings of the tests of the
// cloud limits and of revocations on the server.
#[path = "../spaces/api.rs"]
mod api;
#[path = "../mailbox/fixture.rs"]
mod fixture;
#[path = "../invites/scene.rs"]
mod invites;
#[path = "../revocations/kit.rs"]
mod kit;
#[path = "../cloud_limits/limits.rs"]
mod limits;
// Storing the certificate of a device a linked device issued.
#[path = "../spaces/rosters.rs"]
mod rosters;
#[allow(dead_code)]
#[path = "../linked_certificates/upload.rs"]
mod upload;
#[path = "../mailbox/wire.rs"]
mod wire;

mod usage;

mod devices;
mod history;
mod messages;
mod nothing;
mod owned;
mod plan_ownership;
mod plan_roster;
mod plans;
mod settings;
