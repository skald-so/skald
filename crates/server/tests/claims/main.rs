//! Claims, leases and the DLQ on the server (`docs/protocol/queues.md`, "Task states", "Claims and
//! leases", "Fencing", "Modes and retries", "Claim checks" and "Problems"):
//! - a claim gets the first task, in the order the server stored them, that is `queued`, past its
//!   backoff, before its `deadline`, not in the claimer's wait after its release, and that matches
//!   the claim: its `device`, if any, is the claimer; its uploader's trust level for the claimer is
//!   a stated one; its `template` is a stated one, or it has none and the claim takes such tasks;
//! - only a device that passes the server's claim checks claims, extends, completes, fails or
//!   releases: a member device of the channel's current epoch, of kind `client` or `service`, not
//!   revoked, and, for a `service` device, on a token with `queues`; others get `not-a-member`;
//! - an extend, complete, fail or release passes the fencing check, otherwise `claim-lost`, and
//!   changes nothing then; a repeated complete, fail or release gets the first answer; a release
//!   from a `service` device gets `not-allowed`;
//! - tasks move through every row of "Task states" and "Modes and retries", with their attempts,
//!   generation, progress and reason as reads show them; lease ends and deadlines show in reads as
//!   they pass, on any instance;
//! - a `client` device of the uploader's account or of an admin restarts a `dead` task and cancels
//!   one; others get `not-allowed`, a restart of a task in another state `not-dead`, a cancel of a
//!   `done` or `cancelled` one `not-cancellable`;
//! - with the history retention set, a `done`, `dead` or `cancelled` task is deleted once the
//!   retention has passed since it got there;
//! - under load through two instances, no task is held by two claims at once, no `at-most-once`
//!   task is claimed twice and every task ends `done` or `dead`.
//!
//! Against the world of the tests of queues (`../queues/main.rs`): alice, an admin, and carol, a
//! member who is no admin, both in `general`, whose group alice formed with bob's and carol's
//! laptops; bob, another account of the space in the channel; dave, outside the channel; eve,
//! outside the space; and alice's `service` device, the bot, outside the group unless a test adds
//! it. For alice's laptop, a task alice's laptop uploads is `own`, bob's `peer`, the bot's
//! `service`.
//!
//! The interfaces, as these tests use them, besides those of `../queues/main.rs`. Each request
//! proves its device as those do. Refused: `4xx` with a problem details object of the type
//! `https://skald.so/problems/<problem>`.
//! - `POST /spaces/{space}/channels/{channel}/queues/{queue}/claims`, the body `application/json`,
//!   `{"trust_levels": [<"own" | "peer" | "service">], "templates": [<name>],
//!   "without_template": <bool>}`: `200` with the `claim`'s CBOR bytes, `application/cbor`, or
//!   `204` with no body when no task matches.
//! - `POST .../queues/{queue}/tasks/{task}/extend`, the body `application/json`, `{"claim_id":
//!   <claim ID>, "generation": <n>, "progress": "started" | "awaiting-approval"}`; `.../complete`
//!   and `.../release`, `{"claim_id": ..., "generation": ...}`; `.../fail`, `{"claim_id": ...,
//!   "generation": ..., "retry": <bool>}`: `2xx` when taken.
//! - `POST .../queues/{queue}/tasks/{task}/restart` and `.../cancel`, with no body: `2xx` when
//!   taken.
//! - A task as reads give it, `GET .../tasks` and `GET .../tasks/{task}`, as the spec's `Task`:
//!   `progress` while `claimed` after an extend, `reason` while `dead`, and `claim`, `{"claim_id",
//!   "generation", "device"}`, of its last claim.
//! - The server keeps a task's times, such as its lease's end, its backoff's end, each device's
//!   wait after a release and when it got `done`, `dead` or `cancelled`, in `timestamptz` columns
//!   of rows that hold the task's ID, and reads them there: the tests move them back to let time
//!   pass for a task instead of waiting.
//! - Reads show a lease's end or a deadline within 10 seconds, with no request to the task; the
//!   retention deletes within 30 seconds, with no request.
//!
//! Every response is checked against the OpenAPI spec.

// Each module uses a part of the shared ones.
#![allow(dead_code)]

#[path = "../common/mod.rs"]
mod common;

// The world and requests of the tests of queues, with the answers of the tests of presence and
// the bot's tokens.
#[path = "../queues/kit.rs"]
mod kit;
#[path = "../presence/kit.rs"]
mod presence;
#[path = "../service_tokens/tokens.rs"]
mod tokens;

// The certificates of the tests of revocations; the scene of the Delivery Service's tests.
#[path = "../revocations/kit.rs"]
mod revocations;
#[path = "../delivery_service/scene.rs"]
mod scene;

// The fixture and frames of the tests of storing sent envelopes, with the mailbox requests over
// HTTPS, the connections and the streams the world needs.
#[path = "../mailbox/fixture.rs"]
mod fixture;
#[path = "../mailbox_https/https.rs"]
mod https;
#[path = "../mailbox_delivery/link.rs"]
mod link;
#[path = "../mailbox_sse/sse.rs"]
mod sse;
#[path = "../mailbox/wire.rs"]
mod wire;

// The spaces tests' accounts and revisions, and the bot of the tests of service requests.
#[path = "../spaces/api.rs"]
mod api;
#[path = "../service_requests/bot.rs"]
mod bot;
#[path = "../spaces/rosters.rs"]
mod rosters;

mod holder;

mod checks;
mod edges;
mod fencing;
mod instances;
mod load;
mod matching;
mod restarts;
mod retention;
mod states;
mod waits;
