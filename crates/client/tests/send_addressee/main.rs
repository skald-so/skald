//! The addressee and the channel of `skald send` (`docs/protocol/README.md`, "Agent address" and
//! "Content").
//!
//! - `skald send (--to <address> | --all) [--channel <channel>] <text>`: exactly one of `--to` and
//!   `--all`. Neither or both fails, naming both, and sends nothing.
//! - `--to` sets the content's `to`; `--all` leaves it out, for every agent in the channel.
//! - `--to` takes only an agent address: full, or `owner/agent`, which the CLI completes with the
//!   channel's space and the server's name; input is lowercased. A person's address fails, saying
//!   that only a reply goes to a person; so does an invalid address and one of another space or
//!   server. Each sends nothing.
//! - Without `--channel`, the message goes to the default channel of the working directory's space,
//!   for an agent and for the person; a folder in no space sends nothing and says so.
//! - The help of `--to` says that an addressee does not make a message private.
//! - Every `skald send` in the README and the docs names its addressee.
//!
//! As the tests of channel messages (`channel_messages/main.rs`): the CLI runs on Ada's machine,
//! which creates `clienta` and `clientb`; Bob's device is played by the tests with an MLS state of
//! its own, in the groups of both spaces' channels, and reads the content of each message there.
//! Ada's agents run in `~/work/clientA`, bound to `clienta`. Bob's agents are the addresses of his
//! account in each space; what his device hands to them follows from each content's `to`, as the
//! tests of handing messages over have it (`agent_handover/main.rs`).

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod addressees;
mod clients;
mod default_channel;
mod docs;
mod help;
mod refusals;

// The server of the tests of signing in, finding keys in bytes, and the tests of channel groups'
// kit.
#[path = "../sign_in/cloud.rs"]
mod cloud;
#[path = "../device_keys/keys.rs"]
mod keys;
#[path = "../channel_groups/kit.rs"]
mod kit;

// The server's tests' readers of the KeyPackages API's answers, the device the tests of channel
// groups play, and the channel of the tests of channel messages.
#[path = "../../../server/tests/key_packages/answers.rs"]
mod answers;
#[path = "../channel_groups/played.rs"]
mod played;
#[path = "../channel_messages/talk.rs"]
mod talk;

// The helpers of every client test, and the machines of the tests of signing in.
#[path = "../common/mod.rs"]
mod common;
#[path = "../sign_in/machine.rs"]
mod machine;

// The person's browser, the device the tests of receiving play over the delivery protocol, and the
// relay of the tests of signing in.
#[path = "../../../server/tests/common/browser.rs"]
mod browser;
#[path = "../receiving/peer.rs"]
mod peer;
#[path = "../sign_in/relay.rs"]
mod relay;

// The fixtures and the front of the tests of sending, and people in spaces.
#[path = "../sending/fixtures.rs"]
mod fixtures;
#[path = "../sending/front.rs"]
mod front;
#[path = "../roster_revisions/world.rs"]
mod world;
