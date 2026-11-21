//! Spaces joined on another device of the account (`docs/protocol/membership.md`, "Anchors" and
//! "Roster"): a `client` device also starts a space it does not follow from the anchors objects of
//! its account's other `client` devices with a valid certificate (checks 1–4 of "Device checks"),
//! from the anchor with the lowest revision number, every other anchor of the space being a
//! revision it so accepts; otherwise it warns its user and does not follow the space. On a run that
//! connects, and on a `space-notice` of a space it does not follow, the CLI takes the account's
//! anchors objects for each space the account's spaces list names that it does not follow, follows
//! the ones it can and gets into their channels. As an agent, its output names nothing of other
//! spaces: their warnings wait for the person's next run.
//!
//! Ada's first machine signs up and creates spaces; her second machine is linked by hand, as in the
//! tests of the history on new devices (`history_devices/main.rs`), before the spaces it is to
//! follow exist, so it follows only what her first machine followed then. A third device of her
//! account, linked by hand too and never run, signs the anchors objects a compromised server adds,
//! as the tests of linking with the recovery key do (`recovery_linking/anchoring.rs`). Against a
//! `skald-server` with GitHub sign-in through the stand-in, behind the front and the recording
//! relay of the tests of channel messages, which the tests use as a proxy that sees, withholds,
//! changes and adds what goes through.
//!
//! The CLI, as these tests use it, beside the commands of the tests of spaces, channels, receiving
//! and reading the history:
//! - `skald recv`, a run that connects, follows each space it can start from its account's anchors
//!   objects, and gets into the space's channels in the same run; it does so too on a
//!   `space-notice` of a space it does not follow that comes while it syncs;
//! - `skald recv --wait`, as the person, one-shot or through the helper, does so on a
//!   `space-notice` of a space it does not follow, with no other command;
//! - `skald spaces` lists the spaces the device follows, one line each, holding the space's name; a
//!   warning is a line saying `warning`, and the one of a space it does not follow, as its anchors
//!   disagree, holds the space's name or ID;
//! - a device that follows a space signs an anchors object naming it (`anchors/main.rs`).
//!
//! The server's API, as these tests use it besides that of the tests of anchors objects and roster
//! revisions: `GET /account/anchors`, which the relay withholds, refuses, or adds objects to.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod agents;
mod following;
mod joined;
mod refusals;
mod waiting;

// The modules the tests of the channel history share (`history_common/mod.rs`), the second
// machines of the tests of the history on new devices, the helpers of the tests of reading the
// history, the anchors objects of the tests of anchors, and the runs through a helper of the tests
// of the helper.
#[path = "../history_common/mod.rs"]
mod history_common;
use history_common::*;
#[path = "../anchors/objects.rs"]
mod anchors;
#[path = "../history_devices/devices.rs"]
mod devices;
#[path = "../helper/helped.rs"]
mod helped;
#[path = "../history_reading/reading.rs"]
mod reading;
