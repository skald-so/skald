//! The delivery settings on the paths into Codex sessions (`docs/protocol/delivery.md`, "Device to
//! agent"; `docs/agent-sessions.md`, "Delivery settings"):
//!
//! - How a message wakes a Codex session follows its channel's delivery mode for its sender's
//!   trust level, `own`, `peer` or `service`, as the person sets it in the local state:
//!   - `now`, the default: an idle session gets `codex queue` at once, a busy one the signal from
//!     its next hook;
//!   - `batch`, with its window: at most one signal per window for a channel and level, at the
//!     window's end, for every message that came in it;
//!   - `pull`: no signal by any hook or `codex queue`; `inbox` returns the messages.
//! - `inline` changes nothing here: the hooks and `codex queue` give only the signal.
//! - Wakes: each signal the client puts into a session by any path, a hook or `codex queue`, not
//!   `inbox`, counts one wake for each channel whose messages it covers. The person's
//!   `skald delivery-modes` shows them per channel, and `--json` too; they are kept across runs.
//!
//! As the tests of waking Codex sessions (`codex_wakes/main.rs`): Ada's machine alone in
//! `clienta`, her agents web and api in `~/work/clientA`, Bob joining by a fixture revision and her
//! second machine linked by hand; web's Codex session starts in a Codex app of its own, the
//! stand-in `codex`, which records each `codex queue`. `clienta`'s other channel, the talk's, holds
//! Ada's device, and Bob's once he joins it (`claude_code_e2e/delivery.rs`). The CLI of the
//! settings: `claude_code_modes/main.rs`.
//!
//! This binary, as that of the tests of waking Codex sessions, is also the stand-in, so it runs its
//! own harness (`codex_wakes/harness.rs`).

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod batched;
mod failed;
mod inline;
mod modes;
mod now;
mod pulled;
mod restarted;

// The harness, the stand-in `codex` and the senders of the tests of waking Codex sessions; Codex
// sessions and the runs of their hooks, as the tests of Codex's hooks have them; the runs of a copy
// of `skald` through a helper, as the tests of the helper have them; Ada's and Bob's agents, as
// the tests of Claude Code's hooks have them; the person's delivery settings, as the tests of the
// delivery settings have them.
#[path = "../codex_hooks/codex.rs"]
mod codex;
#[path = "../claude_code_e2e/delivery.rs"]
mod delivery;
#[path = "../codex_wakes/harness.rs"]
mod harness;
#[path = "../helper/helped.rs"]
mod helped;
#[path = "../claude_code_hooks/hooks.rs"]
mod hooks;
#[path = "../codex_wakes/queued.rs"]
mod queued;
#[path = "../codex_wakes/standin.rs"]
mod standin;

include!("../claude_code_hooks/modules.rs");

use std::process::ExitCode;

use harness::harness;

/// Each test, by the name the harness gives it.
const TESTS: &[(&str, fn())] = &[
    (
        "now::with_no_setting_each_message_wakes_the_session_at_once_and_counts_a_wake_per_channel",
        now::with_no_setting_each_message_wakes_the_session_at_once_and_counts_a_wake_per_channel,
    ),
    (
        "batched::in_batch_ten_peer_messages_within_a_window_wake_the_session_once_at_its_end",
        batched::in_batch_ten_peer_messages_within_a_window_wake_the_session_once_at_its_end,
    ),
    (
        "pulled::in_pull_no_path_wakes_the_session_and_inbox_returns_every_message",
        pulled::in_pull_no_path_wakes_the_session_and_inbox_returns_every_message,
    ),
    (
        "inline::inline_for_peer_still_gives_the_codex_session_only_the_signal",
        inline::inline_for_peer_still_gives_the_codex_session_only_the_signal,
    ),
    (
        "restarted::settings_and_wakes_survive_the_helper_s_restart",
        restarted::settings_and_wakes_survive_the_helper_s_restart,
    ),
    (
        "failed::a_failed_codex_queue_counts_no_wake_and_the_next_hook_counts_one",
        failed::a_failed_codex_queue_counts_no_wake_and_the_next_hook_counts_one,
    ),
];

fn main() -> ExitCode {
    if let Some(code) = standin::acts() {
        return code;
    }
    harness(TESTS)
}
