//! Tests of the Codex executor (`docs/protocol/queues.md`, "Running a task"): `codex exec` with no
//! person, in a given folder, at `read-only` or `full`, with a clean context and environment, run by
//! `skald_adapters::codex::Executor`, against a stand-in `codex` that records what it gets and
//! gives a scripted answer (`docs/dev/agent-delivery.md`, "Checks before implementation", Codex).
//!
//! - `runs.rs`: the flags and the folder of a run, its answer, a failed Codex, the request's label,
//!   and `read-only` refused before anything runs.
//! - `clean.rs`: what of the device's environment and the user's config home a run gets.
//! - `stopping.rs`: stopping a run ends Codex and every process it started.

// Each test uses a part of the shared modules.
#![allow(dead_code)]

mod clean;
mod runs;
mod stand_in;
mod stopping;

// Scratch directories and unique tokens, shared with the Claude Code executor's tests.
#[path = "../claude_code_executor/fixtures.rs"]
mod fixtures;

// The model stub, which those fixtures name.
#[path = "../../../client/tests/claude_code_e2e/stub.rs"]
mod stub;
