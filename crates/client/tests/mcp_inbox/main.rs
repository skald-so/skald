//! The MCP server and its `inbox` tool (`docs/dev/architecture.md`, "Components";
//! `docs/protocol/delivery.md`, "Device to agent"): an agent session with MCP registers as an agent
//! in its folder's space and reads its messages with `inbox`.
//!
//! - `skald mcp` is an MCP server on stdio, as an agent CLI starts it from its config; its folder
//!   is its working directory. Once the client initializes, the session registers as its agent in
//!   its folder's space: the messages the device writes from then on are kept for it, before its
//!   first tool call too; one the device wrote before is not.
//! - Without `--agent`, the agent's name joins the client's name, the device's certificate `name`
//!   and the device's reference, so the account's devices, also two of one `name`, give one agent
//!   CLI's sessions different addresses.
//! - `inbox` returns the lines the agent's `skald recv` shows: the messages handed over to it and
//!   the statuses and warnings it shows. It marks them handed over only once its result is written,
//!   so a client gone before that leaves them for the next call. With nothing waiting, it returns
//!   one short line. In a folder of one space it names nothing of another; in a folder in no space
//!   it says the agent sees no space.
//! - Each tool call goes through the helper or runs one-shot, as a command does. A call after the
//!   binary was replaced by a newer one while the server runs is served by the newer version.
//! - The answer to `tools/list` stays within a bound.
//!
//! As the tests of `skald recv --wait` (`recv_wait/main.rs`): the CLI runs on Ada's machine, whose
//! agents run in `~/work/acme`, bound to `acme`; Bob's device is played by the tests with an MLS
//! state of its own. Other machines of Ada's account are linked by hand, as in the tests of the
//! history on new devices (`history_devices/main.rs`). The tests play the agent CLI: an MCP client
//! of their own, over the server's standard streams (`client.rs`). Sessions run one-shot, as the
//! machine's runs do, unless a test runs them through a helper (`helper/main.rs`).
//!
//! The CLI, as these tests use it, beside the commands of the tests above:
//! - `skald mcp`, with the global `--agent <name>` and `--one-shot` and the setting
//!   `SKALD_ONE_SHOT=1` as other commands take them, speaks MCP's stdio transport: a JSON-RPC 2.0
//!   message per line on its standard input and output, nothing else on its output. Its client
//!   ends a session by closing the server's standard input;
//! - it answers `initialize` once the session is registered, as the agent `--agent` names or,
//!   without it, under the default name made of the `name` of the request's `clientInfo`, the
//!   client's name;
//! - the default name: the client's name and the device's certificate `name`, each lowercased, each
//!   run of characters other than `a`–`z` and `0`–`9` made `-`, `-` trimmed at both ends, and left
//!   out if empty; then the device's reference, the first 6 characters of the base32 of SHA-256 of
//!   the device ID's 16 bytes, as a message's reference is made (`compact_output/main.rs`); the
//!   three joined with `-`. Over 63 characters, the device's name is cut from its end, then, once
//!   it is gone with its `-`, the client's, until the whole is 63 characters; a `-` that a cut
//!   leaves at a name's end is trimmed;
//! - the tool `inbox`, without arguments, returns as its result's text the lines the agent's
//!   `skald recv` shows (`compact_output/main.rs`), and is no error; with nothing waiting, one line
//!   of at most [`SHORT`] characters and no message. In a folder in no space its text says
//!   `no space`;
//! - `SKALD_VERSION`, as the tests of the helper set it (`helper/main.rs`): `skald mcp` acting as an
//!   older version than the `skald` on disk is a server whose binary was replaced by a newer one
//!   while it runs;
//! - the result of `tools/list`, as compact JSON, is at most [`TOOLS_BOUND`] bytes.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod client;
mod crashes;
mod modes;
mod names;
mod sessions;
mod spaces;
mod tools;

use std::path::Path;

pub use client::{MCP, Session};

use crate::machine::Machine;

/// The name the MCP client of most tests gives itself at initialization.
pub const CLIENT: &str = "claude-code";

/// How many characters the line of an `inbox` with nothing waiting has at most.
pub const SHORT: usize = 60;

/// How many bytes the result of `tools/list` has at most, as compact JSON: room for `inbox`, the
/// send tool and the reply tool, with short descriptions.
pub const TOOLS_BOUND: usize = 2048;

/// Starts `skald <args> mcp` on `machine` in `dir`, set up as the machine's runs are, and
/// initializes it as the MCP client named `client`.
pub fn session_in(machine: &Machine, dir: &Path, args: &[&str], client: &str) -> Session {
    let mut all = args.to_vec();
    all.push(MCP);
    Session::start(&mut machine.command_in(dir, &all), client)
}

/// Fails unless `shown`, the text of an `inbox`, is one line of at most [`SHORT`] characters, no
/// message, and holds no ID.
#[track_caller]
pub fn assert_nothing_waiting(shown: &str) {
    let lines: Vec<&str> = (shown.lines().map(str::trim))
        .filter(|line| !line.is_empty())
        .collect();
    let [line] = lines.as_slice() else {
        panic!("not one line: {shown:?}");
    };
    assert!(
        line.chars().count() <= SHORT && lines::agent_line(line).is_none(),
        "not one short line: {line:?}"
    );
    lines::assert_no_uuid(shown);
}

// The modules the tests of the channel history share (`history_common/mod.rs`), the second
// machines of the tests of the history on new devices, and the helpers of the tests of reading the
// history.
#[path = "../history_common/mod.rs"]
mod history_common;
use history_common::*;
#[path = "../history_devices/devices.rs"]
mod devices;
#[path = "../history_reading/reading.rs"]
mod reading;

// Ada's agents and a message with a long text, as the tests of handing messages over have them;
// the messages of the tests of gap detection; the runs of a machine's own copy of `skald`, as the
// tests of the helper have them; and what an agent reads, as the tests of compact output have it.
#[path = "../agent_handover/agents.rs"]
mod agents;
#[path = "../gap_detection/gaps.rs"]
mod gaps;
#[path = "../agent_handover/held.rs"]
mod held;
#[path = "../helper/helped.rs"]
mod helped;
#[path = "../compact_output/lines.rs"]
mod lines;
