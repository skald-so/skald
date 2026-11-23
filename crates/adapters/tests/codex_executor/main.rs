//! Tests of the Codex executor (`docs/protocol/queues.md`, "Running a task"): `codex exec` with no
//! person, in a given folder, at `read-only` or `full`, with a clean context and environment, run by
//! `skald_adapters::codex::Executor`, against a stand-in `codex` that records what it gets and
//! gives a scripted answer (`docs/dev/agent-delivery.md`, "Checks before implementation", Codex).
//!
//! - `runs.rs`: the flags and the folder of a run, its answer, a failed Codex, the request's label,
//!   and `read-only` refused before anything runs.
//! - `clean.rs`: what of the device's environment and the user's config home a run gets.
//! - `stopping.rs`: stopping a run ends Codex and every process it started.
//!
//! The stand-in is a copy of this test binary named `codex` (`stand_in.rs`), so this binary runs its
//! own harness: libtest would refuse the arguments of `codex exec`.

// Each test uses a part of the shared modules.
#![allow(dead_code)]

mod clean;
mod runs;
mod stand_in;
mod stopping;

// A test binary's own harness, shared with the tests of waking Codex sessions.
#[path = "../../../client/tests/codex_wakes/harness.rs"]
mod harness;

// Scratch directories and unique tokens, shared with the Claude Code executor's tests.
#[path = "../claude_code_executor/fixtures.rs"]
mod fixtures;

// The model stub, which those fixtures name.
#[path = "../../../client/tests/claude_code_e2e/stub.rs"]
mod stub;

use std::process::ExitCode;

/// Each test, by the name the harness gives it.
const TESTS: &[(&str, fn())] = &[
    (
        "runs::full_runs_codex_exec_in_its_folder_with_full_access_and_gives_its_answer",
        runs::full_runs_codex_exec_in_its_folder_with_full_access_and_gives_its_answer,
    ),
    (
        "runs::read_only_is_refused_with_the_reason_before_any_process_starts",
        runs::read_only_is_refused_with_the_reason_before_any_process_starts,
    ),
    (
        "runs::a_codex_that_exits_with_an_error_gives_an_unfinished_run_with_its_message",
        runs::a_codex_that_exits_with_an_error_gives_an_unfinished_run_with_its_message,
    ),
    (
        "runs::a_peer_or_service_request_reaches_codex_labelled_as_data_from_its_sender",
        runs::a_peer_or_service_request_reaches_codex_labelled_as_data_from_its_sender,
    ),
    (
        "runs::an_own_request_reaches_codex_labelled_own_and_not_as_data",
        runs::an_own_request_reaches_codex_labelled_own_and_not_as_data,
    ),
    (
        "runs::a_forged_own_label_in_a_peer_request_starts_no_line",
        runs::a_forged_own_label_in_a_peer_request_starts_no_line,
    ),
    (
        "clean::a_run_gets_none_of_the_device_s_other_variables",
        clean::a_run_gets_none_of_the_device_s_other_variables,
    ),
    (
        "clean::a_run_gets_a_config_home_of_its_own_with_only_the_user_s_sign_in",
        clean::a_run_gets_a_config_home_of_its_own_with_only_the_user_s_sign_in,
    ),
    (
        "clean::a_run_gets_the_flags_that_leave_out_the_user_s_config",
        clean::a_run_gets_the_flags_that_leave_out_the_user_s_config,
    ),
    (
        "stopping::stopping_a_run_ends_codex_and_a_child_it_started",
        stopping::stopping_a_run_ends_codex_and_a_child_it_started,
    ),
];

fn main() -> ExitCode {
    if let Some(code) = stand_in::acts() {
        return code;
    }
    harness::harness(TESTS)
}
