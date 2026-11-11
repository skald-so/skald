//! Hands the waiting messages of a [`Source`] to a running Codex session with
//! `codex queue --thread <thread> --message <text>`: Codex gives the session the text as the user's
//! next prompt once its current turn ends, or at once if it is idle.
//!
//! It marks the messages handed over once the command succeeds and puts them back if it fails, as
//! `docs/protocol/delivery.md`, "Device to agent", says.

use std::io;
use std::process::{Command, Stdio};

use agent_inbox::render;
use agent_source::{Message, Source};

/// Queues every waiting message of `source` for the Codex session `thread` in one message, the
/// text [`render`] gives them, with `codex`, a command that runs the CLI. Returns the messages
/// handed over; runs nothing when none wait.
pub fn hand_over(source: &Source, thread: &str, mut codex: Command) -> io::Result<Vec<Message>> {
    source.hand_over(|messages| {
        let text = render(messages);
        let output = codex
            .args(["queue", "--thread", thread, "--message", &text])
            .stdin(Stdio::null())
            .output()?;
        if output.status.success() {
            Ok(())
        } else {
            Err(io::Error::other(format!(
                "codex queue failed with {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            )))
        }
    })
}
