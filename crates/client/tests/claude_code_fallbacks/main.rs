//! The fallbacks that wake an idle Claude Code session where its cross-session socket does not reach
//! it, and resending by the next path (`docs/dev/architecture.md`, "Delivery to agents";
//! `docs/dev/agent-delivery.md`, "Channels", "Waking an idle session" and "Read confirmations";
//! `docs/protocol/delivery.md`, "Device to agent"):
//!
//! - Channels: `skald mcp` declares Claude Code's channel capability and pushes each of its agent's
//!   messages as a channel event, rendered as the hooks render it and within their limit: `own`
//!   text as its line, a message below `own` only as the signal. Claude Code acknowledges no event,
//!   so only a confirmation tells that one reached the model.
//! - The `asyncRewake` hook on `SessionStart` and `Stop` waits for the agent's messages up to its
//!   timeout, gives them, rendered so, and starts a turn; with none, it ends printing nothing.
//! - A message goes by the session's first path, in the order socket, Channels, `asyncRewake`, mod;
//!   without a confirmation within a timeout, by the next; after the last it waits for the hooks and
//!   `inbox`. A signal the agent does not follow with `inbox` goes again. One confirmation ends it,
//!   and a message the session file already shows goes by no other path. A helper that restarts
//!   still resends what waits.
//!
//! As the tests of the hook handler (`claude_code_hooks/main.rs`) and of the socket
//! (`claude_code_socket/main.rs`): Ada's machine, alone in `clienta`, her agents web and api in
//! `~/work/clientA`, Bob joining by a fixture revision. `skald mcp` and the hooks run as Claude Code
//! runs them, a copy of `skald` without the one-shot setting (`helper/helped.rs`). The stand-in of a
//! Claude Code session (`claude_code_socket/standin.rs`) plays the socket and the session file, so
//! the tests of resending run on Unix only.
//!
//! The CLI, as these tests use it, beside the commands of those tests:
//! - `SKALD_RESEND_AFTER`: how long, in seconds, a handover waits for its confirmation before the
//!   message goes by the next path, for tests; the helper takes it from the command that starts it.
//! - `skald mcp` answers `initialize` with `capabilities.experimental["claude/channel"]`, an object,
//!   and sends each event as the notification `notifications/claude/channel`, its text in
//!   `params.content`.
//! - `skald [--agent <agent>] hook claude --wait <seconds>` is the `asyncRewake` hook: on the
//!   `SessionStart` or `Stop` input it waits for up to `<seconds>`; with messages, it prints them on
//!   its standard error and exits with 2, which starts a turn; with none, it exits with success,
//!   printing nothing.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod channels;
mod fallbacks;
#[cfg(unix)]
mod resending;
mod rewake;

#[cfg(unix)]
#[path = "../claude_code_socket/registered.rs"]
mod registered;
#[cfg(unix)]
#[path = "../claude_code_socket/standin.rs"]
mod standin;

// The hook handler's runs and Ada's and Bob's agents, as the tests of the hook handler have them;
// the runs of a copy of `skald` through a helper, as the tests of the helper have them.
#[path = "../helper/helped.rs"]
mod helped;
#[path = "../claude_code_hooks/hooks.rs"]
mod hooks;

include!("../claude_code_hooks/modules.rs");
