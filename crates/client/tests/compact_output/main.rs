//! Compact output for agents: what an agent reads from `skald recv`, `skald recv --wait` and
//! `skald send` costs as few tokens as it can, and `--json` and the person's `skald recv` keep
//! every field (`docs/protocol/delivery.md`, "Device to agent").
//!
//! - A message's reference `#<ref>`: the leading characters of the base32 of SHA-256 of its
//!   `msg_id`, at least 4, more only where two messages the device keeps would share them; any
//!   device computes it from the `msg_id`.
//! - As an agent, `skald recv` and `skald recv --wait` show a message as one line, with no channel,
//!   device or message ID; one following missing messages is marked `(after missing)`.
//! - As an agent, they show only the statuses of messages this agent sent, and of those only
//!   `expired` and `refused`; each warning is one line without IDs. The device keeps every status
//!   and warning, and the person's `skald recv` shows them all in its full lines.
//! - As an agent, `skald send` prints `sent #<ref>` per stored message; a refusal or a missing
//!   answer prints the full report.
//! - `--json` gives every field, the full IDs and the reference included, as an agent too.
//!
//! As the tests of handing messages over (`agent_handover/main.rs`): the CLI runs on Ada's machine,
//! whose agents run in `~/work/acme`, bound to `acme`; Bob's device is played by the tests with an
//! MLS state of its own and sends the content they write.
//!
//! The CLI, as these tests use it, beside the commands of the tests of handing messages over:
//! - a reference is in RFC 4648's base32 alphabet, in either case and without padding, of
//!   SHA-256 of the `msg_id`'s 16 bytes;
//! - an agent's message line is `[<from> · <device>] #<ref>: <text>`: the content's `from`, the
//!   sending device's certificate `name` and the text, escaped as in the person's lines; one
//!   following missing messages is `[<from> · <device>] #<ref> (after missing): <text>`;
//! - an agent's status line says the status and holds the message's `#<ref>`; a warning, such as
//!   `skald: dropped #<ref> from <device>: <reason>`, names the sending device by its `name`; an
//!   agent's output holds no UUID;
//! - the person's `skald recv` shows each status and warning once, in its full lines, whether or
//!   not an agent's run showed it;
//! - an agent's `skald send` of a stored message prints one line, `sent #<ref>`, and nothing else;
//! - `skald recv --json`, `skald recv --wait --json` and `skald send --json <channel> <text>` print
//!   one JSON object per line on standard output: each message with `msg_id`, `ref`, `channel`,
//!   `from`, `device`, the sending device's ID, `device_name` and `text`; each status with
//!   `msg_id`, `ref`, `channel`, `status` and `device`, the recipient device's ID; each message
//!   `skald send` sent with `msg_id`, `ref`, `channel` and `stored` among its values once the server
//!   has stored it.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod json;
mod lines;
mod messages;
mod said;
mod sending;
mod statuses;

// Ada's agents, as the tests of handing messages over have them, in the channel of the tests of
// channel messages, with the kit and the device of the tests of channel groups, the messages of the
// tests of gap detection, and the device the tests of receiving play over the delivery protocol.
#[path = "../agent_handover/agents.rs"]
mod agents;
#[path = "../gap_detection/gaps.rs"]
mod gaps;
#[path = "../channel_groups/kit.rs"]
mod kit;
#[path = "../receiving/peer.rs"]
mod peer;
#[path = "../channel_groups/played.rs"]
mod played;
#[path = "../channel_messages/talk.rs"]
mod talk;

// The server's tests' readers of the KeyPackages API's answers, their person's browser, and time
// passing as the server's database shows it.
#[path = "../../../server/tests/key_packages/answers.rs"]
mod answers;
#[path = "../../../server/tests/common/browser.rs"]
mod browser;
#[path = "../../../server/tests/common/times.rs"]
mod times;

// The server, the machines and the relay of the tests of signing in, and people in spaces.
#[path = "../sign_in/cloud.rs"]
mod cloud;
#[path = "../sign_in/machine.rs"]
mod machine;
#[path = "../sign_in/relay.rs"]
mod relay;
#[path = "../roster_revisions/world.rs"]
mod world;

// The fixtures and the front of the tests of sending.
#[path = "../sending/fixtures.rs"]
mod fixtures;
#[path = "../sending/front.rs"]
mod front;

// The helpers of every client test, and finding keys in bytes.
#[path = "../common/mod.rs"]
mod common;
#[path = "../device_keys/keys.rs"]
mod keys;
