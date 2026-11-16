//! Revoking a device in the CLI: a user revokes a device of their account, member devices remove
//! it from every channel by a Commit that carries its revocation, and it reads nothing sent after
//! (`docs/protocol/membership.md`, "Revocation", "Device checks" and "Group changes").
//!
//! - The revocation command, on a device with a valid certificate, revokes a device of its account
//!   that has one, itself included: it signs a revocation without `keep`, by this device, and has
//!   the server store it, resending it until the server has taken it.
//! - Each device check of the client applies the revocations the server gives with the
//!   certificates: of revision signers, senders, recipients and admitted devices. A revocation that
//!   does not count changes nothing.
//! - `skald devices` and `skald members` show revoked devices as revoked, and the client warns when
//!   an account's device set changes: a device revoked, or restored by a voided revocation.
//! - A member device that learns of a revocation commits the Remove of the revoked device in each
//!   channel, with the revocations by which it holds it revoked in the Commit's
//!   `authenticated_data`, the encoded `[+ revocation]`; a device that processes the Commit applies
//!   those that count.
//! - A device that finds itself revoked says so, and sends and receives nothing.
//!
//! As the tests of channel messages (`channel_messages/main.rs`): the CLI runs on Ada's machine,
//! which creates the spaces; Bob's and Carol's devices are played by the tests with an MLS state of
//! their own, and their machines run the CLI only for its revocation command. A device the tests
//! play keeps its state as it was when it stops taking in its mailbox, as a full copy of its disk
//! taken then would. A full copy of the disk of Ada's machine is a machine with a copy of every
//! file under its root. A compromised server is played by the front and the relay, which hide or add
//! revocations and put envelopes into a device's catch-up, and by the server's database, which
//! serves a revoked device again. A fixture account whose master key the test holds signs in with
//! a machine that is not its first device; the test sets its master key and writes its devices to
//! the server's database.
//!
//! The CLI, as these tests use it, beside the commands of the tests of channel messages:
//! - `skald revoke-device <device>`, the device's ID: succeeds once the server has stored the
//!   revocation. A revocation the server has not taken waits in the local state, and the next run
//!   that connects, such as `skald recv`, sends it again, with the same bytes. It fails and sends
//!   no revocation for a device of another account, one without a certificate or one the server
//!   does not list, and on a device without a valid certificate;
//! - `skald devices` and `skald members <space>`: a revoked device's line holds its certificate's
//!   `name` and says `revoked`; no other device's line says so;
//! - a warning of a change of an account's device set is a line saying `warning` that holds the
//!   device's ID;
//! - on a revoked device, `skald send`, `skald recv` and `skald create-channel` say `revoked`, and
//!   `skald send` and `skald create-channel` fail and send nothing;
//! - a message it cannot open is dropped with a warning line holding the channel's ID, or the
//!   sending device's ID when the payload authenticates it, and never its text.
//!
//! The server's interfaces, as these tests use them: `POST /account/revocations` and the
//! `revocations` of `GET /devices` and `GET /spaces/{space}/devices`, as the tests of revocations on
//! the server use them; the tables `accounts`, whose `master` a test sets, and `devices`, whose
//! `revoked` a test clears as a compromised server would.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod agent;
mod checks;
mod copy;
mod removal;
mod revocations;
mod revoking;

// The talk of the tests of channel messages.
#[path = "../channel_messages/talk.rs"]
mod talk;

// The kit of the tests of channel groups and the devices they play.
#[path = "../channel_groups/kit.rs"]
mod kit;
#[path = "../channel_groups/played.rs"]
mod played;

// People in spaces, of the tests of roster revisions.
#[path = "../roster_revisions/world.rs"]
mod world;

// The front and fixtures of the tests of sending, and the device the tests of receiving play
// over the delivery protocol.
#[path = "../sending/fixtures.rs"]
mod fixtures;
#[path = "../sending/front.rs"]
mod front;
#[path = "../receiving/peer.rs"]
mod peer;

// The server, the relay and the machines of the tests of signing in.
#[path = "../sign_in/cloud.rs"]
mod cloud;
#[path = "../sign_in/machine.rs"]
mod machine;
#[path = "../sign_in/relay.rs"]
mod relay;

// The helpers of every client test, and finding keys in bytes.
#[path = "../common/mod.rs"]
mod common;
#[path = "../device_keys/keys.rs"]
mod keys;

// The server's tests' readers of the KeyPackages API's answers, and the person's browser.
#[path = "../../../server/tests/key_packages/answers.rs"]
mod answers;
#[path = "../../../server/tests/common/browser.rs"]
mod browser;
