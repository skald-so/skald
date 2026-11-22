//! Erasing a deleted account's data, in the tables the tests of erasing an account's data
//! (`account_erasure/main.rs`) leave empty: the account's devices' waiting messages, counted
//! messages, link runs, challenges, wrong codes and pairings, acceptances, tasks, claims and
//! queues, the queue key objects and the wraps its devices signed. Against a `skald-server` on
//! PostgreSQL with the GitHub stand-in, the scene of the tests of a channel's history.
//!
//! The interfaces, as these tests use them, besides those of the tests of erasing an account's
//! data:
//! - `accounts.erased` in the database is whether the server has erased the deleted account, and
//!   `accounts.created_at` and `spaces.creator_id` its sign-up time and the space's creator;
//! - rows of the tables no request of these tests fills are written to the database directly.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

#[path = "../common/mod.rs"]
mod common;

// The modules of the tests of erasing an account's data (`account_erasure/main.rs`).
#[path = "../spaces/api.rs"]
mod api;
#[path = "../account_deletion/deletion.rs"]
mod deletion;
#[path = "../account_erasure/erasure.rs"]
mod erasure;
#[path = "../mailbox/fixture.rs"]
mod fixture;
#[path = "../channel_history/kit.rs"]
mod history;
#[path = "../revocations/kit.rs"]
mod kit;
#[path = "../mailbox_delivery/link.rs"]
mod link;
#[path = "../mailbox_delivery/listener.rs"]
mod listener;
#[path = "../anchors/objects.rs"]
mod objects;
#[path = "../spaces/rosters.rs"]
mod rosters;
#[path = "../delivery_service/scene.rs"]
mod scene;
#[path = "../web_sessions/sessions.rs"]
mod sessions;
#[path = "../mailbox/wire.rs"]
mod wire;

mod rows;
