//! Channel groups in the CLI: a channel's admitted devices form its MLS group and keep it in line
//! with the roster revisions they accept (`docs/protocol/membership.md`, "Admitted devices",
//! "Creating a channel", "Commit ordering" and "Group changes"; `architecture.md`, "Membership").
//!
//! - A device admitted to a channel that has no group on the server yet creates it: it adds each
//!   admitted device it gets a KeyPackage for, after the KeyPackage checks, and commits; once the
//!   Commit is stored, it uploads the GroupInfo, then sends the Welcome. Refused with
//!   `epoch-mismatch`, it drops its group and joins the kept one from its Welcome. `skald
//!   create-channel` does so at once.
//! - The default channel, the first one the roster lists: an admin's device that accepts a revision
//!   of a space with no channel, or of a space whose first channel lacks a member, signs the next
//!   revision with a new channel first, every member in it with `history` true, and creates its
//!   group; every revision it signs keeps the default channel first.
//! - `skald recv`, and `skald send` before it sends, go through the mailbox in order and process
//!   each channel's Welcomes and Commits with the checks of "Group changes"; `skald send` shows none
//!   of the entries it writes, which a later `skald recv` shows.
//! - After processing a Commit or accepting a revision, a member device commits the Remove of each
//!   member that is not admitted, and warns when another device's Commit added one. It commits the
//!   Adds of a new channel member's devices when it signed the revision.
//! - Own Commits go out from the outbox with the same bytes until `stored`; then the device merges
//!   and uploads the new epoch's GroupInfo. A member device uploads the GroupInfo when the server
//!   has none of the current epoch.
//!
//! Against a `skald-server` with GitHub sign-in through the GitHub stand-in, on PostgreSQL, behind
//! the recording relay and, before it, the front of the tests of sending (`sending/front.rs`), at
//! the server's public address. A test plays the person on the verification page in headless
//! Chrome.
//!
//! A device follows only the spaces it created until invites and linking give it others, so the
//! CLI under test runs on Ada's machine, which creates the spaces. The other people's devices are
//! played by the tests as their own clients would, with their machines' keys and an MLS state of
//! the tests' own (`skald-crypto`): each one's KeyPackages on the server are replaced by ones it
//! made, and it reads, acks and sends `mls` envelopes over the delivery protocol. The tests sign
//! fixture revisions with the devices' keys and store them through the API, as an admin's device
//! would. A device of no account, which no revision admits, is one the tests make up.
//!
//! A space as one made before default channels, with revision 1 only: the server's database refuses
//! every later revision while `skald create-space` runs, and the test then empties the space's
//! `pending` revision in the local state, as a client of that time left it.
//!
//! The CLI, as these tests use it, beside the commands of the tests of roster revisions, of
//! creating channels, of sending and of receiving:
//! - `skald channels <space>`: one line per channel, holding its ID, its members' handles and the
//!   certificate `name` of each device in its group as the device holds it; a channel the device is
//!   admitted to but whose group it is not in says `not in` on its line;
//! - a warning about a device a Commit added that is not admitted, and about a Welcome refused for
//!   its signer, is a line saying `warning` that holds that device's ID;
//! - `skald --agent <name> recv`, in a folder bound to a space, names nothing of other spaces in
//!   its output; a later run of the person shows those warnings.
//!
//! The server's interfaces, as these tests use them: `GET` and `PUT
//! /spaces/{space}/channels/{channel}/group-info`, the KeyPackages API of the tests of KeyPackages,
//! and the table `key_packages`, whose rows of a device a test deletes before the device it plays
//! uploads its own.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod agent;
mod creating;
mod default_channel;
mod kit;
mod membership;
mod played;
mod retries;

// The server's tests' readers of the KeyPackages API's answers, and the person's browser.
#[path = "../../../server/tests/key_packages/answers.rs"]
mod answers;
#[path = "../../../server/tests/common/browser.rs"]
mod browser;

// The server, the relay and the machines of the tests of signing in.
#[path = "../sign_in/cloud.rs"]
mod cloud;
#[path = "../sign_in/machine.rs"]
mod machine;
#[path = "../sign_in/relay.rs"]
mod relay;

// People in spaces, the front and fixtures of the tests of sending, and the device the tests of
// receiving play over the delivery protocol.
#[path = "../sending/fixtures.rs"]
mod fixtures;
#[path = "../sending/front.rs"]
mod front;
#[path = "../receiving/peer.rs"]
mod peer;
#[path = "../roster_revisions/world.rs"]
mod world;

// Finding keys in bytes, and the helpers of every client test.
#[path = "../common/mod.rs"]
mod common;
#[path = "../device_keys/keys.rs"]
mod keys;
