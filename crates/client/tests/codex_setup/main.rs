//! Skald's setup in Codex's config (`docs/agent-sessions.md`, "What you need to do";
//! `docs/dev/agent-delivery.md`, "Codex hooks"):
//!
//! - One step sets Skald up in the config home Codex uses, `CODEX_HOME` or, without it, `~/.codex`:
//!   `skald mcp` as the MCP server `skald` in `config.toml`, whose tools Codex calls without an
//!   approval prompt (`default_tools_approval_mode = "approve"`), and the hooks of Skald's hook
//!   handler for Codex, `skald hook codex`, for `SessionStart`, `PostToolUse`, `UserPromptSubmit`
//!   and `Stop`, in `config.toml` or `hooks.json`.
//! - Every other entry and comment stays, the hooks Paseo and Herdr write into `hooks.json`
//!   included. Run again, it changes nothing, so Codex asks for no new trust; it replaces an older
//!   Skald's entries; a removal takes out only Skald's.
//! - A config it cannot read stays as it is, and the run fails and says why.
//!
//! The CLI, as these tests use it:
//! - `skald setup codex` sets Skald up; `skald setup codex --remove` removes Skald's entries. Each
//!   takes the config home from `CODEX_HOME`, else `~/.codex` (`HOME`, or `USERPROFILE` on
//!   Windows), needs no sign-in and asks nothing. Each exits with success once done, and with
//!   failure, naming the file, when it cannot read `config.toml` or `hooks.json`.
//!
//! The fixtures in `homes/` are config homes: `others`, with other MCP servers, settings, project
//! and hook trust, hooks in both files and comments; `paseo`, `herdr` and `hosts`, with
//! `hooks.json` as Paseo, Herdr, and Herdr then Paseo, write it, by their sources (Paseo
//! `packages/server/src/terminal/agent-hooks/codex/`, Herdr `src/integration/`); `older`, with an
//! older Skald's entries beside Paseo's; `broken-toml` and `broken-json`, which do not parse. The
//! live checks with the hosts record what they write.
//!
//! The runs on fixtures take a machine of their own each, as the tests of signing in have them. The
//! runs in a Codex session take Ada's machine alone in `clienta`, as the tests of Codex's hooks
//! have it (`codex_hooks/main.rs`), and run each hook's command through `sh -c`, or `cmd /c` on
//! Windows.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod config_home;
mod in_sessions;
mod set_up;

/// The command that sets Skald up in an agent CLI's config, and its flag that removes it.
pub const SETUP: &str = "setup";
pub const REMOVE: &str = "--remove";

// Codex sessions and the runs of their hooks, as the tests of Codex's hooks have them; the runs of
// a copy of `skald` through a helper, as the tests of the helper have them; Ada's agents and the
// runs of a hook handler, as the tests of Claude Code's hooks have them.
#[path = "../codex_hooks/codex.rs"]
mod codex;
#[path = "../helper/helped.rs"]
mod helped;
#[path = "../claude_code_hooks/hooks.rs"]
mod hooks;

include!("../claude_code_hooks/modules.rs");
