//! `skald` as Claude Code's hook handler of `PostToolUse` and `Stop` (`docs/dev/architecture.md`,
//! "Components"; `docs/dev/agent-delivery.md`, "Claude Code hooks"; `docs/protocol/delivery.md`,
//! "Device to agent"): a busy Claude Code session gets its agent's messages through its hooks, by
//! the sender's trust level.
//!
//! - The handler acts as the agent the session's `skald mcp` registers: the one `--agent` names to
//!   both, or the default name of the client name Claude Code gives at initialization.
//! - It gives the agent's waiting messages as additional context, each once; `Stop` continues the
//!   turn with them and, at Claude Code's continuation cap, leaves them waiting.
//! - `own` text goes in, as the agent's line, labelled as a request from the person's own agent;
//!   `peer` and `service` messages only as the signal: a fixed line without the sender or the text
//!   that tells the agent to call `inbox`, which then hands the message over.
//! - It prints nothing while nothing waits, in a folder in no space, and on a failure, which never
//!   keeps a turn going.
//! - Its handover is by construction, so `read` goes as for `inbox`.
//! - What it puts into a session at once stays within Claude Code's hook output limit, 10,000
//!   characters: messages that do not fit wait for the next handover; one longer than the limit is
//!   cut and ends with how to read it whole. `inbox` and the agent's `skald recv` take a message's
//!   `#<ref>` and return it whole.
//!
//! As the tests of messages between agents of one device (`local_agents/main.rs`): the CLI runs on
//! Ada's machine, alone in `clienta`'s groups, `~/work/clientA` bound to `clienta`. Her agent web
//! runs a Claude Code session in `~/work/clientA/web`: the tests start its `skald --agent web mcp`
//! with the MCP client of the tests of the MCP server (`mcp_inbox/client.rs`) and run its hooks
//! with inputs recorded from the real Claude Code (`inputs/`), their `cwd` the agent's folder. Her
//! agent api, in `~/work/clientA/api`, sends web `own` messages with `skald --agent api send`. The
//! tests of the `peer` level add Bob's account to `clienta` by a fixture revision, as those tests
//! do; his agent review sends from his own machine.
//!
//! The CLI, as these tests use it, beside the commands of the tests above:
//! - `skald [--agent <agent>] hook claude` is the hook handler: it reads Claude Code's hook input,
//!   a JSON object, on its standard input, and takes the event from its `hook_event_name`. It runs
//!   in the session's folder, the input's `cwd`;
//! - when it gives messages, it exits with success and prints one JSON object, whose
//!   `hookSpecificOutput` holds `hookEventName`, the input's event, and `additionalContext`, the
//!   text the session gets; when it gives none, it prints nothing, on neither output;
//! - an `own` message is its line as `inbox` shows it (`trust_levels/main.rs`); the signal is a line
//!   that names `inbox`;
//! - a cut message's text is the head of the message's, and the context's last line names `inbox`
//!   and the message's `#<ref>`;
//! - the tool `inbox` takes an optional argument `message`, a message's `#<ref>`, and `skald recv`
//!   an optional positional one, as `skald reply` takes a message: either returns that message's
//!   line whole.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod cap;
mod failures;
mod handover;
mod hooks;
mod levels;
mod limit;
mod naming;
mod quiet;

include!("../claude_code_hooks/modules.rs");
