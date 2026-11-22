//! Service agents in the CLI: an admin of a space, at a `client` device of the account and as the
//! person, creates a service agent such as `ci/deploy-bot` (`docs/protocol/membership.md`, "Roster";
//! `docs/protocol/README.md`, "Service grant"). The run generates the service
//! device's keys, registers it as a `service` device of the admin's account, stores its
//! certificate, issued by the admin's device, signs and stores a roster revision with the agent's
//! entry, signs a grant and creates the agent's first token with it. It prints the key secret and
//! the token once, on stdout, and keeps nothing of them: neither its local state, its key storage
//! nor its logs hold them or the service device's private keys. The key secret holds the service
//! device's private keys, the admin's master key, the space, the hash of the revision with the
//! entry and the server's public address. A creation that races another revision signs on the new
//! one. New tokens revoke the agent's earlier ones unless they are kept; tokens are listed and
//! revoked in the CLI.
//!
//! Against a `skald-server` with GitHub sign-in through the GitHub stand-in, on PostgreSQL, behind
//! the recording relay and, before it, the front of the tests of sending at the server's public
//! address, `http://localhost:<port>`; a test plays the person on the verification page in headless
//! Chrome, and plays the other members' devices, checking revisions and certificates with the
//! protocol's crates. Each machine keeps its state in a directory of its own and its secrets in a
//! file there. The tests sign fixture revisions with the keys of the machines' devices, read from
//! their secrets files, and store them through the API, as an admin's device would. Tokens are
//! checked through the server's API (`api/openapi.json`): `GET /service-token` with a token gives
//! its record and grant, or `401` once it is revoked; `GET /spaces/{space}/service-tokens` lists
//! the space's live tokens.
//!
//! The CLI, as these tests use it, beside the commands of the tests of roster revisions:
//! - `skald create-service-agent <space> <owner>/<agent> --channel <channel>...`: creates the
//!   service agent with a grant for the channels, by their IDs. `--expires <days>d` or an RFC 3339
//!   date-time sets the grant's expiry, 90 days by default; `--permission send|receive|queues`,
//!   once per permission, sets the token's, `send` and `receive` by default. On success, stdout is
//!   exactly the two lines `SKALD_SERVICE_KEY=<key secret>` and `SKALD_SERVICE_TOKEN=<token>`, in
//!   either order. Refused, with nothing registered, signed or stored and nothing on stdout, saying
//!   why: by an account that is no admin of the space, saying `admin`; by an agent, as the person's
//!   command, as the tests of agent isolation say; for an address the space's latest revision has,
//!   as a service agent's or as a member's handle; for an expiry in the past, saying `expir`; in a
//!   space with as many service agents as the server's `service_agents_limit` allows, saying `as
//!   many service agents as it may`.
//! - `skald create-service-token <space> <owner>/<agent> --channel <channel>...`: a new token of
//!   the service agent with a new grant, the options as above; it revokes the agent's earlier
//!   tokens unless `--keep-earlier` is given. On success, stdout is exactly the line
//!   `SKALD_SERVICE_TOKEN=<token>`. An agent cannot run it.
//! - `skald service-tokens <space>`: one line per live token of the space, holding its ID and its
//!   service agent's address.
//! - `skald revoke-service-token <space> <token ID>`: revokes the token. An agent cannot run it.
//! - `skald --agent <name> <command>`: runs the command as the agent, as the tests of agent
//!   isolation run it.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

// The sign-in tests' server, machines and relay.
#[path = "../../../server/tests/common/browser.rs"]
mod browser;
#[path = "../sign_in/cloud.rs"]
mod cloud;
#[path = "../common/mod.rs"]
mod common;
#[path = "../sign_in/machine.rs"]
mod machine;
#[path = "../sign_in/relay.rs"]
mod relay;

// The sending tests' front and fixtures, the device key tests' search for keys and the roster
// revision tests' people and spaces.
#[path = "../sending/fixtures.rs"]
mod fixtures;
#[path = "../sending/front.rs"]
mod front;
#[path = "../device_keys/keys.rs"]
mod keys;
#[path = "../roster_revisions/world.rs"]
mod world;

mod kit;

mod creating;
mod races;
mod refusals;
mod tokens;
