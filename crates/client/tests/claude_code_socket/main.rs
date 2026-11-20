//! Claude Code sessions over their cross-session inbox socket (`docs/dev/architecture.md`,
//! "Components"; `docs/dev/agent-delivery.md`, "Waking an idle session" and "Read confirmations";
//! `docs/protocol/delivery.md`, "Device to agent" and "Receipts"):
//!
//! - `skald` as the `SessionStart` hook registers the session with the helper, starting one if none
//!   runs, as the agent the hooks act as, with its socket, its token and its session file; the
//!   token stays in the local state. `SessionEnd`, or a socket that is gone or takes no connection,
//!   ends it; the helper keeps running while a session is registered.
//! - The helper writes each message for the agent, as it arrives, into the socket of the last
//!   registered of its sessions that still runs, with the token: `own` text as its line, labelled
//!   as a request from the person's own agent; `peer` and `service` messages only as the signal.
//! - Own text written to the socket counts as handed over, and `read` goes, only once the session
//!   file confirms it: an entry that went to the model holds its `msg_id` and the model's own entry
//!   follows, not an API error; meanwhile no other path hands it over. A `peer` message is handed
//!   over by the `inbox` call its signal asks for.
//! - The socket's receipts `held`, `expired`, `refused`, `dropped` and `denied` confirm nothing:
//!   the message waits for the hooks and `inbox`. A held message the session later releases is not
//!   handed over again.
//! - A crash after the write and before the confirmation neither loses the message nor issues
//!   `read` twice. The client keeps no line of the session file.
//!
//! As the tests of the hook handler (`claude_code_hooks/main.rs`): Ada's machine, alone in
//! `clienta`, her agents web and api in `~/work/clientA`, Bob joining by a fixture revision. The
//! hooks run as Claude Code runs them, a copy of `skald` without the one-shot setting, as in the
//! tests of the helper (`helper/helped.rs`), on inputs recorded from Claude Code 2.1.329
//! (`recorded/`). A stand-in plays the Claude Code session: its socket and its session file
//! (`standin.rs`), as Claude Code handles them, so these tests run on Unix only.
//!
//! The CLI, as these tests use it, beside the commands of the tests of the hook handler:
//! - `skald [--agent <agent>] hook claude` also handles `SessionStart` and `SessionEnd`. On
//!   `SessionStart` it takes the session's socket and token from Claude Code's settings
//!   `CLAUDE_CODE_MESSAGING_SOCKET` and `CLAUDE_CODE_MESSAGING_TOKEN`, and the session's ID and
//!   file from the input's `session_id` and `transcript_path`. Each prints nothing.
//! - The helper writes to the socket in Claude Code's protocol: an auth line with the token, then
//!   one user line per message, `{"type":"user","message":{"role":"user","content":<text>},
//!   "from":<from>,"msg_id":<msg_id>,"session_id":<session>}`, its text that of the hooks. Its
//!   `from` is a reply address, `uds:<path>`, where the session sends the message's receipt.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

#[cfg(unix)]
mod confirms;
#[cfg(unix)]
mod crashes;
#[cfg(unix)]
mod cut;
#[cfg(unix)]
mod ended;
#[cfg(unix)]
mod failing;
#[cfg(unix)]
mod lifetime;
#[cfg(unix)]
mod registered;
#[cfg(unix)]
mod standin;
#[cfg(unix)]
mod writes;

// The hook handler's runs and Ada's and Bob's agents, as the tests of the hook handler have them;
// the runs of a copy of `skald` through a helper, as the tests of the helper have them.
#[path = "../helper/helped.rs"]
mod helped;
#[path = "../claude_code_hooks/hooks.rs"]
mod hooks;

include!("../claude_code_hooks/modules.rs");
