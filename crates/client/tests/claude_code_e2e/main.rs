//! End-to-end tests of the Claude Code paths against the real Claude Code (`docs/dev/architecture.md`,
//! "Delivery to agents"; `docs/dev/infrastructure.md`, "PR checks"): sessions of Ada's agents run
//! `claude` on the model stub (`stub.rs`) in a temporary config home with no sign-in, as
//! `claude.rs` starts them, and the tests read what reached the model from the stub's requests.
//!
//! They run outside `script/test`, in a CI job of their own that installs the latest Claude Code:
//! each is ignored unless run with `--run-ignored only`, and says it is skipped without Claude Code
//! on `PATH`, a failure where `SKALD_E2E_REQUIRED=1`.
//!
//! As the tests of the hook handler (`claude_code_hooks/main.rs`): Ada's machine, alone in
//! `clienta`'s groups, `~/work/clientA` bound to it; her agent api sends with `skald --agent api
//! send`; Bob joins `clienta` by a fixture revision and his agent review sends from his machine.
//! A session's `skald mcp` and hooks run as the machine's runs do, as `--agent <agent>` or without
//! it.
//!
//! The sessions of `socket_sessions.rs`, `socket_routing.rs` and `socket_fallbacks.rs` get
//! messages over Claude Code's cross-session socket: `skald hook claude` is also their
//! `SessionStart` and `SessionEnd` hook (`socket_claude.rs`), on a stub that answers each turn by
//! what it holds (`socket_model.rs`). On Windows, where Claude Code's socket is a named pipe that
//! the client does not write to, they get them by the hooks.
//!
//! The sessions of `modes.rs` and `inline.rs` run under the person's delivery settings, as the
//! tests of the settings set them (`claude_code_modes/main.rs`, `delivery.rs`); those that wait for
//! an idle session to wake run on Unix only, where the client writes into the socket.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod claude;
mod delivery;
mod inline;
#[cfg(unix)]
mod modes;
mod sessions;
mod socket_claude;
mod socket_fallbacks;
mod socket_model;
mod socket_routing;
mod socket_sessions;
mod stub;

// The processes of a copy of `skald`, as the tests of the helper have them.
#[path = "../helper/helped.rs"]
mod helped;

// The hook handler's command, its runs and Ada's and Bob's agents, as the tests of the hook handler
// have them.
#[path = "../claude_code_hooks/hooks.rs"]
mod hooks;

include!("../claude_code_hooks/modules.rs");
