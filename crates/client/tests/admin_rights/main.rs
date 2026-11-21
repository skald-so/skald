//! Admin rights in the CLI: an admin's device makes a member a second admin, or hands its account's
//! admin rights to a member, each by the next roster revision it signs; every member device shows
//! the change, and the new admin's revisions count. On a device of the only admin of a space with
//! another member, `skald members` and `skald spaces` warn and name the command that makes a member
//! an admin. A device linked with the recovery key has its account's admin rights
//! (`docs/protocol/membership.md`, "Roster"; `architecture.md`, "Membership").
//!
//! - Each command signs the next revision as `skald remove-member` does: it first accepts the
//!   revisions the server has, applies its change to the latest, writes the revision before sending
//!   it and resends it until the server has taken it. On a device of a member that is no admin it
//!   refuses and sends nothing. Handing over is one revision: the member an admin, if it is not one,
//!   and this account a member.
//! - A member device that accepts a revision changing a member's role tells its user once: the
//!   member's handle, its new role and the handle of the signing device's account.
//!
//! As the tests of removing members (`member_removal/main.rs`): a device follows only the spaces it
//! created until invites give it others, so the CLI under test runs on the machine that creates the
//! space; the other people's devices are played by the tests, which sign fixture revisions with
//! their devices' keys and store them through the API, or, in a channel's group, with an MLS state
//! of their own. A device linked with the recovery key is linked as in the tests of linking with it
//! (`recovery_linking/main.rs`), at a terminal, and starts from the anchors of the device it names
//! lost. Whether a played device accepts a revision is checked with `skald-crypto`, as its own
//! client would, against the certificates the server gives its session.
//!
//! The CLI, as these tests use it, beside the commands of the tests of removing members, each on a
//! space by its name and a member by its handle:
//! - `skald make-admin <space> <handle>` makes the member an admin;
//! - `skald hand-over-admin <space> <handle>` makes the member an admin and this device's account a
//!   member, in one revision;
//! - each succeeds once the server has taken its revision; on a device of a member that is no admin
//!   it fails, saying `admin`, and sends nothing; run with `--agent <name>`, it fails, saying
//!   `skald <command>`, `person's command` and `an agent cannot run`, and sends nothing;
//! - the run in which a device accepts a revision that changes a member's role, the device's own
//!   included, shows one line that holds, as words, the member's handle, its new role as `skald
//!   members` shows it, `admin` or `member`, and the handle of the account of the device that signed
//!   it; no later run on the device shows such a line again, and no other line holds all three;
//! - on a device of the only admin of a space with another member, `skald members <space>` and
//!   `skald spaces` show a warning, a line saying `warning` that holds `admin`, `skald spaces`'s
//!   holding the space's name, and their output holds `recovery key` and `make-admin`; neither
//!   holds `make-admin` for a space with another admin or with no other member, or on a device of
//!   a member that is no admin.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod handover;
mod lost;
mod recovery;
mod refusals;
mod roles;
mod second;
mod told;
mod warning;

// The modules the tests of the channel history share (`history_common/mod.rs`), among them those of
// the tests of removing members, and the terminal and linking with the recovery key of the tests of
// the recovery key and of linking with it.
#[path = "../history_common/mod.rs"]
mod history_common;
use history_common::*;
#[path = "../recovery_key/at_terminal.rs"]
mod at_terminal;
#[path = "../recovery_key/grants.rs"]
mod grants;
#[path = "../recovery_linking/recovering.rs"]
mod recovering;
#[path = "../recovery_key/terminal.rs"]
mod terminal;
