//! Scheduled rotation of the history key in the CLI: no channel keeps one epoch, and so one
//! history key, longer than its space's policy allows, and an admin's device sets that period
//! (`docs/protocol/membership.md`, "Policy" and "History key").
//!
//! - A member device commits an empty Commit, an UpdatePath and no proposals, in each channel whose
//!   current epoch is older than the policy's `history` in the latest revision it accepted: after
//!   its catch-up and before it sends in the channel. It is an own Commit: it goes out from the
//!   outbox with the same bytes until `stored`; after `epoch-mismatch` the device processes the
//!   stored Commit and commits again only if the new epoch is too old as well.
//! - `skald set-history-period`, on a device of an admin of the space, signs the next revision with
//!   the policy's `history` as `skald create-channel` signs one; a period under a day is refused, and
//!   on a device of a member that is no admin it refuses. A refused command sends nothing.
//!
//! As the tests of channel messages (`channel_messages/main.rs`): the CLI runs on Ada's machine,
//! which creates the spaces, Ada an admin; Bob's device, a member, is played by the tests with an
//! MLS state of its own; the tests sign fixture revisions with the devices' keys. For a CLI on the
//! device of a member that is no admin, Bob's machine creates the space and its device signs the
//! revision that makes Ada an admin and Bob a member.
//!
//! The CLI, as these tests use it, beside the commands of the tests of creating channels and of
//! channel messages:
//! - `skald set-history-period <space> <period>`, the space by its name and the period as whole
//!   days, `<n>d`, or whole hours, `<n>h`, sets the policy's `history` to that many seconds; it
//!   succeeds once the server has taken its revision; under a day it fails, saying `day`; on a
//!   device of a member that is no admin it fails, saying `admin`;
//! - `SKALD_CLOCK_OFFSET`, a setting for tests: seconds the CLI adds to the system's clock, by which
//!   it counts an epoch's age, so a test ages an epoch without waiting for days.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod aged;
mod period;
mod races;
mod rotation;

// The modules the tests of the channel history share (`history_common/mod.rs`).
#[path = "../history_common/mod.rs"]
mod history_common;
use history_common::*;
