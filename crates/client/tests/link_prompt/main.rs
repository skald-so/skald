//! The prompt while a device waits for linking (`docs/protocol/membership.md`, "Link code" and
//! "Device certificate"; `docs/protocol/delivery.md`): while a new device of the account has a run
//! of its link code open, every command on a linked `client` device of the account that reaches the
//! server, through the helper or one-shot, as the person or an agent, and its MCP server's tool
//! results, say once that the server reports, unverified, a device of that `name` and kind waiting
//! to be linked, and that `skald link` links it. The name is escaped as other text from the server.
//! A waiting `skald recv --wait` shows it as it shows a warning; `--json` carries it as a field.
//! Once the device links, its code ends or its certificate is refused as over the devices limit, no
//! command says it; the waiting device itself and other accounts' devices never do. A command that
//! makes no request, such as `skald rules`, says nothing of it.
//!
//! As the tests of linking with a link code (`link_codes/main.rs`): Ada's new machine signs in at
//! a terminal and shows its code; her first machine links it with `skald link`.
//! As the tests of `skald recv --wait` (`recv_wait/main.rs`): her agents run in `~/work/acme`,
//! bound to `acme`, and Bob's device is played by the tests; runs through a helper run the
//! machine's own copy of `skald`. As the tests of the MCP server (`mcp_inbox/main.rs`): the tests
//! play the agent CLI.
//!
//! The CLI, as these tests use it, beside the commands of the tests above: while a device of the
//! account waits, a command that reaches the server prints, on its standard output or standard
//! error, one line that holds the device's registered `name`, its control characters and line
//! breaks escaped as `\u{1b}` and `\n`, its kind `client`, `server`, `unverified` and `skald link`;
//! no other line holds that `name`. An agent's line holds no ID. With `--json`, standard output
//! holds only JSON objects, and one of them has, among its values, the `name` and the kind
//! `client`. The `inbox` tool's text holds the same line as an agent's `skald recv`.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod ends;
mod escaping;
mod json;
mod mcp;
mod prompt;
mod runs;
mod services;
mod waiting;

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

// Ada's agents, as the tests of handing messages over have them; the messages of the tests of gap
// detection; the runs of a machine's own copy of `skald`, as the tests of the helper have them;
// what an agent reads, as the tests of compact output have it; and the waits of the tests of
// `skald recv --wait`.
#[path = "../agent_handover/agents.rs"]
mod agents;
#[path = "../gap_detection/gaps.rs"]
mod gaps;
#[path = "../helper/helped.rs"]
mod helped;
#[path = "../compact_output/lines.rs"]
mod lines;
#[path = "../recv_wait/waits.rs"]
mod waits;

// The new device and `skald link` of the tests of linking with a link code, and the terminal and
// grants of the tests of the recovery key.
#[path = "../link_codes/linking.rs"]
mod linking;
#[path = "../recovery_key/terminal.rs"]
mod terminal;
#[path = "../recovery_key/grants.rs"]
mod terminal_grants;

// The agent CLI of the tests of the MCP server.
#[path = "../mcp_inbox/client.rs"]
mod mcp_client;

// The commands of the tests of service agents.
#[path = "../service_agents/kit.rs"]
mod service_kit;
