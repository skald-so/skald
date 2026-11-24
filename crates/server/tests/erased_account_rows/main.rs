//! Erasing a deleted account's data, in the tables the tests of erasing an account's data
//! (`account_erasure/main.rs`) leave empty: the account's devices' waiting messages, counted
//! messages, link runs, challenges, wrong codes and pairings, acceptances, sign-up reservations,
//! tasks, claims and queues, the queue key objects and the wraps its devices signed, and the
//! long-polls of a space that removed it before its deletion. Report rows still within retention
//! are checked after separate deletions of a reporter and a target. Against a `skald-server` on
//! PostgreSQL with the GitHub stand-in, the scene of the tests of a channel's history and the team
//! of the spaces tests.
//!
//! The interfaces, as these tests use them, besides those of the tests of erasing an account's
//! data:
//! - `accounts.erased` in the database is whether the server has erased the deleted account, and
//!   `accounts.created_at` and `spaces.creator_id` its sign-up time and the space's creator;
//! - rows of the tables no request of these tests fills are written to the database directly.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

// The modules the tests of erasing an account's data share (`erasure_common/mod.rs`).
#[path = "../erasure_common/mod.rs"]
mod erasure_common;
use erasure_common::*;

// The requests over HTTPS of the tests of the mailbox over HTTPS.
#[path = "../mailbox_https/https.rs"]
mod https;

// The locks that the tests of account deletion hold.
#[path = "../account_deletion/held.rs"]
mod held;

#[path = "../abuse_reports/reporting.rs"]
mod reporting;

mod findings;
mod removed;
mod reports;
mod roster;
mod rows;
