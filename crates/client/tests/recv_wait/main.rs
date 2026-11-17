//! `skald recv --wait` for any agent with a shell, and sessions through the helper
//! (`docs/dev/architecture.md`, "Components", "Network" and "Delivery to agents";
//! `docs/protocol/delivery.md`, "Device to agent").
//!
//! - `skald recv --wait` shows what is waiting and exits; with nothing waiting, it waits for the
//!   first to come, shows it and exits, or waits until interrupted. As an agent it waits for a
//!   handover; as the person, for anything the person's `skald recv` shows: a message, a status or
//!   a warning.
//! - It goes through the helper, or in one-shot mode holds its own connection and takes the local
//!   state's lock only to write, so a run beside it neither waits nor loses a change. When a helper
//!   starts meanwhile, it goes on waiting through it, unless the helper is newer: then it ends,
//!   saying to run it again, and the helper and its sessions keep running.
//! - The helper hands each message to the waiting sessions of the agents it is for as it arrives,
//!   and marks it handed over only once the session has written it out; a session that dies first
//!   leaves it for the next. A helper that exits for a newer command ends its waiting sessions as a
//!   dead helper does: they fail.
//! - On SSE or long-poll, the helper and a one-shot `skald recv --wait` try WebSocket again after
//!   an interval, then the other transports in order.
//!
//! As the tests of the helper (`helper/main.rs`): the CLI runs on Ada's machine; Bob's device is
//! played by the tests with an MLS state of its own. The runs that may go through a helper run the
//! machine's own copy of `skald`, whose processes the tests find by their executable. The tests
//! behind a proxy run as the tests of the transports' fallback (`transport_fallback/main.rs`).
//!
//! The CLI, as these tests use it, beside the commands of the tests of the helper and of handing
//! messages over to agents:
//! - `skald recv --wait`, as the person or with `--agent <name>` as an agent, which shows each
//!   message as the tests of compact output say (`compact_output/main.rs`);
//! - a session exits once the helper has marked what it showed handed over, so its next run never
//!   shows it again;
//! - `SKALD_WEBSOCKET_RETRY=<seconds>` sets how long a connection on SSE or long-poll waits before
//!   it tries WebSocket again; a helper takes it from the command that starts it.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod crashes;
mod one_shot;
mod proxied;
mod sessions;
mod versions;
mod waits;

// Ada's machine and its own copy of `skald`, as the tests of the helper run it; Ada's agents, and a
// run as an agent whose output a test stops reading, as the tests of handing messages over have
// them; what an agent reads, as the tests of compact output have it; runs that hold the lock, of
// the tests of the lock.
#[path = "../agent_handover/agents.rs"]
mod agents;
#[path = "../agent_handover/held.rs"]
mod held;
#[path = "../helper/helped.rs"]
mod helped;
#[path = "../compact_output/lines.rs"]
mod lines;
#[path = "../state_lock/held.rs"]
mod locked;
#[path = "../sign_in/machine.rs"]
mod machine;

// The channel of the tests of channel messages, Bob's device in it as the tests of channel groups
// and of gap detection play it, the server, its relay and its front, people in spaces, and the
// fixtures of the tests of sending.
#[path = "../sign_in/cloud.rs"]
mod cloud;
#[path = "../sending/fixtures.rs"]
mod fixtures;
#[path = "../sending/front.rs"]
mod front;
#[path = "../gap_detection/gaps.rs"]
mod gaps;
#[path = "../channel_groups/kit.rs"]
mod kit;
#[path = "../receiving/peer.rs"]
mod peer;
#[path = "../channel_groups/played.rs"]
mod played;
#[path = "../sign_in/relay.rs"]
mod relay;
#[path = "../channel_messages/talk.rs"]
mod talk;
#[path = "../roster_revisions/world.rs"]
mod world;

// Ada's machine behind the proxy of the tests of the transports' fallback, the proxy's filter, the
// recording proxy of the tests of `HTTPS_PROXY`, and the test CA and TLS endpoint of the tests of
// HTTPS.
#[path = "../transport_fallback/behind.rs"]
mod behind;
#[path = "../transport_fallback/filter.rs"]
mod filter;
#[path = "../https_proxy/proxy.rs"]
mod proxy;
#[path = "../common/tls.rs"]
mod tls;
#[path = "../https/front.rs"]
mod tls_front;

// The person's browser, time passing as the server's database shows it, the server's tests'
// readers of the KeyPackages API's answers, finding keys in bytes, and the helpers of every client
// test.
#[path = "../../../server/tests/key_packages/answers.rs"]
mod answers;
#[path = "../../../server/tests/common/browser.rs"]
mod browser;
#[path = "../common/mod.rs"]
mod common;
#[path = "../device_keys/keys.rs"]
mod keys;
#[path = "../../../server/tests/common/times.rs"]
mod times;
