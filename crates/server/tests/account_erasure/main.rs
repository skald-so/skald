//! Erasing a deleted account's data: 30 days after an account deleted itself, whether or not every
//! channel has removed its devices, the server erases the account and everything of it, but for
//! what the spaces it shared need: for each space with other members whose revisions list the
//! account, those revisions, the history entries its devices wrote, its devices' certificates and
//! revocations, which members check them with (`docs/protocol/membership.md`, "Roster", "History
//! entry"), and that it was deleted. Those go with the last such space. A space it created that has
//! other members counts against no account's owned spaces. Against a `skald-server` on PostgreSQL
//! with the GitHub stand-in, its accounts, spaces, revisions, devices and channel as in the tests of
//! account deletion and of a channel's history.
//!
//! The interfaces, as these tests use them, besides those of the tests of account deletion:
//! - `accounts.deleted_at` in the database is when the account deleted itself: the tests move it
//!   back to let time pass, as the tests of web sessions move a sign-in.
//! - The server erases a deleted account within 30 seconds of its deletion being 30 days old, with
//!   no request of any session, and nothing of it while the deletion is younger.
//! - What the tests take for the account's data, each read from the server before the deletion:
//!   its ID and its devices' IDs, as text and as the hex of their bytes; its username, from `GET
//!   /session`; its GitHub user ID, its sign-in method's subject; its master public key, account
//!   record and master backup, from `GET /account/master`; its anchors objects, from `GET
//!   /account/anchors`; and the history wraps to its recovery key.
//! - What the tests take for what a shared space keeps: the space's revisions, from `GET
//!   /spaces/{space}/revisions`; the account's devices' certificates and revocations, from `GET
//!   /spaces/{space}/devices`; the history entries its devices uploaded; and the mark that it was
//!   deleted, as `GET /spaces/{space}/members` and `GET /spaces/{space}/devices` give it.
//! - Every row of every table of the database, read as PostgreSQL's text form of the row: a row
//!   that holds none of what a shared space keeps holds nothing of the account's data, but its ID,
//!   either beside the ID of a space that keeps it, as the revisions' own records do, or in one row
//!   at most, the mark that it was deleted. No row holds its username, its subject, its account
//!   record, its anchors objects or a wrap to its recovery key; only a row holding a revision holds
//!   its master public key.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

// The modules the tests of erasing an account's data share (`erasure_common/mod.rs`).
#[path = "../erasure_common/mod.rs"]
mod erasure_common;
use erasure_common::*;

mod before;
mod erased;
mod kept;
mod last;
