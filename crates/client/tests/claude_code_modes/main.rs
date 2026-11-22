//! The delivery settings of Claude Code sessions (`docs/protocol/delivery.md`, "Device to agent";
//! `docs/agent-sessions.md`):
//!
//! - The device's person sets, per channel and sender trust level, `own`, `peer` and `service`,
//!   the delivery mode, and for `peer` and `service` the text setting, in the local state, and
//!   lists them with the wakes of each channel. As an agent, the CLI refuses each and changes
//!   nothing.
//! - The mode, `now` by default: `now` hands each message over as it comes; `batch`, with its
//!   window, keeps the level's messages of the channel until the window ends, the window opening
//!   with the first of them that waits, then hands them over in one handover; `pull` hands them
//!   over only by `inbox` and the agent's `skald recv`: no socket write and no hook gives them.
//! - The text, `signal` by default: `inline` puts a `peer` or `service` message's text in at once,
//!   by the socket and the hooks, as its line labelled as data from its sender, after the warning
//!   line, so no `inbox` call is needed and `read` goes once the session confirms it.
//! - Wakes: each handover a path pushes into a session, the socket or a hook, not `inbox`, counts
//!   one wake for each channel whose messages, or whose signal, it carries.
//!
//! As the tests of the hook handler (`claude_code_hooks/main.rs`) and of the socket
//! (`claude_code_socket/main.rs`): Ada's machine alone in `clienta`, her agents web and api in
//! `~/work/clientA`, Bob joining by a fixture revision; `clienta`'s other channel, the talk's,
//! holds Ada's device, and Bob's once he joins it (`claude_code_e2e/delivery.rs`). The tests of
//! the socket run on Unix only, as its stand-in does.
//!
//! The CLI, as these tests use it, beside the commands of the tests above:
//! - `skald set-delivery-mode <channel> <level> <mode> [--window <window>]`: `<channel>` a
//!   channel's ID of the working directory's space, as `skald channels` shows it; `<level>` one of
//!   `own`, `peer` and `service`; `<mode>` one of `now`, `batch` and `pull`; `<window>` whole
//!   seconds, minutes or hours, such as `30s`, `5m` or `2h`, which `batch` needs and the others
//!   take not;
//! - `skald set-delivery-text <channel> <level> <text>`: `<level>` `peer` or `service`, `<text>`
//!   `signal` or `inline`;
//! - `skald delivery-modes` lists each channel of the working directory's space this device is in:
//!   per channel and level, a line that holds the channel's ID, the level and its mode, and for
//!   `peer` and `service` its text setting; per channel, a line that holds the channel's ID, the
//!   word `wakes` and their count. With `--json`, one object per channel: `channel`, its ID;
//!   `wakes`, a number; and `own`, `peer` and `service`, each an object with `mode`, `window`, the
//!   window in seconds or null, and, but for `own`, `text`;
//! - each fails, says why and changes nothing with the global `--agent <name>`, for an unknown
//!   channel, level, mode or text, `batch` without a window, a window that is no whole positive
//!   count of seconds, minutes or hours, or a text setting for `own`.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod by_hooks;
#[cfg(unix)]
mod by_socket;
mod commands;

// The settings' commands and the other channel, as the end-to-end tests have them.
#[path = "../claude_code_e2e/delivery.rs"]
mod delivery;

// The hook handler's runs and Ada's and Bob's agents, as the tests of the hook handler have them;
// the runs of a copy of `skald` through a helper, as the tests of the helper have them; the
// stand-in of a Claude Code session and its registration, as the tests of the socket have them.
#[path = "../helper/helped.rs"]
mod helped;
#[path = "../claude_code_hooks/hooks.rs"]
mod hooks;
#[cfg(unix)]
#[path = "../claude_code_socket/registered.rs"]
mod registered;
#[cfg(unix)]
#[path = "../claude_code_socket/standin.rs"]
mod standin;

include!("../claude_code_hooks/modules.rs");
