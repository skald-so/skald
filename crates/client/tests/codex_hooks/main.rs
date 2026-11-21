//! Codex sessions and the signal through their hooks (`docs/dev/architecture.md`, "Components";
//! `docs/dev/agent-delivery.md`, "Codex hooks"; `docs/protocol/delivery.md`, "Device to agent"):
//!
//! - `skald` is the hook handler of Codex's `SessionStart`, `PostToolUse`, `Stop` and
//!   `UserPromptSubmit`. Its hooks act as the agent the session's `skald mcp` registers: the one
//!   `--agent` names to both, or the default name of the client name Codex gives at
//!   initialization.
//! - `SessionStart` registers the session with the device and starts the helper when none runs. In
//!   a folder in no space nothing registers.
//! - A session is busy from a hook of its turn until the `Stop` that ends the turn. Of an agent's
//!   live sessions, the one whose hook ran last when a message comes gets its signal. A session
//!   lives while the Codex process that runs it does, and keeps the helper running meanwhile.
//!   Messages come while the session that gets them is busy: an idle one is woken otherwise.
//! - The signal is a fixed line that says Skald messages wait and `inbox` takes them, with no
//!   sender, text, count or reference. Every message, its own account's included, reaches a Codex
//!   session only as it.
//! - While messages wait whose signal the session did not get, `PostToolUse` and
//!   `UserPromptSubmit` add the signal as context and `Stop` continues the turn once with it.
//!   Otherwise every hook prints nothing and `Stop` lets the turn end.
//! - Hooks never wait on the network: with the helper and the server down they return at once and
//!   print nothing. Every output stays within Codex's hook output limit, about 2,500 tokens.
//!
//! As the tests of Claude Code's hooks (`claude_code_hooks/main.rs`): the CLI runs on Ada's
//! machine, alone in `clienta`'s groups, `~/work/clientA` bound to `clienta`; her agent web runs
//! its MCP session in `~/work/clientA/web`, and her agent api, in `~/work/clientA/api`, sends web
//! `own` messages. Bob, of another account, joins `clienta` by a fixture revision and sends from
//! his machine; Ada's second machine is linked by hand, as in the tests of the history on new
//! devices. The hooks run as the tests of the sessions over Claude Code's socket run them
//! (`claude_code_socket/main.rs`): a copy of `skald` without the one-shot setting, so messages from
//! other devices reach Ada's machine through the helper. They run on inputs recorded from Codex
//! 0.162.1 (`inputs/`), each session with its own thread and its config home in `CODEX_HOME`.
//!
//! The CLI, as these tests use it, beside the commands of the tests of Claude Code's hooks:
//! - `skald [--agent <agent>] hook codex` is the hook handler of Codex: it reads Codex's hook
//!   input, a JSON object, on its standard input, and takes the event from its
//!   `hook_event_name`, the session's thread from its `session_id`, its session file from
//!   `SessionStart`'s `transcript_path`, and its folder from its `cwd`, where it runs. The
//!   session's config home is `CODEX_HOME`;
//! - Codex runs a hook as its own child: the session's Codex process is the parent of each of its
//!   hooks' runs (checked on Codex 0.162.1). The tests play it with this process, or with a
//!   process of their own (`codex.rs`) where they end it;
//! - to give the signal, it exits with success and prints one JSON object: on `PostToolUse` and
//!   `UserPromptSubmit`, `hookSpecificOutput` holds `hookEventName`, the input's event, and
//!   `additionalContext`, the signal; on `Stop`, `decision` is `block` and `reason` the signal,
//!   which continues the turn (`docs/dev/agent-delivery.md`, "Codex hooks"). Otherwise it prints
//!   nothing, on neither output;
//! - a failing run prints nothing on its standard output and never exits with code 2, with which
//!   Codex would block `PostToolUse` or keep a turn going.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod codex;
mod limit;
mod offline;
mod quiet;
mod registration;
mod routing;
mod signals;

// The runs of a copy of `skald` through a helper, as the tests of the helper have them; Ada's and
// Bob's agents and the runs of a hook handler, as the tests of Claude Code's hooks have them.
#[path = "../helper/helped.rs"]
mod helped;
#[path = "../claude_code_hooks/hooks.rs"]
mod hooks;

include!("../claude_code_hooks/modules.rs");
