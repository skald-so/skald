//! Detecting a Claude Code session that waits on its own permission prompt
//! (`docs/dev/architecture.md`, "Notifications"; `docs/dev/agent-delivery.md`, "Permission
//! prompts"), for the person's notification:
//!
//! - On each path the live checks confirmed, the client records that a registered session waits on
//!   its permission prompt: the `PermissionRequest` hook, at once, and the `Notification` hook of
//!   type `permission_prompt`, about 6 s later, through the hook handler.
//! - The record names the agent, its session and the time, nothing of the prompt; each prompt is
//!   recorded once, whichever paths tell.
//! - The person's `skald recv` shows it as one line, `--json` with every field; an agent's output
//!   never does.
//! - These hooks print nothing.
//!
//! As the tests of Claude Code sessions over their socket (`claude_code_socket/main.rs`): Ada's
//! machine, alone in `clienta`, her agents web and api in `~/work/clientA`; the hooks run as Claude
//! Code runs them, through a helper, and a stand-in plays each session, registered by its
//! `SessionStart`, so these tests run on Unix only. The hook inputs are recorded from Claude Code
//! 2.1.328 (`recorded/`), with the session's ID and file, and the call's command and description
//! the test's.
//!
//! The CLI, as these tests use it, beside the commands of the tests of the hook handler:
//! - `skald [--agent <agent>] hook claude` also handles `PermissionRequest` and `Notification`;
//!   each prints nothing, on neither output, and exits with success;
//! - the person's `skald recv` shows each recorded prompt once, whichever run checks, as one line
//!   that names the agent and holds the word `permission`, and nothing of the prompt: neither the
//!   tool, its command nor its description;
//! - with `--json`, a recorded prompt is one object of `"type": "permission_prompt"` with `agent`,
//!   the agent's name or `owner/agent`; `session`, Claude Code's `session_id`; and `timestamp`,
//!   when it was recorded, in Unix seconds, as a status's;
//! - an agent's `skald recv`, with `--json` too, and `inbox` show nothing of it.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

#[cfg(unix)]
mod once;
#[cfg(unix)]
mod prompt_lines;
#[cfg(unix)]
mod prompts;
#[cfg(unix)]
mod shown;
#[cfg(unix)]
mod unseen;

// The runs of a copy of `skald` through a helper, as the tests of the helper have them; the hook
// handler's runs and Ada's agents, as the tests of the hook handler have them; Ada's sessions and
// the stand-in of a session, as the tests of sessions over their socket have them.
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
