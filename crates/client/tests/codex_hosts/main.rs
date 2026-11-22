//! Live checks of Codex sessions that agent hosts run, set up by `skald setup codex`
//! (`docs/agent-sessions.md`, "Agent hosts"): Paseo, an SDK host (`paseo.rs`), and Herdr, a PTY
//! host (`herdr.rs`). Each runs the real `codex`, the hosts and Skald's client on Ada's machine,
//! alone in `clienta` as the tests of Codex's hooks have it (`codex_hooks/main.rs`), with the model
//! stub (`claude_code_e2e/stub.rs`) as the model provider: no sign-in, a placeholder key, no
//! tokens. Its home and Codex's config home are the machine's, never the user's; each host writes
//! its hooks there before the setup adds Skald's (`host.rs`). The machine keeps Skald's state and
//! keys where its variables say, so the session's folder names them for Skald's MCP server in its
//! own config layer.
//!
//! - Skald's MCP server and CLI work: the stub calls `inbox` and runs `skald devices`.
//! - An idle session wakes as `codex queue` wakes one: once its turn ends, a message to its agent
//!   starts a turn with the signal, which needs the thread its hooks registered.
//! - The host's own hooks keep working: the host shows what they reported.
//! - The hooks each host writes are those of the fixtures of `codex_setup/homes/`.
//!
//! The checks run outside `script/test`: they are ignored unless run with `--run-ignored only`.
//! Without `codex` or the host on `PATH`, each says it is skipped and passes. Each prints its
//! result line with the host's and Codex's versions; with `SKALD_HOSTS_RECORD=<dir>` it also saves
//! what the hosts and Codex gave it, the recorded outputs of the unit tests here.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod app_server;
mod herdr;
mod host;
mod paseo;

// The model stub and the runs of real CLIs of the e2e tests; Codex sessions, as the tests of Codex's hooks have them; the
// runs of a copy of `skald` through a helper, as the tests of the helper have them; Ada's agents
// and the runs of a hook handler, as the tests of Claude Code's hooks have them.
#[path = "../claude_code_e2e/claude.rs"]
mod claude;
#[path = "../codex_hooks/codex.rs"]
mod codex;
#[path = "../helper/helped.rs"]
mod helped;
#[path = "../claude_code_hooks/hooks.rs"]
mod hooks;
#[path = "../claude_code_e2e/stub.rs"]
mod stub;

include!("../claude_code_hooks/modules.rs");
