//! The helper (`docs/dev/architecture.md`, "Components"): on a developer machine one helper process
//! holds the device's connection and the local state's lock, and the client's commands go through
//! it over a local socket.
//!
//! - The first command that connects as its device starts the helper in the background, detached
//!   from the command's terminal; later commands reuse it, so the server sees one connection. A
//!   command with the one-shot flag or setting starts none, but goes through one already running.
//! - Through the helper, a command shows one-shot mode's output and exit code; the helper applies
//!   the rules of agents and folders by the command's working directory and `--agent`.
//! - The helper exits after an idle time and on sign-out. One helper runs per local state; a
//!   command that finds a dead helper starts a new one, and nothing is lost or shown twice.
//! - A helper older than the command exits after the operations in hand, and a new one of the
//!   command's version serves it. A command older than the helper does not use it: it fails with a
//!   message naming both versions, and the helper keeps running.
//! - Only the account that runs the helper reaches its socket, which lives in the local state's
//!   directory.
//!
//! As the tests of channel messages (`channel_messages/main.rs`): the CLI runs on Ada's machine;
//! Bob's device is played by the tests with an MLS state of its own. The fixtures that set Ada's
//! machine up run one-shot, as every CLI test that does not test the helper (`sign_in/machine.rs`).
//! The runs these tests make on Ada's machine run a copy of `skald` in `bin/` under the machine's
//! directory, without the one-shot setting, so the tests find the processes of that copy, the
//! helpers it starts among them, by their executable, and kill each one they leave
//! (`helped.rs`).
//!
//! The CLI, as these tests use it, beside the commands of the tests of channel messages and of
//! agents:
//! - `skald helper` runs the helper; a command starts it in the background as its own executable
//!   with `helper`, so the helper's process runs the command's executable. Its output is not the
//!   command's: the command's outputs close when the command exits;
//! - `--one-shot`, a global flag, and the setting `SKALD_ONE_SHOT=1` ask for one-shot mode;
//! - `SKALD_HELPER_IDLE=<seconds>` sets how long the helper waits idle before it exits;
//! - `SKALD_VERSION=<version>`, for tests, sets the version a run of `skald` acts as, on the
//!   helper's socket and in its messages; a helper a command starts acts as the command's version.
//!   The `skald` on disk keeps its own version, that of the `skald` these tests build;
//! - a command older than the helper fails with a message naming both versions; it says to run the
//!   command `again` when the `skald` on disk is not older than the helper, and to `update` that
//!   `skald` when it is.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod access;
mod agents;
mod crashes;
mod helped;
mod outputs;
mod reuse;
mod routes;
mod versions;

// The server, the machines and the relay of the tests of signing in.
#[path = "../sign_in/cloud.rs"]
mod cloud;
#[path = "../sign_in/machine.rs"]
mod machine;
#[path = "../sign_in/relay.rs"]
mod relay;

// The server's tests' readers of the KeyPackages API's answers, and the person's browser.
#[path = "../../../server/tests/key_packages/answers.rs"]
mod answers;
#[path = "../../../server/tests/common/browser.rs"]
mod browser;

// The helpers of every client test, and finding keys in bytes.
#[path = "../common/mod.rs"]
mod common;
#[path = "../device_keys/keys.rs"]
mod keys;

// The front and fixtures of the tests of sending, and the device the tests of receiving play.
#[path = "../sending/fixtures.rs"]
mod fixtures;
#[path = "../sending/front.rs"]
mod front;
#[path = "../receiving/peer.rs"]
mod peer;

// The gap tests' messages of Bob's device, the kit of the tests of channel groups and the device
// they play, the channel of the tests of channel messages, and people in spaces.
#[path = "../gap_detection/gaps.rs"]
mod gaps;
#[path = "../channel_groups/kit.rs"]
mod kit;
#[path = "../channel_groups/played.rs"]
mod played;
#[path = "../channel_messages/talk.rs"]
mod talk;
#[path = "../roster_revisions/world.rs"]
mod world;
