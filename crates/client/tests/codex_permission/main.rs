//! Detecting a Codex session that waits on Codex's own permission prompt
//! (`docs/dev/architecture.md`, "Notifications"; `docs/dev/agent-delivery.md`, "Codex permission
//! prompts"), for the person's notification:
//!
//! - On each path the live checks confirmed, the device notes that a registered Codex session
//!   waits on a prompt, from when, until the prompt is answered or dropped: the `PermissionRequest`
//!   hook, through the hook handler, and the app-server's `item/commandExecution/requestApproval`
//!   with its `thread/status/changed`, on the background app-server of the session's config home.
//!   The terminal notification names no session, so it is no such path.
//! - The person's `skald recv` and `skald recv --wait` show each prompt once, whichever paths tell,
//!   as one line that names the agent and its folder, within a second of the path's event; with
//!   `--json`, its session and times. Nothing of the prompt's command shows.
//! - An agent's output never shows it, and nothing of it leaves the device: the server stores no
//!   envelope for it.
//!
//! As the tests of Codex sessions and their hooks (`codex_hooks/main.rs`): Ada's machine, alone in
//! `clienta`, `~/work/clientA` bound to `clienta`, web's MCP session in `~/work/clientA/web`; the
//! hooks run through a copy of `skald` without the one-shot setting, each session with its own
//! thread and the config home `~/.codex` in `CODEX_HOME`. Here web's Codex session runs in
//! `~/work/clientA/frontend` and api's in `~/work/clientA/backend`, so a folder is not the agent's
//! name. The `PermissionRequest` input is recorded from Codex 0.162.1, and the app-server's
//! messages from Codex 0.162.1's app-server on the model stub (`recorded/`), with the test's thread,
//! folder and times.
//!
//! The CLI, as these tests use it, beside the commands of the tests of Codex's hooks:
//! - `skald [--agent <agent>] hook codex` also handles `PermissionRequest`: it prints nothing, on
//!   neither output, and exits with success. The call's `PostToolUse` ends the prompt, as a next
//!   turn's `UserPromptSubmit` does after a prompt the person declined, which runs no hook; so does
//!   the exit of the session's Codex process;
//! - the background app-server of a registered session's config home listens on a WebSocket on a
//!   Unix socket that `app-server-control/app-server-control.sock` there links to, as Codex
//!   0.162.1's does (`standin.rs` plays it). A client gets a thread's events once it has sent
//!   `initialize`, then `initialized`, and `thread/resume` of the thread;
//!   `serverRequest/resolved` ends a prompt, whichever answer the person gave in the terminal. The
//!   device answers no request of the app-server: the person answers the prompt in the terminal;
//! - the person's line of a prompt holds the word `permission`, the agent's name and the session's
//!   folder: its path, or its path under the home as `~/…`;
//! - with `--json`, a prompt is one object of `"type": "permission_prompt"` with `agent`, the
//!   agent's name or `owner/agent`; `session`, Codex's thread; `timestamp`, when the session began
//!   to wait, and `ended`, when the prompt was answered or dropped, in Unix seconds; `ended` is
//!   `null` or missing while the session waits;
//! - an agent's `skald recv`, with `--json` too, and `inbox` show nothing of it.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

#[cfg(unix)]
mod app_server;
mod hook_path;
mod prompts;
#[cfg(unix)]
mod standin;

// Codex's sessions and their hooks, as the tests of Codex's hooks have them; the runs of a copy of
// `skald` through a helper, as the tests of the helper have them; Ada's agents and the runs of a
// hook handler, as the tests of Claude Code's hooks have them; what an agent's runs show, as the
// tests of Claude Code's permission prompts have it.
#[path = "../codex_hooks/codex.rs"]
mod codex;
#[path = "../helper/helped.rs"]
mod helped;
#[path = "../claude_code_hooks/hooks.rs"]
mod hooks;
#[path = "../claude_code_permission/prompt_lines.rs"]
mod prompt_lines;

include!("../claude_code_hooks/modules.rs");
