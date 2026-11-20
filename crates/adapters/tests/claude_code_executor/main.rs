//! Tests of the Claude Code executor (`docs/protocol/queues.md`, "Running a task"): a new Claude
//! Code session with no person, in a given folder, at `read-only` or `full`, with a clean context
//! and environment, run by `skald_adapters::claude_code::Executor`.
//!
//! - `modes.rs`, in `script/test`: the modes, and what a stand-in `claude` sees of a run: its
//!   version decides whether `read-only` runs, the folder and the environment it gets.
//! - `sessions.rs`, the e2e tests: the real Claude Code on the model stub of the client's e2e
//!   tests, in temporary config homes with no sign-in. They run outside `script/test`, ignored
//!   unless run with `--run-ignored only`; without Claude Code on `PATH` each says it is skipped and
//!   passes, unless `SKALD_E2E_REQUIRED=1` makes the skip a failure.

// Each test uses a part of the shared modules.
#![allow(dead_code)]

mod fixtures;
mod modes;
mod planted;
mod sessions;

// The model stub of the client's e2e tests (`docs/dev/agent-delivery.md`, "Model stub").
#[path = "../../../client/tests/claude_code_e2e/stub.rs"]
mod stub;
