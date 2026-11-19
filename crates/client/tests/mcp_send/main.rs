//! The MCP server's send tool (`docs/protocol/README.md`, "Agent address" and "Content"): an agent
//! with MCP sends a message to one agent or to everyone in a channel, as `skald send` does.
//!
//! - Its required field `to` takes an agent address, full or `owner/agent`, or `*` for everyone,
//!   which sends the content without `to`. A call without `to`, with a person's address, an invalid
//!   one or one of another space or server sends nothing and says why.
//! - The channel is optional: without it, the message goes to the default channel of the session's
//!   folder's space. A channel of another space is refused, and the result names nothing of it; in
//!   a folder in no space the tool says the agent sees no space.
//! - It returns `sent #<ref>` once the server stored the message, or the refusal as `skald send`
//!   gives it to an agent.
//! - The description of `to` in `tools/list` says that an addressee does not make a message
//!   private, and the answer stays within the bound the tests of `inbox` set.
//!
//! As the tests of the addressee of `skald send` (`send_addressee/main.rs`): the CLI runs on Ada's
//! machine, which creates `clienta` and `clientb`, each with a channel whose group holds Bob's
//! played device, and reads the content of each message there; `~/work/clientA` is bound to
//! `clienta`. Bob's agents are the addresses of his account in each space. The tests play the agent
//! CLI with the MCP client of the tests of `inbox` (`mcp_inbox/client.rs`), which starts
//! `skald --agent <ADA_AGENT> mcp` in a folder of Ada's machine.
//!
//! The CLI, as these tests use it, beside the commands of those tests:
//! - the tool `send` takes the arguments `to`, a string, `text`, the message's text, and
//!   optionally `channel`, a channel's ID as `skald send --channel` takes it; its result's text is
//!   what the agent's `skald send` prints: `sent #<ref>` for a stored message, or the refusal;
//! - its input schema in `tools/list` lists `to` among its required fields and not `channel`, and
//!   the description of its property `to` says `private`.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod addressees;
mod channels;
mod refusals;
mod results;
mod tools;

use std::path::Path;

use serde_json::Value;
use skald_proto::{ChannelId, MessageId};

use crate::client::Session;
pub use crate::client::{
    CHANNEL_FIELD, CLIENT, EVERYONE, SEND_TOOL, TEXT_FIELD, TO_FIELD, TOOLS_BOUND,
};
use crate::clients::{ADA_AGENT, Clients};
use crate::lines::{assert_says_sent, says_sent};
use crate::talk::one_message_sent;
use crate::world::{AGENT, CLIENT_A, HOST};

/// Starts `skald --agent <ADA_AGENT> mcp` on Ada's machine in `dir`, initialized as [`CLIENT`].
pub fn session_in(clients: &Clients, dir: &Path) -> Session {
    client::session_in(clients.ada(), dir, &[AGENT, ADA_AGENT], CLIENT)
}

/// The address of the agent the sessions run as, in `clienta`.
pub fn ada_agent(clients: &Clients) -> String {
    format!("{CLIENT_A}/{}/{ADA_AGENT}@{HOST}", clients.talk.ada.handle)
}

/// The `from` of every message Bob's device read so far in `channel`, in text form.
pub fn senders_read(clients: &mut Clients, channel: ChannelId) -> Vec<String> {
    (clients.bob_reads(channel).into_iter())
        .map(|read| read.from)
        .collect()
}

/// Calls the send tool with `arguments` from `session`; fails unless the call is no error, the front
/// saw one message in `channel` after it, and the result is `sent #<ref>` of that message. Gives its
/// `msg_id`.
#[track_caller]
pub fn sent_by(
    session: &mut Session,
    clients: &Clients,
    arguments: Value,
    channel: ChannelId,
) -> MessageId {
    let mark = clients.mark();
    let called = session.call(SEND_TOOL, arguments.clone());
    assert!(
        !called.error,
        "{arguments}: {called:?}: {}",
        session.errors()
    );
    let msg_id = one_message_sent(&clients.talk.front, channel, mark);
    assert_says_sent(&called.text, msg_id);
    msg_id
}

/// Fails if a line of `shown` says `sent #`.
#[track_caller]
pub fn assert_says_not_sent(shown: &str, what: &str) {
    assert!(!says_sent(shown), "{what}: says sent: {shown}");
}

// The server's tests' readers of the KeyPackages API's answers and their person's browser.
#[path = "../../../server/tests/key_packages/answers.rs"]
mod answers;
#[path = "../../../server/tests/common/browser.rs"]
mod browser;

// The kit and the played device of the tests of channel groups, and the channel of the tests of
// channel messages.
#[path = "../channel_groups/kit.rs"]
mod kit;
#[path = "../channel_groups/played.rs"]
mod played;
#[path = "../channel_messages/talk.rs"]
mod talk;

// The helpers of every client test, finding keys in bytes, and what an agent reads, as the tests of
// compact output have it.
#[path = "../common/mod.rs"]
mod common;
#[path = "../device_keys/keys.rs"]
mod keys;
#[path = "../compact_output/lines.rs"]
mod lines;

// The MCP client of the tests of `inbox`, the device the tests of receiving play, and people in
// spaces.
#[path = "../mcp_inbox/client.rs"]
mod client;
#[path = "../receiving/peer.rs"]
mod peer;
#[path = "../roster_revisions/world.rs"]
mod world;

// Ada's machine with `clienta` and `clientb` and what Bob's device reads there, as the tests of the
// addressee of `skald send` have them, and the fixtures and the front of the tests of sending.
#[path = "../send_addressee/clients.rs"]
mod clients;
#[path = "../sending/fixtures.rs"]
mod fixtures;
#[path = "../sending/front.rs"]
mod front;

// The server, the machines and the relay of the tests of signing in.
#[path = "../sign_in/cloud.rs"]
mod cloud;
#[path = "../sign_in/machine.rs"]
mod machine;
#[path = "../sign_in/relay.rs"]
mod relay;
