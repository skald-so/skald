//! The local state's lock: a run that changes the local state holds its file lock for the whole
//! run; another such run on the same state waits for it up to the request timeout, then fails,
//! naming the lock (`docs/dev/architecture.md`, "Cryptography", "Local state"). The lock is an OS
//! file lock, so it frees when its process dies.
//!
//! As the tests of channel messages (`channel_messages/main.rs`): the CLI runs on Ada's machine;
//! Bob's device is played by the tests with an MLS state of its own. To keep a run holding the lock,
//! the front before the relay holds back the `auth` of Ada's device, so the run waits for the
//! server with the lock held; a run that waits for the lock reaches the server with no `auth`.
//!
//! The CLI, as these tests use it, beside the commands of the tests of channel messages:
//! - `--timeout <seconds>` sets a run's request timeout, which also bounds its wait for the lock;
//! - a run that waits for the lock past its timeout fails with a message that says `lock`, and
//!   changes nothing: what it was to send is never sent.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod beside;
mod commands;
mod held;
mod sends;

// The front and fixtures of the tests of sending, and the device the tests of receiving play.
#[path = "../sending/fixtures.rs"]
mod fixtures;
#[path = "../sending/front.rs"]
mod front;
#[path = "../receiving/peer.rs"]
mod peer;

// The tests of channel messages' channel and the gap tests' messages of Bob's device, with the
// kit of the tests of channel groups and the device they play.
#[path = "../gap_detection/gaps.rs"]
mod gaps;
#[path = "../channel_groups/kit.rs"]
mod kit;
#[path = "../channel_groups/played.rs"]
mod played;
#[path = "../channel_messages/talk.rs"]
mod talk;

// The machines, the relay and the server of the tests of signing in, and people in spaces.
#[path = "../sign_in/cloud.rs"]
mod cloud;
#[path = "../sign_in/machine.rs"]
mod machine;
#[path = "../sign_in/relay.rs"]
mod relay;
#[path = "../roster_revisions/world.rs"]
mod world;

// The person's browser and the server's tests' readers of the KeyPackages API's answers.
#[path = "../../../server/tests/key_packages/answers.rs"]
mod answers;
#[path = "../../../server/tests/common/browser.rs"]
mod browser;

// The helpers of every client test, and finding keys in bytes.
#[path = "../common/mod.rs"]
mod common;
#[path = "../device_keys/keys.rs"]
mod keys;
